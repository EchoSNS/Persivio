# ADR-003: Force HTTP/1.1 for segment workers

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
Segmented downloading speeds things up because many servers and middleboxes throttle *per TCP
connection*. HTTP/2 multiplexes every request to the same origin over a single TCP connection, so
N "connections" would share one throttled pipe and gain nothing, while adding TCP-level
head-of-line blocking.

## Decision
The `reqwest` client used by segment workers is built with `http1_only()`. Probing uses the same
client so the probe sees the same server behavior as the workers.

## Consequences
- Each segment worker gets its own TCP connection and its own per-connection bandwidth.
- We give up HTTP/2 header compression and multiplexing; for large-body transfers that cost is negligible.
- The Phase 4 throttle test (8 vs 1 connections, ≥ 5× speedup) guards this decision.
