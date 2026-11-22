//! Single-connection download into a `.part` file, with resume.
//!
//! Multi-connection downloads (Phase 4) reuse the same ideas, but with N workers and a
//! dedicated writer thread (ADR-004). With one connection, `tokio::fs::File` (which
//! runs its blocking I/O on tokio's blocking pool) is simple and sufficient.

use std::io::SeekFrom;
use std::path::Path;
use std::time::Duration;

use pv_core::{
    CheckpointSink, HttpResumeInfo, ProgressSink, ProtocolError, RateLimiter, Segment, TaskId,
};
use reqwest::header::{CONTENT_RANGE, IF_RANGE, RANGE};
use reqwest::{Response, StatusCode};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::client::{DownloadRequest, HttpClient, network_error};
use crate::content_range::{ContentRange, parse_content_range};
use crate::probe::{ensure_identity, resume_info_from};

/// Where checkpoints go. The task's single segment has index 0.
pub struct Checkpointing<'a> {
    /// Task id for the sink.
    pub task: TaskId,
    /// Durable storage (the engine passes the store).
    pub sink: &'a dyn CheckpointSink,
    /// How often to sync + checkpoint while streaming.
    pub interval: Duration,
}

/// Everything one single-connection transfer needs.
pub struct Transfer<'a> {
    /// The HTTP client.
    pub client: &'a HttpClient,
    /// Request context (headers, cookies, referer).
    pub request: &'a DownloadRequest,
    /// URL to fetch (normally the probe's final URL).
    pub url: &'a Url,
    /// The `.part` file.
    pub part_path: &'a Path,
    /// Durable offset to resume from (the last checkpoint); 0 for a fresh start.
    pub resume_from: u64,
    /// `If-Range` value for resuming. Without one, resuming is unsafe → start over.
    pub validator: Option<&'a str>,
    /// Size we expect, if known (used to recognize "already complete" on 416).
    pub expected_total: Option<u64>,
    /// Abort if no bytes arrive for this long.
    pub idle_timeout: Duration,
    /// Pause/cancel signal.
    pub cancel: &'a CancellationToken,
    /// Live progress counters.
    pub progress: &'a ProgressSink,
    /// Rate limiter (global + per task).
    pub limiter: &'a dyn RateLimiter,
    /// Durable checkpoints, when the task has a segment row (size known).
    pub checkpoint: Option<Checkpointing<'a>>,
}

/// How a transfer ended (errors are returned as `Err`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferOutcome {
    /// Every byte is written and synced.
    Completed {
        /// Final size.
        size: u64,
        /// The server's copy changed (or ignored our range), so we started over.
        restarted: bool,
        /// Validators of the response actually downloaded.
        validators: HttpResumeInfo,
    },
    /// Cancelled; data up to `offset` is synced and checkpointed.
    Cancelled {
        /// Durable offset.
        offset: u64,
    },
}

/// What to do with a response.
enum Next {
    /// Stream the body starting at the current offset.
    Stream(Response, Option<u64>),
    /// Nothing more to fetch.
    AlreadyComplete(HttpResumeInfo),
    /// Request again from byte 0.
    Restart,
}

/// Runs the transfer. `initial` is a full-body (`200`) response from the probe that can
/// be reused for a fresh start.
///
/// # Errors
/// Network/HTTP errors and I/O errors. Before returning an error mid-stream, the data
/// received so far is synced and checkpointed, so a retry can resume.
pub async fn run_single(
    t: &Transfer<'_>,
    mut initial: Option<Response>,
) -> Result<TransferOutcome, ProtocolError> {
    if let Some(parent) = t.part_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(t.part_path)
        .await?;

    // Resume only from a checkpoint we can validate and that the file actually backs.
    // Bytes past the checkpoint may not have been synced before a crash (ADR-005), so
    // they are cut off rather than trusted.
    let on_disk = file.metadata().await?.len();
    let mut offset = if t.validator.is_some() && t.resume_from <= on_disk {
        t.resume_from
    } else {
        0
    };
    file.set_len(offset).await?;
    let mut restarted = false;

    let mut attempts = 0;
    let (response, total) = loop {
        attempts += 1;
        let response = match initial.take() {
            Some(r) if offset == 0 => r,
            _ => send(t, offset).await?,
        };
        match classify(response, offset, t.expected_total)? {
            Next::Stream(response, total) => {
                if offset > 0 && response.status() == StatusCode::OK {
                    // Server ignored the range or the file changed (If-Range mismatch).
                    tracing::info!(
                        offset,
                        "remote file changed or ranges ignored; restarting from 0"
                    );
                    offset = 0;
                    file.set_len(0).await?;
                    restarted = true;
                }
                break (response, total);
            }
            Next::AlreadyComplete(validators) => {
                file.sync_data().await?;
                t.progress.set_downloaded(offset);
                return Ok(TransferOutcome::Completed {
                    size: offset,
                    restarted,
                    validators,
                });
            }
            Next::Restart if attempts < 3 => {
                offset = 0;
                file.set_len(0).await?;
                restarted = true;
            }
            Next::Restart => {
                return Err(ProtocolError::Network(
                    "server keeps rejecting the requested range".to_owned(),
                ));
            }
        }
    };

    let validators = resume_info_from(&response);
    if offset == 0
        && let (Some(cp), Some(total @ 1..)) = (&t.checkpoint, total)
    {
        // Fresh plan (or new plan after a restart): one segment covering the file.
        let plan = [Segment {
            index: 0,
            start: 0,
            end: total - 1,
            downloaded: 0,
        }];
        cp.sink.save_segments(cp.task, &plan).await?;
    }
    t.progress.set_total(total);
    t.progress.set_downloaded(offset);

    let streamed = stream_body(t, &mut file, response, offset).await;
    let (offset, cancelled, error) = match streamed {
        Ok((offset, cancelled)) => (offset, cancelled, None),
        Err((offset, e)) => (offset, false, Some(e)),
    };
    // Always leave a durable, consistent state behind (data, sync, then checkpoint).
    checkpoint(t, &mut file, offset).await?;

    if let Some(e) = error {
        return Err(e);
    }
    if cancelled {
        return Ok(TransferOutcome::Cancelled { offset });
    }
    match total {
        Some(total) if offset < total => Err(ProtocolError::Network(format!(
            "connection closed after {offset} of {total} bytes"
        ))),
        Some(total) if offset > total => Err(ProtocolError::Network(format!(
            "server sent {offset} bytes but declared {total}"
        ))),
        _ => Ok(TransferOutcome::Completed {
            size: offset,
            restarted,
            validators,
        }),
    }
}

