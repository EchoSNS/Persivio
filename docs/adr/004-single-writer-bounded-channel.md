# ADR-004: Single writer per task with a bounded channel

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
N segment workers produce chunks for disjoint ranges of one file. Letting each worker write
through its own handle risks interleaved seek+write races, especially on Windows where positional
writes move the file cursor. An unbounded queue between a fast network and a slow disk would grow
memory without limit.

## Decision
Each HTTP task has exactly one writer: a dedicated blocking thread that owns the `std::fs::File`
and receives `(offset, Bytes)` messages over a **bounded** channel (capacity sized to ~32 MiB in
flight). It performs positional writes (`write_all_at` on Unix, a `seek_write` loop on Windows)
behind a cross-platform helper.

## Consequences
- No write races; file I/O never blocks the async runtime.
- Backpressure: when the disk is slow, `send().await` suspends workers, so memory stays bounded
  regardless of file size.
- The writer is the natural place to implement flush/sync for checkpoints (ADR-005).
