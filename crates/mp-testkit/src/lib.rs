//! Deterministic test infrastructure: in-process HTTP servers (ranges, throttling,
//! failures, redirects, ...) and fixture helpers with known SHA-256 hashes.
//!
//! **Boundary:** used only as a `dev-dependency`; never linked into shipped binaries.
//!
//! ```no_run
//! # async fn demo() -> std::io::Result<()> {
//! use mp_testkit::{Fixture, TestServer};
//! let fixture = Fixture::random("data.bin", 1024 * 1024, 42);
//! let server = TestServer::start([fixture.clone()]).await?;
//! let url = server.url("/throttle/data.bin?bps=1000000");
//! // ... download `url`, then compare with fixture.sha256 ...
//! server.shutdown().await;
//! # Ok(()) }
//! ```
//!
//! Routes (every `{name}` refers to a registered [`Fixture`]):
//!
//! | Route | Behavior |
//! |---|---|
//! | `/file/{name}` | Full `Range` support (single range), strong `ETag`, `Last-Modified`, `If-Range` |
//! | `/norange/{name}` | Ignores `Range`, always `200` |
//! | `/throttle/{name}?bps=N` | Like `/file`, but each response is paced to `N` bytes/s |
//! | `/flaky/{name}?fail_rate=F&drop_after=N&drop_rate=D&seed=S` | `503` with probability `F`; otherwise like `/file`, cut after `N` body bytes with probability `D` (default 1.0). Seeded RNG. |
//! | `/hang/{name}?after=N` | Sends `N` body bytes, then stalls without closing |
//! | `/redirect/{n}/{name}` | `302` chain of `n` hops ending at `/file/{name}` |
//! | `/disposition/{name}?cd=...` | Like `/file`, with `Content-Disposition: <cd>` |
//! | `/mutable/{name}` | Like `/file`; content and `ETag` change on `POST /control/mutate/{name}` |
//! | `/gzip/{name}` | gzip-compresses the whole body unless the request sends `Accept-Encoding: identity` |

mod body;
mod fixture;
pub mod range;
mod rng;
mod server;

pub use fixture::{Fixture, sha256_file, sha256_hex};
pub use rng::SplitMix64;
pub use server::{RecordedRequest, TestServer};
