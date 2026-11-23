//! [`HttpProtocol`]: the `pv_core::Protocol` implementation for HTTP(S).
//!
//! - Server supports ranges **and** gives a validator → segmented download
//!   (multi-connection, work stealing, resumable per segment).
//! - Otherwise → single connection (resumable only if the server allows it).
//! - If the server's file changes mid-download, start over once with a fresh probe.

use std::time::Duration;

use async_trait::async_trait;
use pv_core::{
    FileEntry, Outcome, Protocol, ProtocolError, ResolvedMetadata, RunContext, RunPhase, Source,
};

use crate::client::{ClientConfig, DownloadRequest, HttpClient};
use crate::filename::{contained_path, part_name, resolve_filename, unique_name};
use crate::probe::probe;
use crate::segmented::{Segmented, SegmentedConfig, SegmentedOutcome, run_segmented};
use crate::transfer::{Checkpointing, Transfer, TransferOutcome, finalize, run_single};
use crate::verify::verify_file;

/// How often a running transfer syncs and checkpoints (ADR-005).
pub const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(1);

/// Downloads `Source::Http` tasks.
#[derive(Debug, Clone, Copy, Default)]
pub struct HttpProtocol;

#[async_trait]
impl Protocol for HttpProtocol {
    fn handles(&self, source: &Source) -> bool {
        matches!(source, Source::Http { .. })
    }

    async fn run(&self, ctx: RunContext) -> Result<Outcome, ProtocolError> {
        let request = DownloadRequest::from_source(&ctx.task.spec.source)?;
        let client = HttpClient::new(&ClientConfig::from_settings(&ctx.settings))?;
        let run = Run {
            ctx: &ctx,
            request: &request,
            client: &client,
        };

        if let Some(outcome) = run.try_resume().await? {
            return Ok(outcome);
        }
        // Fresh start; if the file changes underneath us, one more fresh start.
        for _ in 0..2 {
            if let Some(outcome) = run.fresh().await? {
                return Ok(outcome);
            }
        }
        Err(ProtocolError::Network(
            "the file kept changing on the server during the download".to_owned(),
        ))
    }
}

struct Run<'a> {
    ctx: &'a RunContext,
    request: &'a DownloadRequest,
    client: &'a HttpClient,
}

fn file_entry(name: &str, size: Option<u64>) -> Vec<FileEntry> {
    vec![FileEntry {
        index: 0,
        path: name.to_owned(),
        size: size.unwrap_or(0),
        selected: true,
    }]
}

