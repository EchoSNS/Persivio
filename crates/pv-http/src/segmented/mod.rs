//! Multi-connection (segmented) downloads.
//!
//! ```text
//!   worker 0 ─┐  (HTTP/1.1 range requests)        ┌─► checkpointer (every 1 s):
//!   worker 1 ─┼─► bounded channel ─► writer thread │     writer.sync()  = flush + sync_data
//!   worker N ─┘     (backpressure)     (pwrite)   ─┘     then CheckpointSink (ADR-005)
//!        ▲
//!        └── SegmentTable (claim / reserve / split_largest) — work stealing (ADR-013)
//! ```

mod plan;
mod retry;
mod writer;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pv_core::{
    CheckpointSink, ProgressSink, ProtocolError, RateLimiter, Segment, Settings, TaskId,
};
use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, IF_RANGE, RANGE};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use url::Url;

pub use plan::{LiveSegment, Reservation, SegmentTable, plan_segments};
pub use retry::{RetryPolicy, retry_after};
pub use writer::{MAX_MESSAGE, WriterHandle, WriterStats, Written, spawn_writer};

use crate::client::{DownloadRequest, HttpClient, network_error};
use crate::content_range::{ContentRange, parse_content_range};
use crate::probe::ensure_identity;

/// Largest piece of a received chunk that is rate-limited, reserved and counted at once.
const PIECE: usize = 16 * 1024;

/// Tuning for segmented downloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentedConfig {
    /// Parallel connections (1..=32).
    pub connections: u8,
    /// Never plan or split below this many bytes per segment.
    pub min_segment: u64,
    /// Abort a request that receives nothing for this long (then retry).
    pub idle_timeout: Duration,
    /// Backoff policy per segment.
    pub retry: RetryPolicy,
    /// Sync + checkpoint period.
    pub checkpoint_interval: Duration,
    /// Writer queue length in messages of ≤ [`MAX_MESSAGE`] bytes
    /// (512 × 64 KiB = 32 MiB in flight at most).
    pub write_queue: usize,
    /// Test hook: slow every disk write down by this much.
    #[doc(hidden)]
    pub write_delay: Option<Duration>,
}

impl SegmentedConfig {
    /// Defaults from the app settings, with an optional per-task connection override.
    #[must_use]
    pub fn from_settings(settings: &Settings, connections: Option<u8>) -> Self {
        Self {
            connections: connections
                .unwrap_or(settings.default_connections)
                .clamp(1, 32),
            min_segment: settings.min_segment_size,
            idle_timeout: Duration::from_secs(u64::from(settings.idle_timeout_secs)),
            retry: RetryPolicy {
                max_retries: settings.max_retries,
                ..RetryPolicy::default()
            },
            checkpoint_interval: Duration::from_secs(1),
            write_queue: 512,
            write_delay: None,
        }
    }
}

/// A segmented transfer of a file whose size and validator are known.
pub struct Segmented<'a> {
    /// Shared HTTP client.
    pub client: &'a HttpClient,
    /// Request context.
    pub request: &'a DownloadRequest,
    /// URL to fetch (the probe's final URL).
    pub url: &'a Url,
    /// The `.part` file.
    pub part_path: &'a Path,
    /// Exact size.
    pub total: u64,
    /// `If-Range` value: guarantees every segment comes from the same version.
    pub validator: &'a str,
    /// Persisted segments to resume; empty = plan a fresh download.
    pub segments: Vec<Segment>,
    /// Pause/cancel.
    pub cancel: &'a CancellationToken,
    /// Live progress.
    pub progress: &'a ProgressSink,
    /// Rate limiter shared by all workers.
    pub limiter: Arc<dyn RateLimiter>,
    /// Where checkpoints go.
    pub checkpoint: Option<(TaskId, Arc<dyn CheckpointSink>)>,
    /// Optional shared writer statistics (diagnostics, bounded-memory tests).
    pub writer_stats: Option<Arc<WriterStats>>,
}

/// How a segmented transfer ended (errors are `Err`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentedOutcome {
    /// Every byte written and synced.
    Completed,
    /// Cancelled; progress is checkpointed.
    Cancelled,
    /// The server's file changed (a range answered with `200`), or the `.part` file
    /// does not match the persisted plan: the caller should start over.
    RestartNeeded,
}

