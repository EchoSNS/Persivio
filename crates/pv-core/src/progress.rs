//! Progress reporting.
//!
//! Protocols update a [`ProgressSink`] on every chunk; the engine's 250 ms ticker
//! [`sample`](ProgressSink::sample)s it and turns samples into [`Progress`] for the
//! `ProgressBatch` event (ADR-010). The sink is a handful of atomics, so the hot path
//! (one `fetch_add` per chunk) never takes a lock or allocates.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::ids::TaskId;

/// Sentinel meaning "unknown" in the optional atomic fields.
const NONE_U64: u64 = u64::MAX;
const NONE_U32: u32 = u32::MAX;

/// Live progress of one task, as sent to front ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    /// Which task.
    pub task_id: TaskId,
    /// Bytes downloaded.
    pub downloaded: u64,
    /// Total bytes, if known.
    pub total: Option<u64>,
    /// Current download speed (bytes/second, smoothed).
    pub speed_bps: u64,
    /// Estimated seconds remaining.
    pub eta_secs: Option<u64>,
    /// Open connections (HTTP) or active peer connections (torrent).
    pub connections_active: u16,
    /// Connected peers (torrents only).
    pub peers: Option<u32>,
    /// Upload speed (torrents only).
    pub upload_bps: Option<u64>,
}

/// A point-in-time read of a [`ProgressSink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressSample {
    /// Bytes downloaded.
    pub downloaded: u64,
    /// Total bytes, if known.
    pub total: Option<u64>,
    /// Active connections.
    pub connections_active: u16,
    /// Connected peers.
    pub peers: Option<u32>,
    /// Upload speed.
    pub upload_bps: Option<u64>,
}

impl ProgressSample {
    /// Builds a [`Progress`] given a speed computed by the caller (EWMA in the engine).
    #[must_use]
    pub fn to_progress(self, task_id: TaskId, speed_bps: u64) -> Progress {
        Progress {
            task_id,
            downloaded: self.downloaded,
            total: self.total,
            speed_bps,
            eta_secs: self
                .total
                .and_then(|t| eta_secs(t.saturating_sub(self.downloaded), speed_bps)),
            connections_active: self.connections_active,
            peers: self.peers,
            upload_bps: self.upload_bps,
        }
    }
}

/// Seconds to transfer `remaining` bytes at `speed_bps`, rounded up.
/// `None` when the speed is zero (stalled: no meaningful estimate).
#[must_use]
pub const fn eta_secs(remaining: u64, speed_bps: u64) -> Option<u64> {
    if speed_bps == 0 {
        None
    } else {
        Some(remaining.div_ceil(speed_bps))
    }
}

#[derive(Debug)]
struct Counters {
    downloaded: AtomicU64,
    total: AtomicU64,
    connections: AtomicU32,
    peers: AtomicU32,
    upload_bps: AtomicU64,
}

/// Cheap-to-clone, lock-free progress counters shared between a protocol run (writer)
/// and the engine ticker (reader).
///
/// All operations use `Ordering::Relaxed`: each field is an independent statistic and
/// a reader seeing slightly stale or mutually inconsistent values for one tick is
/// harmless. Nothing else is synchronized through these atomics.
#[derive(Debug, Clone)]
pub struct ProgressSink {
    inner: Arc<Counters>,
}

