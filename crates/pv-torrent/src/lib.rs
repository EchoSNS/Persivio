//! BitTorrent / magnet support as an adapter over `librqbit` implementing the
//! `pv-core` `Protocol` trait (ADR-006).
//!
//! **Boundary:** this is the only crate allowed to depend on `librqbit`, and no
//! librqbit type appears in its public API. See ADR-016 for how librqbit's model is
//! mapped onto Persivio's (shared session, list-only metadata step, path validation,
//! seeding limits, no librqbit persistence).

mod motw;
mod paths;
mod seeding;
#[cfg(feature = "testing")]
pub mod testing;

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use librqbit::api::TorrentIdOrHash;
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, ListenerMode,
    ListenerOptions, Magnet, Session, SessionOptions, TorrentStatsState,
};
use pv_core::{
    FileEntry, Outcome, Protocol, ProtocolError, ResolvedMetadata, RunContext, RunPhase, Settings,
    Source,
};
use tokio::sync::OnceCell;

pub use paths::validate_torrent_path;
pub use seeding::seeding_done;

/// How often torrent stats are copied into the task's progress counters.
const POLL: Duration = Duration::from_millis(250);

/// Parsed facts about a magnet link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnetInfo {
    /// Lower-case hex v1 info hash.
    pub info_hash: String,
    /// The `dn` display name, if present.
    pub name: Option<String>,
    /// Tracker URLs (`tr`).
    pub trackers: Vec<String>,
}

/// Parses a `magnet:?` URI.
///
/// # Errors
/// [`ProtocolError::InvalidSource`] if it is not a valid BitTorrent v1 magnet link.
pub fn parse_magnet(uri: &str) -> Result<MagnetInfo, ProtocolError> {
    let magnet = Magnet::parse(uri).map_err(|e| ProtocolError::InvalidSource(format!("{e:#}")))?;
    let id = magnet
        .as_id20()
        .ok_or_else(|| ProtocolError::InvalidSource("magnet has no v1 info hash".to_owned()))?;
    Ok(MagnetInfo {
        info_hash: id.as_string(),
        name: magnet.name.clone(),
        trackers: magnet.trackers.clone(),
    })
}

/// Downloads `Source::Magnet` and `Source::TorrentFile` tasks through one shared
/// librqbit session (one listening port, one DHT node for the whole app).
#[derive(Default)]
pub struct TorrentProtocol {
    session: OnceCell<Arc<Session>>,
    /// Peers to contact directly, in addition to trackers/DHT (LAN use and tests).
    initial_peers: Vec<SocketAddr>,
}

impl TorrentProtocol {
    /// A protocol whose session is created on first use from the app settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Also connects to these peers for every torrent (local seeding, tests).
    #[must_use]
    pub fn with_initial_peers(mut self, peers: Vec<SocketAddr>) -> Self {
        self.initial_peers = peers;
        self
    }

    /// The shared session. Settings that only apply at creation (port, DHT, `UPnP`,
    /// extra trackers, global limit) are read from the first task's settings.
    async fn session(
        &self,
        settings: &Settings,
        dest: &Path,
    ) -> Result<&Arc<Session>, ProtocolError> {
        self.session
            .get_or_try_init(|| async {
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
                    // DHT off = `None` (tests and privacy-conscious users). No DHT cache
                    // file: librqbit would write it outside Persivio's folders (ADR-016).
                    dht: t.dht.then(|| DhtSessionConfig {
                        persistence: None,
                        ..DhtSessionConfig::default()
                    }),
                    // Persivio's database is the source of truth (ADR-012, ADR-016).
                    persistence: None,
                    fastresume: false,
                    listen: Some(ListenerOptions {
                        mode: ListenerMode::TcpOnly,
                        listen_addr: SocketAddr::from((
                            Ipv4Addr::UNSPECIFIED,
                            t.listen_port.unwrap_or(0),
                        )),
                        enable_upnp_port_forwarding: t.upnp,
                        ..ListenerOptions::default()
                    }),
                    trackers,
                    ratelimits: LimitsConfig {
                        download_bps: settings
                            .global_speed_limit_bps
                            .and_then(|b| NonZeroU32::new(u32::try_from(b).unwrap_or(u32::MAX))),
                        upload_bps: None,
                    },
                    disable_local_service_discovery: !t.dht,
                    ..SessionOptions::default()
                };
                Session::new_with_opts(dest.to_path_buf(), opts)
                    .await
                    .map_err(|e| {
                        ProtocolError::Torrent(format!("cannot start BitTorrent session: {e:#}"))
                    })
            })
            .await
    }

    /// Stops the session (app shutdown).
    pub async fn stop(&self) {
        if let Some(s) = self.session.get() {
            s.stop().await;
        }
    }
}