/// Everything a worker task shares (workers are `'static` tokio tasks).
struct WorkerCtx {
    client: HttpClient,
    request: DownloadRequest,
    url: Url,
    validator: String,
    table: Arc<SegmentTable>,
    writer: WriterHandle,
    progress: ProgressSink,
    limiter: Arc<dyn RateLimiter>,
    cancel: CancellationToken,
    cfg: SegmentedConfig,
}

enum WorkerExit {
    /// No more work to claim or steal.
    Done,
    Cancelled,
    Changed,
}

enum FetchError {
    Retry {
        err: ProtocolError,
        after: Option<Duration>,
        progressed: bool,
    },
    Fatal(ProtocolError),
    Changed,
    Cancelled,
}

/// Fails early and clearly if `needed` more bytes cannot fit in `dir`. Blocking.
pub(crate) fn ensure_space(dir: &Path, needed: u64) -> Result<(), ProtocolError> {
    let available = fs4::available_space(dir)?;
    if available < needed {
        return Err(ProtocolError::DiskFull { needed, available });
    }
    Ok(())
}

/// Opens (fresh: creates and preallocates) the `.part` file on the blocking pool.
async fn open_part(
    path: PathBuf,
    total: u64,
    fresh: bool,
) -> Result<Option<std::fs::File>, ProtocolError> {
    tokio::task::spawn_blocking(move || -> Result<Option<std::fs::File>, ProtocolError> {
        if fresh {
            let dir = path.parent().unwrap_or_else(|| Path::new("."));
            std::fs::create_dir_all(dir)?;
            ensure_space(dir, total)?;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?;
            // Preallocate: later positional writes never extend the file, and a full
            // disk shows up now rather than at 99 %.
            file.set_len(total)?;
            Ok(Some(file))
        } else {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
            {
                Ok(file) if file.metadata()?.len() == total => Ok(Some(file)),
                Ok(_) => Ok(None), // wrong size: not the file we planned
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            }
        }
    })
    .await
    .map_err(|e| ProtocolError::Network(format!("blocking task failed: {e}")))?
}

