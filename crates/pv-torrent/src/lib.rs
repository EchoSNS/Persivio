//! BitTorrent / magnet support as an adapter over `librqbit` implementing the
//! `pv-core` `Protocol` trait (ADR-006).
//!
//! **Boundary:** this is the only crate allowed to depend on `librqbit`, and no
//! librqbit type appears in its public API. See ADR-016 for how librqbit's model is
//! mapped onto Persivio's (shared session, list-only metadata step, path validation,
//! seeding limits), and ADR-023 for resuming without a full re-check.

mod bencode;
mod have;
mod motw;
mod paths;
mod seeding;
mod session;
#[cfg(feature = "testing")]
pub mod testing;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use librqbit::api::{Api, TorrentIdOrHash};
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ListOnlyResponse, Magnet, Session,
    TorrentStatsState,
};
use pv_core::{
    FileEntry, Outcome, Protocol, ProtocolError, ResolvedMetadata, RunContext, RunPhase, Settings,
    Source,
};

use have::HaveMap;
use session::SharedSession;

pub use paths::validate_torrent_path;
pub use seeding::seeding_done;

/// With no connected peer for this long, peers are searched again (then after twice as
/// long, and so on up to [`MAX_STALL`], to stay polite to trackers).
const STALL: Duration = Duration::from_mins(1);

/// Longest interval between two searches.
const MAX_STALL: Duration = Duration::from_mins(10);

/// How often torrent stats are copied into the task's progress counters.
const POLL: Duration = Duration::from_millis(250);

/// The same while existing data is being checked, so a fast check ends promptly.
const CHECK_POLL: Duration = Duration::from_millis(50);

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
pub struct TorrentProtocol {
    session: Arc<SharedSession>,
    /// Peers to contact directly, in addition to trackers/DHT (LAN use and tests).
    initial_peers: Vec<SocketAddr>,
    stall: Duration,
}

impl Default for TorrentProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl TorrentProtocol {
    /// A protocol whose session is created on first use from the app settings. Without
    /// a [work directory](Self::with_work_dir) every resume re-checks all data.
    #[must_use]
    pub fn new() -> Self {
        Self::with_options(None, session::DEFAULT_IDLE)
    }

    /// Enables fast resume. `dir` must be used by nothing else: it is **emptied** each
    /// time the session starts.
    #[must_use]
    pub fn with_work_dir(dir: impl Into<PathBuf>) -> Self {
        Self::with_options(Some(dir.into()), session::DEFAULT_IDLE)
    }

    /// [`with_work_dir`](Self::with_work_dir) with a custom idle time before the session
    /// closes.
    #[must_use]
    pub fn with_options(work_dir: Option<PathBuf>, idle: Duration) -> Self {
        Self {
            session: Arc::new(SharedSession::new(work_dir, idle)),
            initial_peers: Vec::new(),
            stall: STALL,
        }
    }

    /// How long a torrent may have no connected peer before peers are searched again.
    #[must_use]
    pub fn with_stall_timeout(mut self, stall: Duration) -> Self {
        self.stall = stall;
        self
    }

    /// Also connects to these peers for every torrent (local seeding, tests).
    #[must_use]
    pub fn with_initial_peers(mut self, peers: Vec<SocketAddr>) -> Self {
        self.initial_peers = peers;
        self
    }

    /// Stops the session (app shutdown).
    pub async fn stop(&self) {
        self.session.stop().await;
    }

    /// Whether the BitTorrent session (listening port, DHT) is currently open.
    pub async fn is_online(&self) -> bool {
        self.session.is_open().await
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

    fn settings_changed(&self, settings: &Settings) {
        let (session, limit) = (self.session.clone(), settings.global_speed_limit_bps);
        tokio::spawn(async move { session.set_download_limit(limit).await });
    }

    async fn run(&self, ctx: RunContext) -> Result<Outcome, ProtocolError> {
        tokio::fs::create_dir_all(&ctx.task.spec.dest_dir).await?;
        let session = self
            .session
            .acquire(&ctx.settings, &ctx.task.spec.dest_dir)
            .await?;
        let result = self.run_in(&ctx, &session).await;
        drop(session);
        self.session.release().await;
        result
    }
}

impl TorrentProtocol {
    async fn run_in(
        &self,
        ctx: &RunContext,
        session: &Arc<Session>,
    ) -> Result<Outcome, ProtocolError> {
        let dest = ctx.task.spec.dest_dir.clone();

        // 1) Metadata only: file list + .torrent bytes, nothing written yet.
        let Some(listed) = self.list(ctx, session).await? else {
            return Ok(Outcome::Stopped);
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
            files: files.clone(),
            http: None,
        });

