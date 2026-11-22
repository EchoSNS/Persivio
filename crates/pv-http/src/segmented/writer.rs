//! The single positional writer (ADR-004) and its durability barrier (ADR-005).

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

/// Largest payload of one message. Workers split bigger chunks so that
/// `channel capacity × MAX_MESSAGE` is a hard bound on bytes in flight.
pub const MAX_MESSAGE: usize = 64 * 1024;

/// Per-segment durable progress: segment index → absolute end offset (exclusive) of the
/// contiguous bytes written for that segment.
pub type Written = HashMap<u32, u64>;

enum Msg {
    Write {
        segment: u32,
        offset: u64,
        data: Bytes,
    },
    /// Durability barrier: everything sent before this message is written, then
    /// `sync_data` runs, then the reply carries the progress covered by that sync.
    Sync(oneshot::Sender<io::Result<Written>>),
}

/// Live statistics (used by the bounded-memory test and diagnostics).
#[derive(Debug, Default)]
pub struct WriterStats {
    /// Bytes queued but not yet written.
    pub in_flight: AtomicU64,
    /// Peak of `in_flight`.
    pub max_in_flight: AtomicU64,
}

/// Cloneable handle that workers use to queue writes.
#[derive(Clone)]
pub struct WriterHandle {
    tx: mpsc::Sender<Msg>,
    stats: Arc<WriterStats>,
}

impl WriterHandle {
    /// Queues `data` for `offset`. Waits (backpressure) when the queue is full, which
    /// is what keeps memory bounded when the disk is slower than the network.
    ///
    /// # Errors
    /// If the writer thread has stopped.
    pub async fn write(&self, segment: u32, offset: u64, mut data: Bytes) -> io::Result<()> {
        let mut offset = offset;
        while !data.is_empty() {
            let piece = data.split_to(data.len().min(MAX_MESSAGE));
            let len = u64::try_from(piece.len()).unwrap_or(u64::MAX);
            let now = self.stats.in_flight.fetch_add(len, Ordering::Relaxed) + len;
            self.stats.max_in_flight.fetch_max(now, Ordering::Relaxed);
            self.tx
                .send(Msg::Write {
                    segment,
                    offset,
                    data: piece,
                })
                .await
                .map_err(|_| writer_gone())?;
            offset += len;
        }
        Ok(())
    }

    /// Flushes everything queued so far to disk (`sync_data`) and returns the durable
    /// per-segment progress. Only these numbers may be checkpointed.
    ///
    /// # Errors
    /// The `sync_data` error, an earlier failed write, or a stopped writer thread.
    pub async fn sync(&self) -> io::Result<Written> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Msg::Sync(reply))
            .await
            .map_err(|_| writer_gone())?;
        rx.await.map_err(|_| writer_gone())?
    }

    /// Writer statistics.
    #[must_use]
    pub fn stats(&self) -> &WriterStats {
        &self.stats
    }
}

fn writer_gone() -> io::Error {
    io::Error::other("writer thread stopped")
}

/// Starts the writer thread. `capacity` messages of at most [`MAX_MESSAGE`] bytes may be
/// queued. `delay` slows every write down (tests only: simulates a slow disk).
///
/// # Errors
/// If the OS refuses to create the thread.
pub fn spawn_writer(
    file: File,
    initial: Written,
    capacity: usize,
    delay: Option<Duration>,
    stats: Arc<WriterStats>,
) -> io::Result<WriterHandle> {
    let (tx, rx) = mpsc::channel(capacity.max(1));
    let thread_stats = stats.clone();
    std::thread::Builder::new()
        .name("persivio-writer".to_owned())
        .spawn(move || writer_loop(&file, rx, initial, &thread_stats, delay))?;
    Ok(WriterHandle { tx, stats })
}

/// Runs on a dedicated OS thread: blocking file I/O never stalls the async runtime.
/// The loop ends when every [`WriterHandle`] is dropped (normal end, cancel, or crash
/// simulation), after draining queued writes.
fn writer_loop(
    file: &File,
    mut rx: mpsc::Receiver<Msg>,
    mut written: Written,
    stats: &WriterStats,
    delay: Option<Duration>,
) {
    let mut failure: Option<io::ErrorKind> = None;
    while let Some(msg) = rx.blocking_recv() {
        match msg {
            Msg::Write {
                segment,
                offset,
                data,
            } => {
                let len = u64::try_from(data.len()).unwrap_or(u64::MAX);
                if failure.is_none() {
                    if let Some(d) = delay {
                        std::thread::sleep(d);
                    }
                    match write_all_at(file, &data, offset) {
                        Ok(()) => {
                            let end = offset + len;
                            let entry = written.entry(segment).or_insert(0);
                            *entry = (*entry).max(end);
                        }
                        Err(e) => failure = Some(e.kind()),
                    }
                }
                stats.in_flight.fetch_sub(len, Ordering::Relaxed);
            }
            Msg::Sync(reply) => {
                let result = match failure {
                    Some(kind) => Err(io::Error::new(kind, "an earlier write failed")),
                    None => file.sync_data().map(|()| written.clone()),
                };
                let _ = reply.send(result);
            }
        }
    }
}

/// Writes the whole buffer at `offset` without touching a shared cursor.
#[cfg(unix)]
fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

/// Windows has no `write_all_at`; `seek_write` may write less than asked, so loop.
#[cfg(windows)]
fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_write(buf, offset) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                buf = &buf[n..];
                offset += u64::try_from(n).unwrap_or(u64::MAX);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_out_of_order_and_reports_durable_progress() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.set_len(10).unwrap();
        let w = spawn_writer(file, Written::from([(1, 5)]), 4, None, Arc::default()).unwrap();
        w.write(1, 5, Bytes::from_static(b"FGHIJ")).await.unwrap();
        w.write(0, 0, Bytes::from_static(b"AB")).await.unwrap();
        w.write(0, 2, Bytes::from_static(b"CDE")).await.unwrap();
        let durable = w.sync().await.unwrap();
        assert_eq!(durable, Written::from([(0, 5), (1, 10)]));
        assert_eq!(std::fs::read(&path).unwrap(), b"ABCDEFGHIJ");
        assert_eq!(w.stats().in_flight.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn large_chunks_are_split_and_memory_stays_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let file = std::fs::File::create(dir.path().join("f")).unwrap();
        let w = spawn_writer(
            file,
            Written::new(),
            2,
            Some(Duration::from_millis(2)),
            Arc::default(),
        )
        .unwrap();
        let big = Bytes::from(vec![7u8; 20 * MAX_MESSAGE]);
        w.write(0, 0, big).await.unwrap();
        assert_eq!(w.sync().await.unwrap()[&0], 20 * MAX_MESSAGE as u64);
        // Capacity 2 (+1 being written +1 being sent) messages of 64 KiB at most.
        let peak = w.stats().max_in_flight.load(Ordering::Relaxed);
        assert!(peak <= 4 * MAX_MESSAGE as u64, "peak {peak}");
    }

    #[tokio::test]
    async fn write_errors_surface_at_the_next_sync() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ro");
        std::fs::write(&path, b"x").unwrap();
        // A read-only handle: every write fails.
        let file = std::fs::File::open(&path).unwrap();
        let w = spawn_writer(file, Written::new(), 4, None, Arc::default()).unwrap();
        w.write(0, 0, Bytes::from_static(b"data")).await.unwrap();
        assert!(w.sync().await.is_err());
    }
}