fn torrent_err(e: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::Torrent(e.to_string())
}

/// Removes the torrent from the session (keeping its files) so it stops transferring.
async fn detach(session: &Arc<Session>, id: usize) {
    if let Err(e) = session.delete(TorrentIdOrHash::from(id), false).await {
        tracing::debug!(%e, "detaching torrent");
    }
}

#[async_trait]
impl Protocol for TorrentProtocol {
    fn handles(&self, source: &Source) -> bool {
        matches!(source, Source::Magnet { .. } | Source::TorrentFile { .. })
    }

    async fn shutdown(&self) {
        self.stop().await;
    }

    async fn run(&self, ctx: RunContext) -> Result<Outcome, ProtocolError> {
        let dest = ctx.task.spec.dest_dir.clone();
        tokio::fs::create_dir_all(&dest).await?;
        let session = self.session(&ctx.settings, &dest).await?;
        let initial_peers = (!self.initial_peers.is_empty()).then(|| self.initial_peers.clone());

        // 1) Metadata only: file list + .torrent bytes, nothing written yet.
        let add = match (&ctx.resume.torrent, &ctx.task.spec.source) {
            (Some(saved), _) => AddTorrent::from_bytes(saved.torrent_bytes.clone()),
            (None, Source::Magnet { uri }) => {
                ctx.events.phase(RunPhase::FetchingMetadata);
                AddTorrent::from_url(uri.clone())
            }
            (None, Source::TorrentFile { bytes }) => AddTorrent::from_bytes(bytes.clone()),
            (None, Source::Http { .. }) => {
                return Err(ProtocolError::InvalidSource("not a torrent".to_owned()));
            }
        };
        let list_opts = AddTorrentOptions {
            list_only: true,
            initial_peers: initial_peers.clone(),
            output_folder: Some(dest.to_string_lossy().into_owned()),
            ..AddTorrentOptions::default()
        };
        let listed = tokio::select! {
            biased;
            () = ctx.cancel.cancelled() => return Ok(Outcome::Stopped),
            r = session.add_torrent(add, Some(list_opts)) => r.map_err(|e| torrent_err(format!("{e:#}")))?,
        };
        let AddTorrentResponse::ListOnly(listed) = listed else {
            return Err(torrent_err("unexpected response to a metadata request"));
        };

        // 2) Validate every path before anything touches the disk.
        let layout = paths::Layout::from_info(
            &listed.info,
            ctx.task.spec.file_name.as_deref(),
            &dest,
            ctx.resume.torrent.is_some(),
        )
        .await?;
        let selected: Option<HashSet<u32>> = ctx
            .task
            .spec
            .selected_files
            .as_ref()
            .map(|v| v.iter().copied().collect());
        let files: Vec<FileEntry> = layout
            .files
            .iter()
            .zip(0u32..)
            .map(|((path, size), index)| FileEntry {
                index,
                path: path.clone(),
                size: *size,
                selected: selected.as_ref().is_none_or(|s| s.contains(&index)),
            })
            .collect();
        let total: u64 = files.iter().filter(|f| f.selected).map(|f| f.size).sum();
        let info_hash = listed.info_hash.as_string();
        ctx.checkpoint
            .save_torrent(ctx.task.id, &info_hash, &listed.torrent_bytes)
            .await?;
        ctx.events.metadata(ResolvedMetadata {
            name: Some(layout.root_name.clone()),
            total_size: Some(total),
            files,
            http: None,
        });

        // 3) The real download.
        let only_files = selected.map(|s| {
            s.into_iter()
                .filter_map(|i| usize::try_from(i).ok())
                .collect()
        });
        let opts = AddTorrentOptions {
            overwrite: true, // reuse (and re-check) existing pieces on resume
            output_folder: Some(layout.output_folder.to_string_lossy().into_owned()),
            only_files,
            initial_peers,
            ratelimits: LimitsConfig {
                download_bps: ctx
                    .task
                    .spec
                    .speed_limit_bps
                    .and_then(|b| NonZeroU32::new(u32::try_from(b).unwrap_or(u32::MAX))),
                upload_bps: None,
            },
            ..AddTorrentOptions::default()
        };
        let response = session
            .add_torrent(
                AddTorrent::from_bytes(listed.torrent_bytes.clone()),
                Some(opts),
            )
            .await
            .map_err(|e| torrent_err(format!("{e:#}")))?;
        let (id, handle) = match response {
            AddTorrentResponse::Added(id, h) | AddTorrentResponse::AlreadyManaged(id, h) => (id, h),
            AddTorrentResponse::ListOnly(_) => return Err(torrent_err("torrent was not added")),
        };
        ctx.events.phase(RunPhase::Downloading);
        ctx.progress.set_total(Some(total));

        follow(&ctx, session, id, &handle, total, layout.root_path).await
    }
}

