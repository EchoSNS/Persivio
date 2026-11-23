//! Moves engine events to the UI: progress through a Tauri channel, at most
//! [`PROGRESS_INTERVAL`] apart, and everything else as typed app events.
//!
//! Both loops only wake up when the engine sends something, so an idle app costs no
//! CPU (Phase 7 acceptance criterion).

use std::time::Duration;

use pv_core::EngineEvent;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;

use crate::dto::{EngineUiEvent, ProgressView};

/// Minimum time between two progress messages to the UI (≤ 4 Hz).
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Forwards every non-progress event through `emit` until the engine stops. A lagging
/// receiver (events were dropped) emits [`EngineUiEvent::Resync`] so the UI reloads.
pub async fn forward_events(
    mut events: broadcast::Receiver<EngineEvent>,
    mut emit: impl FnMut(EngineUiEvent),
) {
    loop {
        match events.recv().await {
            Ok(event) => {
                if let Some(ui) = EngineUiEvent::from_engine(&event) {
                    emit(ui);
                }
            }
            Err(RecvError::Lagged(_)) => emit(EngineUiEvent::Resync),
            Err(RecvError::Closed) => return,
        }
    }
}

/// Forwards progress batches through `send`, coalescing bursts so that at most one
/// batch is sent per `interval` and the latest values always arrive. Each engine batch
/// covers every running task, so the newest batch replaces any pending one.
///
/// Ends when `cancel` fires, the engine stops, or `send` returns `false` (the UI's
/// channel is gone).
pub async fn forward_progress(
    mut events: broadcast::Receiver<EngineEvent>,
    cancel: CancellationToken,
    interval: Duration,
    mut send: impl FnMut(Vec<ProgressView>) -> bool,
) {
    let mut pending: Option<Vec<ProgressView>> = None;
    let mut next_allowed = Instant::now();
    loop {
        let flush_at = pending.as_ref().map(|_| next_allowed);
        tokio::select! {
            biased;
            () = cancel.cancelled() => return,
            () = sleep_until(flush_at.unwrap_or_else(Instant::now)), if flush_at.is_some() => {}
            received = events.recv() => match received {
                Ok(EngineEvent::ProgressBatch(batch)) => {
                    pending = Some(batch.iter().map(ProgressView::from).collect());
                }
                // Progress is absolute (not deltas): a missed batch needs no recovery.
                Ok(_) | Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return,
            },
        }
        let now = Instant::now();
        if now >= next_allowed
            && let Some(batch) = pending.take()
        {
            if !send(batch) {
                return;
            }
            next_allowed = now + interval;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use pv_core::{Progress, TaskId};

    use super::*;

    fn batch(id: TaskId, downloaded: u64) -> EngineEvent {
        EngineEvent::ProgressBatch(vec![Progress {
            task_id: id,
            downloaded,
            total: Some(1000),
            speed_bps: 10,
            eta_secs: None,
            connections_active: 1,
            peers: None,
            upload_bps: None,
        }])
    }

    #[tokio::test(start_paused = true)]
    async fn bursts_are_coalesced_to_four_per_second_and_the_last_value_wins() {
        let (tx, rx) = broadcast::channel(64);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let sent = Arc::clone(&sent);
            let cancel = cancel.clone();
            async move {
                forward_progress(rx, cancel, PROGRESS_INTERVAL, |b| {
                    sent.lock().unwrap().push((Instant::now(), b[0].downloaded));
                    true
                })
                .await;
            }
        });
        let id = TaskId::new();
        let start = Instant::now();
        // 40 batches over one second (40 Hz).
        for i in 1..=40 {
            tx.send(batch(id, i)).unwrap();
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        cancel.cancel();
        task.await.unwrap();

        let sent = sent.lock().unwrap().clone();
        assert!(
            (4..=6).contains(&sent.len()),
            "about 4 per second, got {}",
            sent.len()
        );
        for pair in sent.windows(2) {
            assert!(pair[1].0 - pair[0].0 >= PROGRESS_INTERVAL, "{sent:?}");
        }
        assert_eq!(
            sent.first().unwrap().0,
            start,
            "the first batch is not delayed"
        );
        assert_eq!(sent.last().unwrap().1, 40, "the latest value is delivered");
    }

    #[tokio::test(start_paused = true)]
    async fn stops_when_the_ui_channel_is_gone_or_the_engine_stops() {
        let (tx, rx) = broadcast::channel(8);
        let forward = tokio::spawn(forward_progress(
            rx,
            CancellationToken::new(),
            PROGRESS_INTERVAL,
            |_| false,
        ));
        tx.send(batch(TaskId::new(), 1)).unwrap();
        forward.await.unwrap();

        let (tx, rx) = broadcast::channel::<EngineEvent>(8);
        let forward = tokio::spawn(forward_progress(
            rx,
            CancellationToken::new(),
            PROGRESS_INTERVAL,
            |_| true,
        ));
        drop(tx);
        forward.await.unwrap();
    }

    #[tokio::test]
    async fn events_are_mapped_and_lag_triggers_a_resync() {
        let (tx, rx) = broadcast::channel(2);
        let id = TaskId::new();
        // Four events into a channel of two: the receiver lags.
        tx.send(batch(id, 1)).unwrap();
        tx.send(EngineEvent::TaskRemoved(id)).unwrap();
        tx.send(EngineEvent::TaskRemoved(id)).unwrap();
        tx.send(batch(id, 2)).unwrap();
        drop(tx);
        let mut got = Vec::new();
        forward_events(rx, |e| got.push(e)).await;
        assert_eq!(
            got,
            [
                EngineUiEvent::Resync,
                EngineUiEvent::TaskRemoved { id: id.to_string() },
            ],
            "progress is not an app event"
        );
    }
}