        // 3) The real download.
        let only_files = selected.map(|s| {
            s.into_iter()
                .filter_map(|i| usize::try_from(i).ok())
                .collect()
        });
        let paths: Vec<PathBuf> = layout
            .files
            .iter()
            .map(|(rel, _)| layout.output_folder.join(rel))
            .collect();
        ensure_space(&dest, &layout, &files).await?;
        let resuming = ctx.resume.torrent.is_some();
        self.prepare_fast_resume(ctx, &info_hash, &paths).await;
        let opts = AddTorrentOptions {
            preferred_id: Some(self.session.next_id()),
            overwrite: true, // reuse (and re-check) existing pieces on resume
            output_folder: Some(layout.output_folder.to_string_lossy().into_owned()),
            only_files,
            initial_peers: self.peers(),
            ratelimits: LimitsConfig {
                download_bps: session::limit(ctx.task.spec.speed_limit_bps),
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
            AddTorrentResponse::Added(id, h) => (id, h),
            // Two tasks for one torrent would fight over the same files.
            AddTorrentResponse::AlreadyManaged(..) => {
                return Err(torrent_err("this torrent is already being downloaded"));
            }
            AddTorrentResponse::ListOnly(_) => return Err(torrent_err("torrent was not added")),
        };
        ctx.progress.set_total(Some(total));
        // A first run has nothing to check, so the (instant) check is not worth showing.
        ctx.events.phase(if resuming {
            RunPhase::Checking
        } else {
            RunPhase::Downloading
        });
        Active {
            ctx,
            session,
            id,
            handle: &handle,
            total,
            paths,
            root_path: layout.root_path,
            stall: self.stall,
        }
        .follow(resuming)
        .await
    }

    fn peers(&self) -> Option<Vec<SocketAddr>> {
        (!self.initial_peers.is_empty()).then(|| self.initial_peers.clone())
    }

    /// Resolves the torrent's metadata without starting it. `None`: cancelled.
    async fn list(
        &self,
        ctx: &RunContext,
        session: &Arc<Session>,
    ) -> Result<Option<ListOnlyResponse>, ProtocolError> {
        let add = match (&ctx.resume.torrent, &ctx.task.spec.source) {
            (Some(saved), _) => AddTorrent::from_bytes(saved.torrent_bytes.clone()),
            (None, Source::Magnet { uri }) => {
                ctx.events.phase(RunPhase::FetchingMetadata);
                AddTorrent::from_url(uri.clone())
            }
            (None, Source::TorrentFile { bytes }) => {
                bencode::check(bytes).map_err(|e| {
                    ProtocolError::InvalidSource(format!("not a valid .torrent file: {e}"))
                })?;
                AddTorrent::from_bytes(bytes.clone())
            }
            (None, Source::Http { .. }) => {
                return Err(ProtocolError::InvalidSource("not a torrent".to_owned()));
            }
        };
        let list_opts = AddTorrentOptions {
            list_only: true,
            initial_peers: self.peers(),
            output_folder: Some(ctx.task.spec.dest_dir.to_string_lossy().into_owned()),
            ..AddTorrentOptions::default()
        };
        let listed = tokio::select! {
            biased;
            () = ctx.cancel.cancelled() => return Ok(None),
            r = session.add_torrent(add, Some(list_opts)) => r.map_err(|e| torrent_err(format!("{e:#}")))?,
        };
        match listed {
            AddTorrentResponse::ListOnly(listed) => Ok(Some(listed)),
            _ => Err(torrent_err("unexpected response to a metadata request")),
        }
    }

