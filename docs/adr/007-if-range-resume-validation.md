# ADR-007: Use If-Range to validate resumes

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
When resuming, the remote file may have changed since the partial download began. Checking with a
separate HEAD and then a GET is racy (the file can change in between) and costs a round trip.

## Decision
Resume requests send `Range` together with `If-Range` carrying the stored **strong** ETag (or
`Last-Modified` when no strong ETag exists). A `206` means the server confirms the file is
unchanged; a `200` means it changed, so we restart from zero reusing that same response stream.

## Consequences
- An atomic, server-side check with no extra round trip.
- Weak ETags (`W/"..."`) are not allowed in `If-Range`; without a strong validator we fall back to
  `Last-Modified`, or restart if neither exists.
- The Phase 3 mutable-file test enforces restart-on-change.
