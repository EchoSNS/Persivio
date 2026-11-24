//! The shared librqbit session: created for the first running torrent, and closed again
//! when none has run for a while, so an idle Persivio has no listening port and no DHT
//! traffic (ADR-023).

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use librqbit::limits::LimitsConfig;
use librqbit::{
    DhtSessionConfig, ListenerMode, ListenerOptions, Session, SessionOptions,
    SessionPersistenceConfig,
};
use pv_core::{ProtocolError, Settings};
use tokio::sync::Mutex;

use crate::torrent_err;

/// How long the session outlives the last running torrent. Long enough that a quick
/// pause and resume does not have to rejoin the network.
pub(crate) const DEFAULT_IDLE: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Slot {
    session: Option<Arc<Session>>,
    /// Runs using the session.
    active: usize,
    /// Counts acquisitions, so a pending idle timer notices that it is outdated.
    generation: u64,
}

/// See the module docs.
pub(crate) struct SharedSession {
    slot: Mutex<Slot>,
    /// Private scratch directory for librqbit's own state (`None`: no fast resume).
    work_dir: Option<PathBuf>,
    idle: Duration,
    next_id: AtomicUsize,
}

impl SharedSession {
    pub(crate) fn new(work_dir: Option<PathBuf>, idle: Duration) -> Self {
        Self {
            slot: Mutex::default(),
            work_dir,
            idle,
            next_id: AtomicUsize::new(0),
        }
    }

    /// A torrent id. librqbit would pick `max + 1` without a lock, so two torrents added
    /// at once could get the same id, and the second the first one's handle.
    pub(crate) fn next_id(&self) -> usize {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Where librqbit looks for a torrent's piece bitfield.
    pub(crate) fn bitfield_path(&self, info_hash: &str) -> Option<PathBuf> {
        self.work_dir
            .as_ref()
            .map(|d| d.join(format!("{info_hash}.bitv")))
    }

    /// The running session, created if needed. Every successful call must be paired with
    /// one [`release`](Self::release).
    pub(crate) async fn acquire(
        &self,
        settings: &Settings,
        dest: &Path,
    ) -> Result<Arc<Session>, ProtocolError> {
        let mut slot = self.slot.lock().await;
        let session = if let Some(session) = &slot.session {
            session.clone()
        } else {
            let session = open(settings, dest, self.work_dir.as_deref()).await?;
            slot.session = Some(session.clone());
            session
        };
        slot.active += 1;
        slot.generation += 1;
        Ok(session)
    }

    /// Ends one use. The last one starts the idle timer.
    pub(crate) async fn release(self: &Arc<Self>) {
        let mut slot = self.slot.lock().await;
        slot.active = slot.active.saturating_sub(1);
        if slot.active > 0 {
            return;
        }
        let generation = slot.generation;
        drop(slot);
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(this.idle).await;
            let mut slot = this.slot.lock().await;
            if slot.active == 0 && slot.generation == generation {
                close(&mut slot).await;
            }
        });
    }

    pub(crate) async fn stop(&self) {
        close(&mut *self.slot.lock().await).await;
    }

    pub(crate) async fn is_open(&self) -> bool {
        self.slot.lock().await.session.is_some()
    }

    /// Applies a changed global download limit to the running session.
    pub(crate) async fn set_download_limit(&self, bps: Option<u64>) {
        if let Some(session) = &self.slot.lock().await.session {
            session.ratelimits.set_download_bps(limit(bps));
        }
    }
}

/// Stops the session while the slot stays locked: a new one must not try to listen on
/// the same port before the old one let go of it.
async fn close(slot: &mut Slot) {
    if let Some(session) = slot.session.take() {
        session.stop().await;
        tracing::debug!("BitTorrent session closed");
    }
}

pub(crate) fn limit(bps: Option<u64>) -> Option<NonZeroU32> {
    bps.and_then(|b| NonZeroU32::new(u32::try_from(b).unwrap_or(u32::MAX)))
}

async fn open(
    settings: &Settings,
    dest: &Path,
    work_dir: Option<&Path>,
) -> Result<Arc<Session>, ProtocolError> {
    if let Some(dir) = work_dir {
        // librqbit would restore *and start* torrents it finds here. Persivio's database
        // is the source of truth (ADR-012), so a session always starts empty.
        match tokio::fs::remove_dir_all(dir).await {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    let t = &settings.torrent;
    let trackers = if t.use_extra_trackers {
        t.extra_trackers
            .iter()
            .filter_map(|u| u.parse().ok())
            .collect()
    } else {
        HashSet::new()
    };
    let opts = SessionOptions {
        // DHT off = `None` (tests and privacy-conscious users). No DHT cache file:
        // librqbit would write it outside Persivio's folders (ADR-016).
        dht: t.dht.then(|| DhtSessionConfig {
            persistence: None,
            ..DhtSessionConfig::default()
        }),
        persistence: work_dir.map(|dir| SessionPersistenceConfig::Json {
            folder: Some(dir.to_path_buf()),
        }),
        fastresume: work_dir.is_some(),
        listen: Some(ListenerOptions {
            mode: ListenerMode::TcpOnly,
            listen_addr: SocketAddr::from((Ipv4Addr::UNSPECIFIED, t.listen_port.unwrap_or(0))),
            enable_upnp_port_forwarding: t.upnp,
            ..ListenerOptions::default()
        }),
        trackers,
        ratelimits: LimitsConfig {
            download_bps: limit(settings.global_speed_limit_bps),
            upload_bps: None,
        },
        disable_local_service_discovery: !t.dht,
        ..SessionOptions::default()
    };
    Session::new_with_opts(dest.to_path_buf(), opts)
        .await
        .map_err(|e| torrent_err(format!("cannot start BitTorrent session: {e:#}")))
}