    /// Hands librqbit the saved piece map, if the files it describes are unchanged.
    /// Any problem only means a full check.
    async fn prepare_fast_resume(&self, ctx: &RunContext, info_hash: &str, paths: &[PathBuf]) {
        let Some(target) = self.session.bitfield_path(info_hash) else {
            return;
        };
        let saved = ctx
            .resume
            .torrent
            .as_ref()
            .and_then(|t| t.have.as_deref())
            .and_then(HaveMap::from_bytes);
        let bits = match saved {
            Some(map) => {
                let paths = paths.to_vec();
                tokio::task::spawn_blocking(move || map.validated_bits(&paths))
                    .await
                    .ok()
                    .flatten()
            }
            None => None,
        };
        let result = match bits {
            Some(bits) => tokio::fs::write(&target, bits).await,
            None => match tokio::fs::remove_file(&target).await {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                r => r,
            },
        };
        if let Err(e) = result {
            tracing::warn!(%e, "fast resume is not available for this run");
        }
    }
}

/// One torrent in the session.
struct Active<'a> {
    ctx: &'a RunContext,
    session: &'a Arc<Session>,
    id: usize,
    handle: &'a Arc<librqbit::ManagedTorrent>,
    /// Bytes of the selected files.
    total: u64,
    /// Every file of the torrent.
    paths: Vec<PathBuf>,
    root_path: PathBuf,
    stall: Duration,
}

impl Active<'_> {
    /// Mirrors torrent stats into the task's progress until it finishes (and, if seeding
    /// is enabled, until a seeding limit is reached), or the task is cancelled.
    async fn follow(self, resuming: bool) -> Result<Outcome, ProtocolError> {
        let ctx = self.ctx;
        let mut checking = true;
        let mut seeding_since: Option<Instant> = None;
        let mut stall = Stall::new(self.stall);
        loop {
            tokio::select! {
                biased;
                () = ctx.cancel.cancelled() => {
                    self.stop_and_save().await;
                    return Ok(Outcome::Stopped);
                }
                () = tokio::time::sleep(if checking { CHECK_POLL } else { POLL }) => {}
            }
            let stats = self.handle.stats();
            match stats.state {
                // Here `progress_bytes` counts *hashed* bytes, good or bad, of all files.
                TorrentStatsState::Initializing { .. } => {
                    if resuming {
                        ctx.progress.set_check_percent(Some(percent(
                            stats.progress_bytes,
                            stats.total_bytes,
                        )));
                    }
                    continue;
                }
                TorrentStatsState::Error => {
                    detach(self.session, self.id).await;
                    return Err(run_error(stats.error));
                }
                TorrentStatsState::Live | TorrentStatsState::Paused => {}
            }
            let downloaded = stats.progress_bytes.min(self.total);
            if checking {
                checking = false;
                ctx.progress.set_check_percent(None);
                ctx.progress.rebase(downloaded);
                // From here the data changes, so the saved map is no longer valid.
                ctx.checkpoint
                    .save_torrent_progress(ctx.task.id, downloaded, None)
                    .await?;
                if resuming {
                    ctx.events.phase(RunPhase::Downloading);
                }
            }
            ctx.progress.set_downloaded(downloaded);
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
            if !stats.finished {
                let connected = stats
                    .live
                    .as_ref()
                    .is_some_and(|l| l.snapshot.peer_stats.live > 0);
                if stall.is_over(connected, Instant::now()) {
                    self.search_peers_again().await;
                }
                continue;
            }
            if seeding_since.is_none() {
                // Every wanted piece is on disk: mark the files before anyone opens them.
                motw::mark_all(self.root_path.clone()).await;
                if !ctx.settings.torrent.never_seed {
                    ctx.events.phase(RunPhase::Seeding);
                }
            }
            let started = *seeding_since.get_or_insert_with(Instant::now);
            if seeding_done(
                &ctx.settings.torrent,
                stats.uploaded_bytes,
                self.total,
                started.elapsed(),
            ) {
                detach(self.session, self.id).await;
                ctx.checkpoint
                    .save_torrent_progress(ctx.task.id, downloaded, None)
                    .await?;
                return Ok(Outcome::Completed {
                    path: self.root_path,
                });
            }
        }
    }

    /// Restarts the torrent's networking: known peers are tried at once, and trackers and
    /// the DHT are asked again. librqbit backs off from a peer that failed once for 10 s,
    /// then minutes, up to an hour, which stalls a torrent with few peers. Pieces are kept.
    async fn search_peers_again(&self) {
        tracing::info!(id = %self.ctx.task.id, "no peers connected: searching again");
        let result = match self.session.pause(self.handle).await {
            Ok(()) => self.session.unpause(self.handle).await,
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            tracing::warn!(%e, "could not restart the torrent");
        }
    }

    /// Stops the torrent and records which pieces are on disk (ADR-023). Failing to
    /// record them only costs a full check on the next resume.
    async fn stop_and_save(&self) {
        let mut bits = None;
        if let Err(e) = self.session.pause(self.handle).await {
            tracing::debug!(%e, "pausing torrent");
        }
        // Still checking (or failed): the previous record, if any, stays as it is.
        let api = Api::new(self.session.clone(), None);
        if let Ok((have, _)) = api.api_dump_haves(TorrentIdOrHash::from(self.id)) {
            bits = Some(have.as_raw_slice().to_vec());
        }
        let downloaded = self.handle.stats().progress_bytes.min(self.total);
        detach(self.session, self.id).await;
        let Some(bits) = bits else { return };
        let paths = self.paths.clone();
        let captured = tokio::task::spawn_blocking(move || HaveMap::capture(bits, &paths)).await;
        let record = match captured {
            Ok(Ok(map)) => map.to_bytes(),
            Ok(Err(e)) => {
                tracing::warn!(%e, "could not sync the torrent's files");
                None
            }
            Err(_) => None,
        };
        if let Err(e) = self
            .ctx
            .checkpoint
            .save_torrent_progress(self.ctx.task.id, downloaded, record.as_deref())
            .await
        {
            tracing::warn!(%e, "could not save torrent progress");
        }
    }
}

