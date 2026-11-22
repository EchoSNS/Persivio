# ADR-011: Accept-Encoding: identity for segmented downloads

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
With a content coding (gzip, br), `Content-Length` and `Content-Range` describe the *encoded*
bytes, and range offsets do not map to offsets in the decoded file. Transparent decompression in
the client would make segment arithmetic and resume meaningless.

## Decision
Probe and segment requests send `Accept-Encoding: identity`, and the HTTP client's automatic
decompression is disabled.

## Consequences
- Byte offsets in requests, responses and the `.part` file all agree.
- A misbehaving server that compresses anyway is detected (`Content-Encoding` present) and handled
  by falling back to a single, non-resumable connection.
- The Phase 3 gzip route test enforces this.

## Amendment (Phase 3, 2026-11-23)
A server that compresses anyway (sends `Content-Encoding` other than `identity`) is now
**rejected** with a clear `InvalidSource` error, instead of using the "single
non-resumable connection" fallback described above. Saving the compressed bytes would
produce a file that differs from what the user expects, and decompressing would make
size and resume handling unreliable. Failing loudly is safer than a silently different
file. The test `gzip_route_serves_identity_and_misconfigured_servers_are_rejected`
covers this, using the testkit route `/gzip-always`.
