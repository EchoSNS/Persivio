# ADR-013: Dynamic segment splitting (work stealing)

- **Status:** Accepted
- **Date:** 2026-11-23

## Context
A static plan of N equal segments finishes only as fast as its slowest connection. When
one segment is on a slow route, or a resumed download has one big segment left, the
other connections sit idle while the slowest one keeps going.

## Decision
When a worker finishes its segment, it first claims any unowned unfinished segment. If
there is none, it **steals**: it splits the active segment with the most bytes left, but
only if more than `2 × min_segment` remain (default `min_segment` = 2 MiB):

```
victim:  [start ........ pos ======== remaining ======== end]
split:   [start ........ pos ==== mid-1] [mid ===== end]   ← new segment, new index
                         (victim keeps)   (thief takes)
mid = pos + remaining / 2
```

All of this happens in `SegmentTable` under one `std::sync::Mutex`, held for a few
arithmetic operations and never across `.await`.

**The race.** The victim's HTTP request still asks for bytes up to the *old* end, so the
server keeps sending past `mid`. Every received chunk goes through
`SegmentTable::reserve(index, len)`, which, under the same lock, clamps the chunk to
the segment's *current* end and advances its position. Bytes past `mid - 1` are
dropped, and the victim then closes its connection. A byte is therefore reserved by
exactly one worker, which is checked by a property test that interleaves random
reserves and splits.

**Persistence.** A split bumps the table's `generation`. The checkpointer then persists
the whole table (`save_segments`) instead of just positions (`save_progress`). The
positions come from the writer's post-`sync_data` counters (ADR-005), so the persisted
table never claims unwritten bytes, even mid-split.

## Consequences
- Idle connections stay busy until the end. In the test, a resumed download with one
  750 KB segment and 4 connections splits into ≥ 4 segments and finishes well under
  the single-connection time.
- Segment indexes are not contiguous after splits, and segments are not sorted by
  offset. Code must use `index` and `start`, never the position in the vector.
- `min_segment` bounds the fragmentation: segments are never split below 2 × the
  minimum.
- A worker dropping a connection after a split wastes up to one in-flight chunk of
  bandwidth. That is negligible.