/// Fails early and clearly if the selected files cannot fit. What exists already counts
/// as present, so this never refuses a download that would have worked.
async fn ensure_space(
    dest: &Path,
    layout: &paths::Layout,
    files: &[FileEntry],
) -> Result<(), ProtocolError> {
    let wanted: Vec<(PathBuf, u64)> = layout
        .files
        .iter()
        .zip(files)
        .filter(|(_, entry)| entry.selected)
        .map(|((rel, size), _)| (layout.output_folder.join(rel), *size))
        .collect();
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let needed: u64 = wanted
            .iter()
            .map(|(path, size)| {
                let present = std::fs::metadata(path).map_or(0, |m| m.len());
                size.saturating_sub(present)
            })
            .sum();
        let available = fs4::available_space(&dest)?;
        if available < needed {
            return Err(ProtocolError::DiskFull { needed, available });
        }
        Ok(())
    })
    .await
    .map_err(torrent_err)?
}

/// librqbit reports errors as text; a full disk must still be recognizable.
fn run_error(text: Option<String>) -> ProtocolError {
    let text = text.unwrap_or_else(|| "unknown torrent error".to_owned());
    // ENOSPC is 28 on Linux and macOS; ERROR_DISK_FULL is 112 on Windows.
    let code = if cfg!(windows) { 112 } else { 28 };
    if text.contains(&format!("(os error {code})")) {
        ProtocolError::DiskFull {
            needed: 0,
            available: 0,
        }
    } else {
        torrent_err(text)
    }
}

/// Decides when a torrent without peers should search again.
struct Stall {
    wait: Duration,
    /// Since when no peer has been connected.
    since: Option<Instant>,
}

impl Stall {
    const fn new(wait: Duration) -> Self {
        Self { wait, since: None }
    }

    /// Feeds the current situation; `true` when it is time to search again.
    fn is_over(&mut self, connected: bool, now: Instant) -> bool {
        if connected {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if now.duration_since(since) < self.wait {
            return false;
        }
        self.since = Some(now);
        self.wait = (self.wait * 2).min(MAX_STALL);
        true
    }
}

/// `part` of `whole` in percent, rounded down.
fn percent(part: u64, whole: u64) -> u8 {
    let p = u128::from(part) * 100 / u128::from(whole.max(1));
    u8::try_from(p.min(100)).unwrap_or(100)
}

/// What a `.torrent` file contains, read without any network access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentInfo {
    /// Torrent name (the folder name for multi-file torrents).
    pub name: String,
    /// Info hash, lower-case hex.
    pub info_hash: String,
    /// Files, all marked selected. Paths are validated like a real download's.
    pub files: Vec<FileEntry>,
}