/// Mirrors torrent stats into the task's progress until it finishes (and, if seeding
/// is enabled, until a seeding limit is reached), or the task is cancelled.
async fn follow(
    ctx: &RunContext,
    session: &Arc<Session>,
    id: usize,
    handle: &librqbit::ManagedTorrent,
    total: u64,
    root_path: std::path::PathBuf,
) -> Result<Outcome, ProtocolError> {
    let mut seeding_since: Option<Instant> = None;
    loop {
        tokio::select! {
            biased;
            () = ctx.cancel.cancelled() => {
                detach(session, id).await;
                return Ok(Outcome::Stopped);
            }
            () = tokio::time::sleep(POLL) => {}
        }
        let stats = handle.stats();
        ctx.progress.set_downloaded(stats.progress_bytes.min(total));
        if let Some(live) = &stats.live {
            let peers = live.snapshot.peer_stats.live;
            ctx.progress.set_peers(Some(peers));
            ctx.progress
                .set_connections(u16::try_from(peers).unwrap_or(u16::MAX));
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "speed in MiB/s → B/s, saturating"
            )]
            let up = (live.upload_speed.mbps * 1024.0 * 1024.0) as u64;
            ctx.progress.set_upload_bps(Some(up));
        }
        if matches!(stats.state, TorrentStatsState::Error) {
            detach(session, id).await;
            return Err(torrent_err(
                stats
                    .error
                    .unwrap_or_else(|| "unknown torrent error".to_owned()),
            ));
        }
        if !stats.finished {
            continue;
        }
        if seeding_since.is_none() {
            // Every wanted piece is on disk: mark the files before anyone opens them.
            motw::mark_all(root_path.clone()).await;
            if !ctx.settings.torrent.never_seed {
                ctx.events.phase(RunPhase::Seeding);
            }
        }
        let started = *seeding_since.get_or_insert_with(Instant::now);
        if seeding_done(
            &ctx.settings.torrent,
            stats.uploaded_bytes,
            total,
            started.elapsed(),
        ) {
            detach(session, id).await;
            return Ok(Outcome::Completed { path: root_path });
        }
    }
}

/// Where a torrent's files go, decided once and reproducible on resume.
pub use paths::Layout;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnets_parse() {
        let m = parse_magnet(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=ubuntu.iso&tr=udp%3A%2F%2Ft.example%3A80",
        )
        .unwrap();
        assert_eq!(m.info_hash, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(m.name.as_deref(), Some("ubuntu.iso"));
        assert_eq!(m.trackers, ["udp://t.example:80"]);
        assert!(parse_magnet("magnet:?dn=nohash").is_err());
        assert!(parse_magnet("https://example.com").is_err());
    }

    #[test]
    fn handles_only_torrent_sources() {
        let p = TorrentProtocol::new();
        assert!(p.handles(&Source::magnet("magnet:?xt=urn:btih:abc").unwrap()));
        assert!(p.handles(&Source::TorrentFile { bytes: vec![1] }));
        assert!(!p.handles(&Source::http(url::Url::parse("https://e.x/f").unwrap()).unwrap()));
    }
}
