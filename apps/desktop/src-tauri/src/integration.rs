//! Reacts to engine events on the desktop: tray tooltip with speeds, notifications on
//! completion and failure, and sleep prevention while downloads run.
//!
//! [`Activity`] is the pure part (tested): it tracks names and running tasks and turns
//! events into [`Effect`]s. [`run`] applies the effects to the real tray, notifications
//! and sleep guard.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use pv_core::{DesktopSettings, EngineEvent, TaskId, TaskSnapshot, TaskState};
use tokio::sync::{broadcast, watch};

/// Something to do in response to an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Show a system notification.
    Notify {
        /// Title.
        title: String,
        /// Body.
        body: String,
    },
    /// Whether downloads are running (for sleep prevention).
    Busy(bool),
    /// Totals for the tray tooltip.
    Speeds {
        /// Download bytes/second.
        down: u64,
        /// Upload bytes/second.
        up: u64,
        /// Running tasks.
        active: usize,
    },
}

/// What the desktop knows about tasks, from the initial list and the event stream.
#[derive(Debug, Default)]
pub struct Activity {
    names: HashMap<TaskId, String>,
    running: HashSet<TaskId>,
}

fn is_running(state: &TaskState) -> bool {
    matches!(
        state,
        TaskState::Probing
            | TaskState::FetchingMetadata
            | TaskState::Checking
            | TaskState::Downloading
            | TaskState::Verifying
            | TaskState::Seeding
    )
}

fn display_name(task: &TaskSnapshot) -> String {
    task.spec
        .file_name
        .clone()
        .unwrap_or_else(|| crate::dto::TaskView::from(task).name)
}

impl Activity {
    /// Starts from the engine's task list.
    #[must_use]
    pub fn new(tasks: &[TaskSnapshot]) -> Self {
        Self {
            names: tasks.iter().map(|t| (t.id, display_name(t))).collect(),
            running: tasks
                .iter()
                .filter(|t| is_running(&t.state))
                .map(|t| t.id)
                .collect(),
        }
    }

    /// Whether any download runs.
    #[must_use]
    pub fn busy(&self) -> bool {
        !self.running.is_empty()
    }

    /// Applies one engine event.
    pub fn apply(&mut self, event: &EngineEvent, settings: &DesktopSettings) -> Vec<Effect> {
        let was_busy = self.busy();
        let mut effects = Vec::new();
        match event {
            EngineEvent::TaskAdded(task) => {
                self.names.insert(task.id, display_name(task));
            }
            EngineEvent::MetadataResolved { id, name, .. } if !name.is_empty() => {
                self.names.insert(*id, name.clone());
            }
            EngineEvent::TaskRemoved(id) => {
                self.names.remove(id);
                self.running.remove(id);
            }
            EngineEvent::StateChanged { id, state } => {
                if is_running(state) {
                    self.running.insert(*id);
                } else {
                    self.running.remove(id);
                }
                let name = self.names.get(id).cloned().unwrap_or_default();
                match state {
                    TaskState::Completed if settings.notify_on_complete => {
                        effects.push(Effect::Notify {
                            title: "Download complete".to_owned(),
                            body: name,
                        });
                    }
                    TaskState::Failed { reason, .. } if settings.notify_on_failure => {
                        effects.push(Effect::Notify {
                            title: "Download failed".to_owned(),
                            body: format!("{name}: {reason}"),
                        });
                    }
                    _ => {}
                }
            }
            EngineEvent::ProgressBatch(batch) => {
                effects.push(Effect::Speeds {
                    down: batch.iter().map(|p| p.speed_bps).sum(),
                    up: batch.iter().filter_map(|p| p.upload_bps).sum(),
                    active: batch.len(),
                });
            }
            _ => {}
        }
        if self.busy() != was_busy {
            effects.push(Effect::Busy(self.busy()));
            if !self.busy() {
                effects.push(Effect::Speeds {
                    down: 0,
                    up: 0,
                    active: 0,
                });
            }
        }
        effects
    }
}

/// The tray tooltip for the given totals.
#[must_use]
pub fn tooltip(down: u64, up: u64, active: usize) -> String {
    if active == 0 {
        return "Persivio: idle".to_owned();
    }
    let upload = if up > 0 {
        format!("  Up {}/s", human_bytes(up))
    } else {
        String::new()
    };
    format!(
        "Persivio: {active} active\nDown {}/s{upload}",
        human_bytes(down)
    )
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    #[allow(clippy::cast_precision_loss, reason = "display only")]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Where effects go. Implemented by the app (tray, notifications, sleep guard).
pub trait Sink: Send + 'static {
    /// Shows a notification.
    fn notify(&self, title: &str, body: &str);
    /// Sets the tray tooltip.
    fn tooltip(&self, text: &str);
    /// Prevents or allows sleep.
    fn keep_awake(&self, on: bool);
}

/// Minimum time between two tooltip updates.
const TOOLTIP_INTERVAL: Duration = Duration::from_secs(1);

