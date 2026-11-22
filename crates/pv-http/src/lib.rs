//! HTTP/HTTPS download protocol: probing, filename resolution, single-connection
//! transfers with resume (Phase 3), and later segmented downloads (Phase 4).
//!
//! **Rules:** the client is HTTP/1.1-only and sends `Accept-Encoding: identity`
//! (ADR-003, ADR-011); data is synced before checkpoints are persisted (ADR-005); every
//! server-provided name goes through [`filename::sanitize_filename`] and
//! [`filename::contained_path`].

pub mod client;
pub mod content_range;
pub mod disposition;
pub mod filename;
pub mod probe;
pub mod protocol;
pub mod transfer;

pub use client::{ClientConfig, DownloadRequest, HttpClient, MAX_REDIRECTS};
pub use content_range::{ContentRange, ContentRangeError, parse_content_range};
pub use probe::{Probe, probe};
pub use protocol::HttpProtocol;
pub use transfer::{Checkpointing, Transfer, TransferOutcome, finalize, run_single};