async fn send(t: &Transfer<'_>, offset: u64) -> Result<Response, ProtocolError> {
    let mut req = t.client.get(t.request, t.url);
    if offset > 0 {
        req = req.header(RANGE, format!("bytes={offset}-"));
        if let Some(v) = t.validator {
            req = req.header(IF_RANGE, v);
        }
    }
    req.send().await.map_err(network_error)
}

fn classify(
    response: Response,
    offset: u64,
    expected_total: Option<u64>,
) -> Result<Next, ProtocolError> {
    ensure_identity(response.headers())?;
    match response.status() {
        StatusCode::OK => {
            let total = response.content_length();
            Ok(Next::Stream(response, total))
        }
        StatusCode::PARTIAL_CONTENT => {
            let raw = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| ProtocolError::Network("206 without Content-Range".to_owned()))?;
            match parse_content_range(raw).map_err(|e| ProtocolError::Network(e.to_string()))? {
                // The server must resume exactly where we asked, or bytes land at the
                // wrong offset.
                ContentRange::Satisfied {
                    start, complete, ..
                } if start == offset => Ok(Next::Stream(response, complete)),
                other => Err(ProtocolError::Network(format!(
                    "asked for bytes from {offset}, server sent {other:?}"
                ))),
            }
        }
        StatusCode::RANGE_NOT_SATISFIABLE => {
            if offset > 0 && expected_total == Some(offset) {
                Ok(Next::AlreadyComplete(resume_info_from(&response)))
            } else {
                Ok(Next::Restart)
            }
        }
        StatusCode::FORBIDDEN if offset > 0 => Err(ProtocolError::LinkExpired),
        status => Err(ProtocolError::HttpStatus {
            status: status.as_u16(),
        }),
    }
}

/// Streams the body; returns `(offset, cancelled)` or `(offset, error)`.
async fn stream_body(
    t: &Transfer<'_>,
    file: &mut File,
    mut response: Response,
    mut offset: u64,
) -> Result<(u64, bool), (u64, ProtocolError)> {
    file.seek(SeekFrom::Start(offset))
        .await
        .map_err(|e| (offset, e.into()))?;
    let mut last_checkpoint = Instant::now();
    loop {
        let chunk = tokio::select! {
            biased;
            () = t.cancel.cancelled() => return Ok((offset, true)),
            next = tokio::time::timeout(t.idle_timeout, response.chunk()) => match next {
                Err(_) => {
                    return Err((offset, ProtocolError::Network(format!(
                        "no data received for {}s", t.idle_timeout.as_secs_f32()
                    ))));
                }
                Ok(Err(e)) => return Err((offset, network_error(e))),
                Ok(Ok(None)) => return Ok((offset, false)),
                Ok(Ok(Some(chunk))) => chunk,
            },
        };
        let len = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
        t.limiter.acquire(len).await;
        file.write_all(&chunk)
            .await
            .map_err(|e| (offset, e.into()))?;
        offset += len;
        t.progress.add_downloaded(len);

        if t.checkpoint
            .as_ref()
            .is_some_and(|c| last_checkpoint.elapsed() >= c.interval)
        {
            checkpoint(t, file, offset).await.map_err(|e| (offset, e))?;
            last_checkpoint = Instant::now();
        }
    }
}

/// ADR-005 in one place: flush buffered writes, `sync_data`, and only then persist the
/// offset, so a checkpoint never claims bytes that are not on disk.
async fn checkpoint(t: &Transfer<'_>, file: &mut File, offset: u64) -> Result<(), ProtocolError> {
    file.flush().await?;
    file.sync_data().await?;
    if let Some(cp) = &t.checkpoint {
        cp.sink.save_progress(cp.task, &[(0, offset)]).await?;
    }
    Ok(())
}

/// Moves a finished `.part` file to its final name (ADR-008), retrying briefly because
/// antivirus scanners on Windows often hold new files open for a moment.
///
/// # Errors
/// The last I/O error if every attempt fails.
pub async fn finalize(part: &Path, final_path: &Path) -> Result<(), ProtocolError> {
    let mut delay = Duration::from_millis(50);
    let mut attempt = 0;
    loop {
        match tokio::fs::rename(part, final_path).await {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 5 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                attempt += 1;
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            Err(e) => return Err(e.into()),
        }
    }
}