impl Default for ProgressSink {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressSink {
    /// Zero downloaded, everything else unknown.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Counters {
                downloaded: AtomicU64::new(0),
                total: AtomicU64::new(NONE_U64),
                connections: AtomicU32::new(0),
                peers: AtomicU32::new(NONE_U32),
                upload_bps: AtomicU64::new(NONE_U64),
            }),
        }
    }

    /// Adds freshly received bytes (hot path).
    pub fn add_downloaded(&self, bytes: u64) {
        self.inner.downloaded.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Sets the absolute byte count (resume, torrent stats).
    pub fn set_downloaded(&self, bytes: u64) {
        self.inner.downloaded.store(bytes, Ordering::Relaxed);
    }

    /// Sets the total size.
    pub fn set_total(&self, total: Option<u64>) {
        self.inner
            .total
            .store(total.unwrap_or(NONE_U64), Ordering::Relaxed);
    }

    /// Sets the number of active connections.
    pub fn set_connections(&self, n: u16) {
        self.inner
            .connections
            .store(u32::from(n), Ordering::Relaxed);
    }

    /// Sets the connected peer count (torrents).
    pub fn set_peers(&self, peers: Option<u32>) {
        self.inner
            .peers
            .store(peers.unwrap_or(NONE_U32), Ordering::Relaxed);
    }

    /// Sets the upload speed (torrents).
    pub fn set_upload_bps(&self, bps: Option<u64>) {
        self.inner
            .upload_bps
            .store(bps.unwrap_or(NONE_U64), Ordering::Relaxed);
    }

    /// Reads all counters.
    #[must_use]
    pub fn sample(&self) -> ProgressSample {
        let some_u64 = |v: u64| (v != NONE_U64).then_some(v);
        let peers = self.inner.peers.load(Ordering::Relaxed);
        ProgressSample {
            downloaded: self.inner.downloaded.load(Ordering::Relaxed),
            total: some_u64(self.inner.total.load(Ordering::Relaxed)),
            connections_active: u16::try_from(self.inner.connections.load(Ordering::Relaxed))
                .unwrap_or(u16::MAX),
            peers: (peers != NONE_U32).then_some(peers),
            upload_bps: some_u64(self.inner.upload_bps.load(Ordering::Relaxed)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eta_rounds_up_and_handles_stall() {
        assert_eq!(eta_secs(0, 10), Some(0));
        assert_eq!(eta_secs(10, 10), Some(1));
        assert_eq!(eta_secs(11, 10), Some(2));
        assert_eq!(eta_secs(10, 0), None);
    }

    #[test]
    fn sink_starts_unknown_and_tracks_updates() {
        let sink = ProgressSink::default();
        assert_eq!(
            sink.sample(),
            ProgressSample {
                downloaded: 0,
                total: None,
                connections_active: 0,
                peers: None,
                upload_bps: None
            }
        );
        let clone = sink.clone(); // shares the same counters
        clone.set_total(Some(1000));
        clone.add_downloaded(100);
        clone.add_downloaded(150);
        clone.set_connections(8);
        clone.set_peers(Some(12));
        clone.set_upload_bps(Some(64));
        let s = sink.sample();
        assert_eq!(s.downloaded, 250);
        assert_eq!(s.total, Some(1000));
        assert_eq!(s.connections_active, 8);
        assert_eq!(s.peers, Some(12));
        assert_eq!(s.upload_bps, Some(64));

        sink.set_downloaded(10);
        sink.set_total(None);
        sink.set_peers(None);
        sink.set_upload_bps(None);
        let s = sink.sample();
        assert_eq!(
            (s.downloaded, s.total, s.peers, s.upload_bps),
            (10, None, None, None)
        );
    }

    #[test]
    fn sink_is_consistent_under_concurrent_adds() {
        let sink = ProgressSink::new();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let s = sink.clone();
                std::thread::spawn(move || {
                    for _ in 0..10_000 {
                        s.add_downloaded(1);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(sink.sample().downloaded, 80_000);
    }

    #[test]
    fn sample_to_progress_computes_eta() {
        let id = TaskId::new();
        let sample = ProgressSample {
            downloaded: 400,
            total: Some(1000),
            connections_active: 4,
            peers: None,
            upload_bps: None,
        };
        let p = sample.to_progress(id, 200);
        assert_eq!(p.eta_secs, Some(3));
        assert_eq!(p.task_id, id);
        assert_eq!(sample.to_progress(id, 0).eta_secs, None);
        let unknown = ProgressSample {
            total: None,
            ..sample
        };
        assert_eq!(unknown.to_progress(id, 200).eta_secs, None);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Progress>(&json).unwrap(), p);
    }
}