/// Largest `.torrent` file accepted (large ones list many pieces; 16 MiB is generous).
pub const MAX_TORRENT_FILE_SIZE: usize = 16 * 1024 * 1024;

/// Reads a `.torrent` file's name and file list, e.g. to choose files before adding it.
///
/// # Errors
/// [`ProtocolError::InvalidSource`] if the bytes are too large, not a valid torrent,
/// or contain an unsafe path.
pub fn inspect_torrent(bytes: &[u8]) -> Result<TorrentInfo, ProtocolError> {
    let invalid = |e: &dyn std::fmt::Display| {
        ProtocolError::InvalidSource(format!("not a valid .torrent file: {e}"))
    };
    if bytes.len() > MAX_TORRENT_FILE_SIZE {
        return Err(ProtocolError::InvalidSource(
            "the .torrent file is too large".to_owned(),
        ));
    }
    bencode::check(bytes).map_err(|e| invalid(&e))?;
    let meta = librqbit::torrent_from_bytes(bytes).map_err(|e| invalid(&e))?;
    let info_hash = meta.info_hash.as_string();
    let info = meta.info.data.validate().map_err(|e| invalid(&e))?;
    let mut files = Vec::new();
    for (index, details) in info.iter_file_details().enumerate() {
        let components: Vec<String> = details
            .filename
            .iter_components()
            .map(std::borrow::Cow::into_owned)
            .collect();
        files.push(FileEntry {
            index: u32::try_from(index).map_err(|e| invalid(&e))?,
            path: validate_torrent_path(&components)?,
            size: details.len,
            selected: true,
        });
    }
    let name = info
        .name()
        .map_or_else(|| info_hash.clone(), std::borrow::Cow::into_owned);
    validate_torrent_path(&[name.as_str()])?;
    Ok(TorrentInfo {
        name,
        info_hash,
        files,
    })
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
    fn a_full_disk_is_recognized_in_torrent_errors() {
        let code = if cfg!(windows) { 112 } else { 28 };
        let text = format!("error writing: disk says no (os error {code})");
        assert!(matches!(
            run_error(Some(text)),
            ProtocolError::DiskFull { .. }
        ));
        assert!(matches!(
            run_error(Some("boom (os error 5)".into())),
            ProtocolError::Torrent(_)
        ));
        assert!(run_error(None).to_string().contains("unknown"));
    }

    #[test]
    fn stalls_are_noticed_at_growing_intervals() {
        let start = Instant::now();
        let at = |s| start + Duration::from_secs(s);
        let mut stall = Stall::new(Duration::from_secs(60));
        assert!(!stall.is_over(false, at(0)));
        assert!(!stall.is_over(false, at(59)));
        assert!(!stall.is_over(true, at(60)), "a peer connected");
        assert!(!stall.is_over(false, at(61)));
        assert!(stall.is_over(false, at(121)));
        assert!(!stall.is_over(false, at(240)), "now after two minutes");
        assert!(stall.is_over(false, at(241)));
        let mut now = 241;
        for _ in 0..20 {
            now += MAX_STALL.as_secs();
            assert!(stall.is_over(false, at(now)), "never less often than that");
        }
    }

    #[test]
    fn percent_is_bounded() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(1, 3), 33);
        assert_eq!(percent(9, 3), 100);
        assert_eq!(percent(u64::MAX, u64::MAX), 100);
    }

    #[test]
    fn handles_only_torrent_sources() {
        let p = TorrentProtocol::new();
        assert!(p.handles(&Source::magnet("magnet:?xt=urn:btih:abc").unwrap()));
        assert!(p.handles(&Source::TorrentFile { bytes: vec![1] }));
        assert!(!p.handles(&Source::http(url::Url::parse("https://e.x/f").unwrap()).unwrap()));
    }
}