/// Runs a segmented download to completion, cancellation or error.
///
/// # Errors
/// A worker's non-retryable error (or retries exhausted), I/O and checkpoint errors.
/// Progress is checkpointed before returning in every case.
pub async fn run_segmented(
    t: Segmented<'_>,
    cfg: &SegmentedConfig,
) -> Result<SegmentedOutcome, ProtocolError> {
    let fresh = t.segments.is_empty();
    let segments = if fresh {
        plan_segments(t.total, cfg.connections, cfg.min_segment)
    } else {
        t.segments.clone()
    };
    let Some(file) = open_part(t.part_path.to_path_buf(), t.total, fresh).await? else {
        return Ok(SegmentedOutcome::RestartNeeded);
    };
    if fresh && let Some((id, sink)) = &t.checkpoint {
        sink.save_segments(*id, &segments).await?;
    }

    let initial: Written = segments
        .iter()
        .map(|s| (s.index, s.start + s.downloaded.min(s.len())))
        .collect();
    let stats = t.writer_stats.clone().unwrap_or_default();
    let writer = spawn_writer(file, initial, cfg.write_queue, cfg.write_delay, stats)?;
    let table = Arc::new(SegmentTable::new(&segments));
    t.progress.set_total(Some(t.total));
    t.progress.set_downloaded(table.reserved_bytes());

    let worker_cancel = t.cancel.child_token();
    let ctx = Arc::new(WorkerCtx {
        client: t.client.clone(),
        request: t.request.clone(),
        url: t.url.clone(),
        validator: t.validator.to_owned(),
        table: table.clone(),
        writer: writer.clone(),
        progress: t.progress.clone(),
        limiter: t.limiter.clone(),
        cancel: worker_cancel.clone(),
        cfg: cfg.clone(),
    });

    let mut workers = JoinSet::new();
    for _ in 0..cfg.connections {
        if let Some(index) = table
            .claim_idle()
            .or_else(|| table.split_largest(cfg.min_segment))
        {
            workers.spawn(worker(ctx.clone(), index));
        }
    }
    drop(ctx); // workers hold the remaining references (and thus the writer)

    let sink = t.checkpoint.as_ref().map(|(id, s)| (*id, s.as_ref()));
    let mut persisted_generation = 0;
    let mut tick = tokio::time::interval(cfg.checkpoint_interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await;
    let mut failure: Option<ProtocolError> = None;
    let mut changed = false;

    loop {
        tokio::select! {
            _ = tick.tick() => {
                t.progress.set_connections(u16::try_from(workers.len()).unwrap_or(u16::MAX));
                if let Err(e) = checkpoint(&writer, &table, sink, &mut persisted_generation).await {
                    failure.get_or_insert(e);
                    worker_cancel.cancel();
                }
            }
            joined = workers.join_next() => match joined {
                None => break,
                Some(Ok(Ok(WorkerExit::Done | WorkerExit::Cancelled))) => {}
                Some(Ok(Ok(WorkerExit::Changed))) => {
                    changed = true;
                    worker_cancel.cancel();
                }
                Some(Ok(Err(e))) => {
                    failure.get_or_insert(e);
                    worker_cancel.cancel();
                }
                Some(Err(join)) => {
                    failure.get_or_insert(ProtocolError::Network(format!("worker task failed: {join}")));
                    worker_cancel.cancel();
                }
            },
        }
    }
    t.progress.set_connections(0);

    // Always leave durable, consistent state behind (ADR-005).
    let rows = checkpoint(&writer, &table, sink, &mut persisted_generation).await?;
    drop(writer); // last handle: the writer thread drains and exits

    if let Some(e) = failure {
        return Err(e);
    }
    if changed {
        return Ok(SegmentedOutcome::RestartNeeded);
    }
    let durable: u64 = rows.iter().map(|s| s.downloaded).sum();
    if durable == t.total {
        Ok(SegmentedOutcome::Completed)
    } else if t.cancel.is_cancelled() {
        Ok(SegmentedOutcome::Cancelled)
    } else {
        Err(ProtocolError::Network(format!(
            "download stopped at {durable} of {} bytes",
            t.total
        )))
    }
}

/// Durability barrier, then persistence. Persists the whole table when it changed
/// (a split), otherwise just positions. Returns the persisted rows.
async fn checkpoint(
    writer: &WriterHandle,
    table: &SegmentTable,
    sink: Option<(TaskId, &dyn CheckpointSink)>,
    persisted_generation: &mut u64,
) -> Result<Vec<Segment>, ProtocolError> {
    // 1) write everything queued, 2) sync_data, 3) read what that sync covers.
    let written = writer.sync().await?;
    let (generation, live) = table.snapshot();
    let rows: Vec<Segment> = live
        .iter()
        .map(|s| {
            let len = s.end - s.start + 1;
            let done = written
                .get(&s.index)
                .map_or(0, |abs| abs.saturating_sub(s.start));
            Segment {
                index: s.index,
                start: s.start,
                end: s.end,
                downloaded: done.min(len),
            }
        })
        .collect();
    // 4) only now persist.
    if let Some((id, sink)) = sink {
        if generation == *persisted_generation {
            let progress: Vec<(u32, u64)> = rows.iter().map(|r| (r.index, r.downloaded)).collect();
            sink.save_progress(id, &progress).await?;
        } else {
            sink.save_segments(id, &rows).await?;
            *persisted_generation = generation;
        }
    }
    Ok(rows)
}

async fn worker(ctx: Arc<WorkerCtx>, mut index: u32) -> Result<WorkerExit, ProtocolError> {
    loop {
        let mut failures = 0u32;
        while let Some((pos, end)) = ctx.table.next_range(index) {
            match fetch(&ctx, index, pos, end).await {
                Ok(()) => failures = 0,
                Err(FetchError::Cancelled) => {
                    ctx.table.release(index);
                    return Ok(WorkerExit::Cancelled);
                }
                Err(FetchError::Changed) => return Ok(WorkerExit::Changed),
                Err(FetchError::Fatal(e)) => return Err(e),
                Err(FetchError::Retry {
                    err,
                    after,
                    progressed,
                }) => {
                    if progressed {
                        failures = 0;
                    }
                    if failures >= ctx.cfg.retry.max_retries {
                        return Err(err);
                    }
                    let delay = ctx.cfg.retry.delay(failures, after, u64::from(index));
                    failures += 1;
                    tracing::debug!(index, failures, ?delay, %err, "segment retry");
                    tokio::select! {
                        () = tokio::time::sleep(delay) => {}
                        () = ctx.cancel.cancelled() => {
                            ctx.table.release(index);
                            return Ok(WorkerExit::Cancelled);
                        }
                    }
                }
            }
        }
        // This segment is done: take an unowned one, or steal half of the biggest.
        match ctx
            .table
            .claim_idle()
            .or_else(|| ctx.table.split_largest(ctx.cfg.min_segment))
        {
            Some(next) => index = next,
            None => return Ok(WorkerExit::Done),
        }
    }
}

fn retryable(err: ProtocolError, after: Option<Duration>, progressed: bool) -> FetchError {
    if err.is_retryable() {
        FetchError::Retry {
            err,
            after,
            progressed,
        }
    } else {
        FetchError::Fatal(err)
    }
}

/// One range request for `pos..=end`, streamed into the writer.
async fn fetch(ctx: &WorkerCtx, index: u32, pos: u64, end: u64) -> Result<(), FetchError> {
    let extra = [
        (RANGE, format!("bytes={pos}-{end}")),
        (IF_RANGE, ctx.validator.clone()),
    ];
    let request = ctx.client.send(&ctx.request, &ctx.url, &extra);
    let mut response = tokio::select! {
        biased;
        () = ctx.cancel.cancelled() => return Err(FetchError::Cancelled),
        sent = tokio::time::timeout(ctx.cfg.idle_timeout, request) => match sent {
            Err(_) => return Err(retryable(ProtocolError::Network("timed out waiting for response".into()), None, false)),
            Ok(Err(e)) => return Err(retryable(e, None, false)),
            Ok(Ok(r)) => r,
        },
    };

    match response.status() {
        StatusCode::PARTIAL_CONTENT => {}
        // If-Range did not match: the server sends the whole (new) file.
        StatusCode::OK | StatusCode::RANGE_NOT_SATISFIABLE => return Err(FetchError::Changed),
        StatusCode::FORBIDDEN => return Err(FetchError::Fatal(ProtocolError::LinkExpired)),
        status => {
            let after = retry_after(response.headers());
            return Err(retryable(
                ProtocolError::HttpStatus {
                    status: status.as_u16(),
                },
                after,
                false,
            ));
        }
    }
    ensure_identity(response.headers()).map_err(FetchError::Fatal)?;
    let content_range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .map(parse_content_range);
    match content_range {
        Some(Ok(ContentRange::Satisfied { start, .. })) if start == pos => {}
        other => {
            return Err(FetchError::Fatal(ProtocolError::Network(format!(
                "asked for bytes {pos}-{end}, server answered {other:?}"
            ))));
        }
    }

    let mut progressed = false;
    loop {
        let chunk = tokio::select! {
            biased;
            () = ctx.cancel.cancelled() => return Err(FetchError::Cancelled),
            next = tokio::time::timeout(ctx.cfg.idle_timeout, response.chunk()) => match next {
                Err(_) => return Err(retryable(ProtocolError::Network("no data received (idle timeout)".into()), None, progressed)),
                Ok(Err(e)) => return Err(retryable(network_error(e), None, progressed)),
                Ok(Ok(None)) if progressed => return Ok(()), // caller re-requests the rest, if any
                Ok(Ok(None)) => return Err(retryable(ProtocolError::Network("empty response body".into()), None, false)),
                Ok(Ok(Some(chunk))) => chunk,
            },
        };
        // Pay for, reserve and count the chunk in small pieces: the limiter is fairer
        // between connections and progress moves smoothly instead of in 64 KiB jumps.
        let mut chunk = chunk;
        while !chunk.is_empty() {
            let piece = chunk.split_to(chunk.len().min(PIECE));
            let len = u64::try_from(piece.len()).unwrap_or(u64::MAX);
            ctx.limiter.acquire(len).await;
            let r = ctx.table.reserve(index, len);
            if r.take > 0 {
                let keep = piece.slice(..usize::try_from(r.take).unwrap_or(piece.len()));
                ctx.writer
                    .write(index, r.offset, keep)
                    .await
                    .map_err(|e| FetchError::Fatal(e.into()))?;
                ctx.progress.add_downloaded(r.take);
                progressed = true;
            }
            if r.finished {
                // Our (possibly shrunk) range is complete; dropping the response closes
                // the connection even if the server would send more.
                return Ok(());
            }
        }
    }
}
