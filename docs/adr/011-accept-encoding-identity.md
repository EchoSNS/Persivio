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
