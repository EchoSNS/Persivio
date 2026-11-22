//! `persivio watch`: live progress bars until the queue is empty.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use indicatif::{HumanBytes, MultiProgress, ProgressBar, ProgressStyle};
use pv_core::{EngineEvent, TaskId, TaskSnapshot, TaskState, TaskStateKind};
use pv_engine::EngineHandle;
use tokio::sync::broadcast::error::RecvError;

fn style() -> ProgressStyle {
    ProgressStyle::with_template("{prefix:<24!} [{bar:30}] {bytes:>10}/{total_bytes:<10} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("=> ")
}

fn label(t: &TaskSnapshot) -> String {
    t.spec.file_name.clone().unwrap_or_else(|| "…".to_owned())
}

/// Runs until no task is queued or running. Ctrl+C returns early; the caller's
/// `shutdown()` then checkpoints everything, and tasks resume on the next run.
pub async fn watch(engine: &EngineHandle) -> Result<()> {
    let mut events = engine.subscribe();
    let multi = MultiProgress::new();
    let mut bars: HashMap<TaskId, ProgressBar> = HashMap::new();
    let mut ctrl_c = std::pin::pin!(tokio::signal::ctrl_c());
    let mut idle_check = tokio::time::interval(Duration::from_millis(500));

    for t in engine.list().await? {
        if is_active(&t.state) {
            bars.insert(t.id, add_bar(&multi, &t));
        }
    }

    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                multi.println("Stopping… run `persivio watch` to continue later.")?;
                return Ok(());
            }
            _ = idle_check.tick() => {
                if !engine.list().await?.iter().any(|t| is_active(&t.state)) {
                    return Ok(());
                }
            }
            event = events.recv() => match event {
                Ok(EngineEvent::TaskAdded(t)) => {
                    bars.insert(t.id, add_bar(&multi, &t));
                }
                Ok(EngineEvent::MetadataResolved { id, name, size, .. }) => {
                    if let Some(bar) = bars.get(&id) {
                        bar.set_prefix(name);
                        if let Some(size) = size {
                            bar.set_length(size);
                        }
                    }
                }
                Ok(EngineEvent::ProgressBatch(batch)) => {
                    for p in batch {
                        if let Some(bar) = bars.get(&p.task_id) {
                            bar.set_position(p.downloaded);
                            if let Some(total) = p.total {
                                bar.set_length(total);
                            }
                            let eta = p.eta_secs.map_or(String::new(), |s| format!(" ETA {s}s"));
                            bar.set_message(format!("{}/s{eta}", HumanBytes(p.speed_bps)));
                        }
                    }
                }
                Ok(EngineEvent::StateChanged { id, state }) => {
                    if let Some(bar) = bars.get(&id) {
                        finish_or_label(bar, &state);
                    }
                }
                Ok(EngineEvent::Warning { message, .. }) => multi.println(format!("warning: {message}"))?,
                Ok(EngineEvent::TaskRemoved(id)) => {
                    if let Some(bar) = bars.remove(&id) {
                        bar.finish_and_clear();
                    }
                }
                // Missed events: the periodic list() check keeps us correct.
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return Ok(()),
            },
        }
    }
}

/// Tasks `watch` waits for. Seeding is excluded: it has no natural end, so the command
/// exits once every download is done (seeding torrents resume on the next run).
fn is_active(state: &TaskState) -> bool {
    let kind = state.kind();
    kind == TaskStateKind::Queued || (kind.is_running() && kind != TaskStateKind::Seeding)
}

fn add_bar(multi: &MultiProgress, t: &TaskSnapshot) -> ProgressBar {
    let bar = multi.add(ProgressBar::new(t.total_size.unwrap_or(0)));
    bar.set_style(style());
    bar.set_prefix(label(t));
    bar.set_position(t.downloaded);
    bar.set_message(t.state.kind().to_string());
    bar
}

fn finish_or_label(bar: &ProgressBar, state: &TaskState) {
    match state {
        TaskState::Completed => bar.finish_with_message("done"),
        TaskState::Failed { reason, .. } => bar.abandon_with_message(format!("failed: {reason}")),
        TaskState::Cancelled => bar.abandon_with_message("cancelled"),
        TaskState::Paused => bar.abandon_with_message("paused"),
        TaskState::Seeding => bar.finish_with_message("downloaded, seeding"),
        other => bar.set_message(other.kind().to_string()),
    }
}