/// Applies engine events until the engine stops. Settings changes take effect at once
/// (e.g. turning sleep prevention off while downloads run releases the guard).
pub async fn run(
    mut events: broadcast::Receiver<EngineEvent>,
    mut settings: watch::Receiver<DesktopSettings>,
    mut activity: Activity,
    sink: impl Sink,
) {
    let mut last_tooltip = tokio::time::Instant::now() - TOOLTIP_INTERVAL;
    sink.keep_awake(activity.busy() && settings.borrow().prevent_sleep);
    sink.tooltip(&tooltip(0, 0, 0));
    loop {
        tokio::select! {
            changed = settings.changed() => {
                if changed.is_err() {
                    return;
                }
                sink.keep_awake(activity.busy() && settings.borrow().prevent_sleep);
            }
            received = events.recv() => {
                let event = match received {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                let current = settings.borrow().clone();
                for effect in activity.apply(&event, &current) {
                    match effect {
                        Effect::Notify { title, body } => sink.notify(&title, &body),
                        Effect::Busy(busy) => sink.keep_awake(busy && current.prevent_sleep),
                        Effect::Speeds { down, up, active } => {
                            let now = tokio::time::Instant::now();
                            if active == 0 || now - last_tooltip >= TOOLTIP_INTERVAL {
                                last_tooltip = now;
                                sink.tooltip(&tooltip(down, up, active));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use pv_core::{FailReason, Progress, Source, TaskSpec};

    use super::*;

    fn task(name: &str) -> TaskSnapshot {
        let mut spec = TaskSpec::new(
            Source::http(url::Url::parse("https://example.com/x").unwrap()).unwrap(),
            std::env::temp_dir(),
        );
        spec.file_name = Some(name.to_owned());
        TaskSnapshot::new(spec)
    }

    fn state(id: TaskId, state: TaskState) -> EngineEvent {
        EngineEvent::StateChanged { id, state }
    }

    #[test]
    fn notifications_follow_the_settings() {
        let t = task("ubuntu.iso");
        let mut a = Activity::new(std::slice::from_ref(&t));
        let on = DesktopSettings::default();
        assert_eq!(
            a.apply(&state(t.id, TaskState::Completed), &on),
            [Effect::Notify {
                title: "Download complete".into(),
                body: "ubuntu.iso".into()
            }]
        );
        let failed = TaskState::Failed {
            reason: FailReason::LinkExpired,
            retryable: false,
        };
        assert_eq!(
            a.apply(&state(t.id, failed.clone()), &on),
            [Effect::Notify {
                title: "Download failed".into(),
                body: "ubuntu.iso: download link expired".into()
            }]
        );
        let off = DesktopSettings {
            notify_on_complete: false,
            notify_on_failure: false,
            ..DesktopSettings::default()
        };
        assert_eq!(a.apply(&state(t.id, TaskState::Completed), &off), []);
        assert_eq!(a.apply(&state(t.id, failed), &off), []);
    }

    #[test]
    fn busy_changes_only_on_the_first_start_and_last_stop() {
        let (x, y) = (task("x"), task("y"));
        let mut a = Activity::new(&[x.clone(), y.clone()]);
        let s = DesktopSettings::default();
        assert!(!a.busy());
        assert_eq!(
            a.apply(&state(x.id, TaskState::Downloading), &s),
            [Effect::Busy(true)]
        );
        assert_eq!(a.apply(&state(y.id, TaskState::Probing), &s), []);
        assert_eq!(a.apply(&state(x.id, TaskState::Paused), &s), []);
        assert_eq!(
            a.apply(&EngineEvent::TaskRemoved(y.id), &s),
            [
                Effect::Busy(false),
                Effect::Speeds {
                    down: 0,
                    up: 0,
                    active: 0
                }
            ]
        );
    }

    #[test]
    fn speeds_are_summed_and_formatted() {
        let mut a = Activity::default();
        let p = |down, up| Progress {
            task_id: TaskId::new(),
            downloaded: 0,
            total: None,
            speed_bps: down,
            eta_secs: None,
            connections_active: 1,
            peers: None,
            upload_bps: up,
            check_percent: None,
        };
        let effects = a.apply(
            &EngineEvent::ProgressBatch(vec![p(1024 * 1024, None), p(512 * 1024, Some(2048))]),
            &DesktopSettings::default(),
        );
        assert_eq!(
            effects,
            [Effect::Speeds {
                down: 1536 * 1024,
                up: 2048,
                active: 2
            }]
        );
        assert_eq!(
            tooltip(1536 * 1024, 2048, 2),
            "Persivio: 2 active\nDown 1.5 MiB/s  Up 2.0 KiB/s"
        );
        assert_eq!(tooltip(0, 0, 0), "Persivio: idle");
        assert_eq!(tooltip(10, 0, 1), "Persivio: 1 active\nDown 10 B/s");
    }

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<String>>>);
    impl Sink for Recorder {
        fn notify(&self, title: &str, body: &str) {
            self.0
                .lock()
                .unwrap()
                .push(format!("notify {title}: {body}"));
        }
        fn tooltip(&self, text: &str) {
            self.0
                .lock()
                .unwrap()
                .push(format!("tooltip {}", text.lines().next().unwrap()));
        }
        fn keep_awake(&self, on: bool) {
            self.0.lock().unwrap().push(format!("awake {on}"));
        }
    }

    #[tokio::test]
    async fn turning_sleep_prevention_off_releases_the_guard_at_once() {
        let t = task("a");
        let (events_tx, events_rx) = broadcast::channel(16);
        let (settings_tx, settings_rx) = watch::channel(DesktopSettings::default());
        let rec = Recorder::default();
        let runner = tokio::spawn(run(
            events_rx,
            settings_rx,
            Activity::new(std::slice::from_ref(&t)),
            rec.clone(),
        ));
        events_tx.send(state(t.id, TaskState::Downloading)).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        settings_tx.send_modify(|s| s.prevent_sleep = false);
        tokio::time::sleep(Duration::from_millis(50)).await;
        events_tx.send(state(t.id, TaskState::Completed)).unwrap();
        drop(events_tx);
        runner.await.unwrap();
        assert_eq!(
            *rec.0.lock().unwrap(),
            [
                "awake false",
                "tooltip Persivio: idle",
                "awake true",
                "awake false",
                "notify Download complete: a",
                "awake false",
                "tooltip Persivio: idle",
            ]
        );
    }
}
