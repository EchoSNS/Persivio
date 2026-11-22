//! [`HttpProtocol`]: the `pv_core::Protocol` implementation for HTTP(S).
//!
//! Phase 3 downloads over a single connection; Phase 4 switches to segmented
//! downloads when the server supports ranges.

use std::time::Duration;

use async_trait::async_trait;
use pv_core::{
    FileEntry, Outcome, Protocol, ProtocolError, ResolvedMetadata, RunContext, RunPhase, Source,
};

use crate::client::{ClientConfig, DownloadRequest, HttpClient};
use crate::filename::{contained_path, part_name, resolve_filename, unique_name};
use crate::probe::probe;
use crate::transfer::{Checkpointing, Transfer, TransferOutcome, finalize, run_single};

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
        run(&ctx).await
    }
}

fn file_entry(name: &str, size: Option<u64>) -> Vec<FileEntry> {
    vec![FileEntry {
        index: 0,
        path: name.to_owned(),
        size: size.unwrap_or(0),
        selected: true,
    }]
}

async fn run(ctx: &RunContext) -> Result<Outcome, ProtocolError> {
    let request = DownloadRequest::from_source(&ctx.task.spec.source)?;
    let client = HttpClient::new(&ClientConfig::from_settings(&ctx.settings))?;
    let dest = &ctx.task.spec.dest_dir;
    let spec_name = ctx.task.spec.file_name.as_deref();

    // Resume when we have a name, validators and durable progress from a previous run.
    let segment = ctx.resume.segments.first();
    let resume = match (&ctx.resume.http, spec_name, segment) {
        (Some(http), Some(name), Some(seg)) if seg.downloaded > 0 => Some((http, name, seg)),
        _ => None,
    };

    let (name, url, validator, resume_from, total, initial) = if let Some((http, name, seg)) =
        resume
    {
        let validator = http.etag.clone().or_else(|| http.last_modified.clone());
        let url = http
            .final_url
            .clone()
            .unwrap_or_else(|| request.url.clone());
        (
            name.to_owned(),
            url,
            validator,
            seg.downloaded,
            ctx.task.total_size,
            None,
        )
    } else {
        ctx.events.phase(RunPhase::Probing);
        let probe = probe(&client, &request).await?;
        let resolved = resolve_filename(
            spec_name,
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
        if !probe.resumable() {
            ctx.events.warning(format!(
                "{name}: the server does not support resuming; an interruption restarts the download"
            ));
        }
        let url = probe.final_url.clone();
        (name, url, None, 0, probe.total_size, probe.body)
    };

    ctx.events.phase(RunPhase::Downloading);
    let part = contained_path(dest, &part_name(&name))?;
    let checkpoint = total.filter(|&t| t > 0).map(|_| Checkpointing {
        task: ctx.task.id,
        sink: ctx.checkpoint.as_ref(),
        interval: CHECKPOINT_INTERVAL,
    });
    let transfer = Transfer {
        client: &client,
        request: &request,
        url: &url,
        part_path: &part,
        resume_from,
        validator: validator.as_deref(),
        expected_total: total,
        idle_timeout: Duration::from_secs(u64::from(ctx.settings.idle_timeout_secs)),
        cancel: &ctx.cancel,
        progress: &ctx.progress,
        limiter: ctx.limiter.as_ref(),
        checkpoint,
    };

    match run_single(&transfer, initial).await? {
        TransferOutcome::Cancelled { .. } => Ok(Outcome::Stopped),
        TransferOutcome::Completed {
            size,
            restarted,
            validators,
        } => {
            if restarted {
                ctx.events.metadata(ResolvedMetadata {
                    name: Some(name.clone()),
                    total_size: Some(size),
                    files: file_entry(&name, Some(size)),
                    http: Some(validators),
                });
            }
            // Never overwrite: if a file with this name appeared meanwhile, pick another.
            let final_name = unique_name(dest, &name).await?;
            let final_path = contained_path(dest, &final_name)?;
            finalize(&part, &final_path).await?;
            Ok(Outcome::Completed { path: final_path })
        }
    }
}
