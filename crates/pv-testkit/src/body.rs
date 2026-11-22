//! Streaming response bodies with injectable faults.

use std::io;
use std::time::Duration;

use axum::body::Body;
use bytes::Bytes;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Chunk size for streamed bodies.
const CHUNK: u64 = 16 * 1024;

/// How a body should misbehave.
#[derive(Debug, Clone, Copy, Default)]
pub struct Faults {
    /// Pace output to this many bytes per second.
    pub bps: Option<u64>,
    /// Abort the connection after this many body bytes.
    pub drop_after: Option<u64>,
    /// Stop sending (without closing) after this many body bytes.
    pub hang_after: Option<u64>,
}

struct State {
    data: Bytes,
    sent: u64,
    start: Instant,
    faults: Faults,
    cancel: CancellationToken,
    done: bool,
}

/// Streams `data` according to `faults`. The stream ends with an error (which makes
/// hyper abort the connection mid-body) on a drop, after a hang is released, or when
/// the server shuts down (`cancel`).
pub fn stream(data: Bytes, faults: Faults, cancel: CancellationToken) -> Body {
    let state = State {
        data,
        sent: 0,
        start: Instant::now(),
        faults,
        cancel,
        done: false,
    };
    Body::from_stream(futures::stream::unfold(state, next_chunk))
}

fn aborted(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, msg)
}

async fn next_chunk(mut st: State) -> Option<(Result<Bytes, io::Error>, State)> {
    let len = u64::try_from(st.data.len()).unwrap_or(u64::MAX);
    if st.done || st.sent >= len {
        return None;
    }
    if st.faults.drop_after.is_some_and(|d| st.sent >= d) {
        // Return Pending briefly first: hyper flushes buffered headers and body while the
        // stream is pending. Erroring immediately would abort before anything reached the
        // socket, and the client would see a missing response rather than a truncated body.
        tokio::time::sleep(Duration::from_millis(25)).await;
        st.done = true;
        return Some((Err(aborted("testkit: injected connection drop")), st));
    }
    if st.faults.hang_after.is_some_and(|h| st.sent >= h) {
        // Stall until the server shuts down; the client sees an open, silent connection.
        st.cancel.cancelled().await;
        st.done = true;
        return Some((Err(aborted("testkit: server shut down during hang")), st));
    }

    let mut n = CHUNK.min(len - st.sent);
    for limit in [st.faults.drop_after, st.faults.hang_after]
        .into_iter()
        .flatten()
    {
        n = n.min(limit - st.sent);
    }
    if let Some(bps) = st.faults.bps.filter(|&b| b > 0) {
        // Small chunks (~50 ms worth) keep pacing smooth; sleep until this chunk is "due".
        n = n.min((bps / 20).max(1));
        let due = st.start + nanos_for(st.sent + n, bps);
        tokio::select! {
            () = tokio::time::sleep_until(due) => {}
            () = st.cancel.cancelled() => {
                st.done = true;
                return Some((Err(aborted("testkit: server shut down")), st));
            }
        }
    }

    let from = usize::try_from(st.sent).unwrap_or(usize::MAX);
    let to = usize::try_from(st.sent + n).unwrap_or(usize::MAX);
    let chunk = st.data.slice(from..to);
    st.sent += n;
    Some((Ok(chunk), st))
}

/// Time to send `bytes` at `bps`, using integer math (no float casts).
fn nanos_for(bytes: u64, bps: u64) -> Duration {
    let nanos = u128::from(bytes) * 1_000_000_000 / u128::from(bps);
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacing_math() {
        assert_eq!(nanos_for(1000, 1000), Duration::from_secs(1));
        assert_eq!(nanos_for(1, 2), Duration::from_millis(500));
        assert_eq!(nanos_for(u64::MAX, 1), Duration::from_nanos(u64::MAX));
    }
}