impl Run<'_> {
    fn config(&self) -> SegmentedConfig {
        SegmentedConfig::from_settings(&self.ctx.settings, self.ctx.task.spec.connections)
    }

    fn idle_timeout(&self) -> Duration {
        Duration::from_secs(u64::from(self.ctx.settings.idle_timeout_secs))
    }

    /// Resumes from persisted state. `Ok(None)` means "start fresh instead".
    async fn try_resume(&self) -> Result<Option<Outcome>, ProtocolError> {
        let ctx = self.ctx;
        let (Some(http), Some(name), false) = (
            &ctx.resume.http,
            ctx.task.spec.file_name.as_deref(),
            ctx.resume.segments.is_empty(),
        ) else {
            return Ok(None);
        };
        if ctx.resume.segments.iter().all(|s| s.downloaded == 0) {
            return Ok(None);
        }
        let validator = http.etag.clone().or_else(|| http.last_modified.clone());
        let url = http
            .final_url
            .clone()
            .unwrap_or_else(|| self.request.url.clone());
        let part = contained_path(&ctx.task.spec.dest_dir, &part_name(name))?;
        ctx.events.phase(RunPhase::Downloading);

        let (Some(validator), Some(total)) = (validator, ctx.task.total_size) else {
            return Ok(None);
        };
        let segmented = Segmented {
            client: self.client,
            request: self.request,
            url: &url,
            part_path: &part,
            total,
            validator: &validator,
            segments: ctx.resume.segments.clone(),
            cancel: &ctx.cancel,
            progress: &ctx.progress,
            limiter: ctx.limiter.clone(),
            checkpoint: Some((ctx.task.id, ctx.checkpoint.clone())),
            writer_stats: None,
        };
        match run_segmented(segmented, &self.config()).await? {
            SegmentedOutcome::Completed => self.finish(name, &part).await.map(Some),
            SegmentedOutcome::Cancelled => Ok(Some(Outcome::Stopped)),
            SegmentedOutcome::RestartNeeded => Ok(None),
        }
    }

    /// Probes and downloads from scratch. `Ok(None)` means the file changed mid-way.
    async fn fresh(&self) -> Result<Option<Outcome>, ProtocolError> {
        let ctx = self.ctx;
        let dest = &ctx.task.spec.dest_dir;
        ctx.events.phase(RunPhase::Probing);
        let probe = probe(self.client, self.request).await?;
        let resolved = resolve_filename(
            ctx.task.spec.file_name.as_deref(),
            probe.content_disposition.as_deref(),
            &probe.final_url,
            probe.content_type.as_deref(),
        );
        let name = unique_name(dest, &resolved).await?;
        ctx.events.metadata(ResolvedMetadata {
            name: Some(name.clone()),
            total_size: probe.total_size,
            files: file_entry(&name, probe.total_size),
            http: Some(probe.resume_info()),
        });
        let part = contained_path(dest, &part_name(&name))?;
        ctx.events.phase(RunPhase::Downloading);

        if let (true, Some(total @ 1..), Some(validator)) =
            (probe.resumable(), probe.total_size, probe.validator())
        {
            let segmented = Segmented {
                client: self.client,
                request: self.request,
                url: &probe.final_url,
                part_path: &part,
                total,
                validator,
                segments: Vec::new(),
                cancel: &ctx.cancel,
                progress: &ctx.progress,
                limiter: ctx.limiter.clone(),
                checkpoint: Some((ctx.task.id, ctx.checkpoint.clone())),
                writer_stats: None,
            };
            return match run_segmented(segmented, &self.config()).await? {
                SegmentedOutcome::Completed => self.finish(&name, &part).await.map(Some),
                SegmentedOutcome::Cancelled => Ok(Some(Outcome::Stopped)),
                SegmentedOutcome::RestartNeeded => Ok(None),
            };
        }

        if !probe.resumable() {
            ctx.events.warning(format!(
                "{name}: the server does not support resuming; an interruption restarts the download"
            ));
        }
        let total = probe.total_size;
        let transfer = Transfer {
            client: self.client,
            request: self.request,
            url: &probe.final_url,
            part_path: &part,
            resume_from: 0,
            validator: None,
            expected_total: total,
            idle_timeout: self.idle_timeout(),
            cancel: &ctx.cancel,
            progress: &ctx.progress,
            limiter: ctx.limiter.as_ref(),
            checkpoint: total.filter(|&t| t > 0).map(|_| Checkpointing {
                task: ctx.task.id,
                sink: ctx.checkpoint.as_ref(),
                interval: CHECKPOINT_INTERVAL,
            }),
        };
        match run_single(&transfer, probe.body).await? {
            TransferOutcome::Cancelled { .. } => Ok(Some(Outcome::Stopped)),
            TransferOutcome::Completed { .. } => self.finish(&name, &part).await.map(Some),
        }
    }

    /// Optional checksum verification, then the atomic rename to a name that does not
    /// exist yet (ADR-008). A mismatching `.part` is deleted, so a retry starts clean
    /// instead of "resuming" a file that is complete but wrong.
    async fn finish(&self, name: &str, part: &std::path::Path) -> Result<Outcome, ProtocolError> {
        if let Some(expected) = &self.ctx.task.spec.checksum {
            self.ctx.events.phase(RunPhase::Verifying);
            if let Err(e) = verify_file(part, expected).await {
                if matches!(e, ProtocolError::ChecksumMismatch { .. }) {
                    let _ = tokio::fs::remove_file(part).await;
                }
                return Err(e);
            }
        }
        let dest = &self.ctx.task.spec.dest_dir;
        let final_name = unique_name(dest, name).await?;
        let final_path = contained_path(dest, &final_name)?;
        finalize(part, &final_path).await?;
        crate::motw::mark(&final_path, &self.request.url).await;
        Ok(Outcome::Completed { path: final_path })
    }
}
