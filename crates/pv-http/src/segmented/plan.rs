//! Splitting a file into byte ranges, and the live segment table workers share.

use std::sync::{Mutex, MutexGuard, PoisonError};

use pv_core::Segment;

/// Splits `[0, total)` into at most `connections` contiguous segments of at least
/// `min_segment` bytes (except when the whole file is smaller). Ends are inclusive.
#[must_use]
pub fn plan_segments(total: u64, connections: u8, min_segment: u64) -> Vec<Segment> {
    if total == 0 {
        return Vec::new();
    }
    let by_size = (total / min_segment.max(1)).max(1);
    let n = u64::from(connections.max(1)).min(by_size);
    let base = total / n;
    let remainder = total % n;
    let mut start = 0;
    (0..n)
        .map(|i| {
            // The first `remainder` segments get one extra byte.
            let len = base + u64::from(i < remainder);
            let seg = Segment {
                index: u32::try_from(i).unwrap_or(u32::MAX),
                start,
                end: start + len - 1,
                downloaded: 0,
            };
            start += len;
            seg
        })
        .collect()
}

/// One row of the live table. `pos` is the next byte to *request*; bytes before it are
/// reserved by the worker (sent to the writer), not necessarily durable yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveSegment {
    /// Stable index (new segments created by splitting get fresh indexes).
    pub index: u32,
    /// First byte.
    pub start: u64,
    /// Last byte, inclusive. Shrinks when the segment is split.
    pub end: u64,
    /// Next byte to fetch.
    pub pos: u64,
    /// A worker currently owns this segment.
    pub active: bool,
}

impl LiveSegment {
    /// Bytes not yet reserved.
    #[must_use]
    pub const fn remaining(&self) -> u64 {
        (self.end + 1).saturating_sub(self.pos)
    }
}

/// Result of reserving a received chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reservation {
    /// File offset where the kept bytes go.
    pub offset: u64,
    /// How many bytes of the chunk to keep (the rest belongs to someone else now).
    pub take: u64,
    /// The segment has no bytes left after this chunk.
    pub finished: bool,
}

/// The shared segment table. A `std::sync::Mutex` is fine here: it is held only for a
/// few arithmetic operations and **never across an `.await`** (see CONTRIBUTING.md).
#[derive(Debug)]
pub struct SegmentTable {
    inner: Mutex<TableInner>,
}

#[derive(Debug)]
struct TableInner {
    segments: Vec<LiveSegment>,
    next_index: u32,
    /// Bumped whenever the set of segments or an `end` changes, so the checkpointer
    /// knows to persist the whole table instead of just positions.
    generation: u64,
}

impl SegmentTable {
    /// Builds the table from persisted segments (positions resume where they stopped).
    #[must_use]
    pub fn new(segments: &[Segment]) -> Self {
        let live = segments
            .iter()
            .map(|s| LiveSegment {
                index: s.index,
                start: s.start,
                end: s.end,
                pos: s.start + s.downloaded.min(s.len()),
                active: false,
            })
            .collect::<Vec<_>>();
        let next_index = live.iter().map(|s| s.index + 1).max().unwrap_or(0);
        Self {
            inner: Mutex::new(TableInner {
                segments: live,
                next_index,
                generation: 0,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, TableInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A copy of the table.
    #[must_use]
    pub fn snapshot(&self) -> (u64, Vec<LiveSegment>) {
        let t = self.lock();
        (t.generation, t.segments.clone())
    }

    /// Claims an unfinished, unowned segment for a worker.
    pub fn claim_idle(&self) -> Option<u32> {
        let mut t = self.lock();
        let seg = t
            .segments
            .iter_mut()
            .find(|s| !s.active && s.remaining() > 0)?;
        seg.active = true;
        Some(seg.index)
    }

    /// The range a worker should request next for `index`: `(pos, end)`, or `None` if
    /// the segment is finished.
    pub fn next_range(&self, index: u32) -> Option<(u64, u64)> {
        let t = self.lock();
        let s = t.segments.iter().find(|s| s.index == index)?;
        (s.remaining() > 0).then_some((s.pos, s.end))
    }

    /// Reserves a received chunk of `len` bytes for segment `index`.
    ///
    /// This is where the dynamic-splitting race is resolved: if another worker took over
    /// the tail of this segment after our request was sent, the server may still send
    /// bytes past our (new) `end`. They are cut off here, under the lock, so no byte is
    /// ever written by two workers.
    pub fn reserve(&self, index: u32, len: u64) -> Reservation {
        let mut t = self.lock();
        let Some(s) = t.segments.iter_mut().find(|s| s.index == index) else {
            return Reservation {
                offset: 0,
                take: 0,
                finished: true,
            };
        };
        let take = len.min(s.remaining());
        let offset = s.pos;
        s.pos += take;
        Reservation {
            offset,
            take,
            finished: s.remaining() == 0,
        }
    }

    /// Marks a worker's segment as no longer owned (on error/cancel).
    pub fn release(&self, index: u32) {
        let mut t = self.lock();
        if let Some(s) = t.segments.iter_mut().find(|s| s.index == index) {
            s.active = false;
        }
    }

    /// Work stealing: splits the active segment with the most bytes left, if more than
    /// `2 × min_segment` remain, and hands the back half to the caller as a new, already
    /// claimed segment. See ADR-013.
    pub fn split_largest(&self, min_segment: u64) -> Option<u32> {
        let mut t = self.lock();
        let threshold = min_segment.saturating_mul(2);
        let victim = t
            .segments
            .iter_mut()
            .filter(|s| s.active && s.remaining() > threshold)
            .max_by_key(|s| s.remaining())?;
        let mid = victim.pos + victim.remaining() / 2;
        let old_end = victim.end;
        victim.end = mid - 1;
        let index = t.next_index;
        t.next_index += 1;
        t.segments.push(LiveSegment {
            index,
            start: mid,
            end: old_end,
            pos: mid,
            active: true,
        });
        t.generation += 1;
        Some(index)
    }

    /// Bytes reserved so far across all segments.
    #[must_use]
    pub fn reserved_bytes(&self) -> u64 {
        self.lock().segments.iter().map(|s| s.pos - s.start).sum()
    }

    /// Whether every byte has been reserved.
    #[must_use]
    pub fn all_reserved(&self) -> bool {
        self.lock().segments.iter().all(|s| s.remaining() == 0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn plan_examples() {
        assert_eq!(plan_segments(0, 8, MIB), Vec::new());
        assert_eq!(
            plan_segments(10, 8, MIB),
            [Segment {
                index: 0,
                start: 0,
                end: 9,
                downloaded: 0
            }]
        );
        let p = plan_segments(10 * MIB + 3, 4, 2 * MIB);
        assert_eq!(p.len(), 4);
        assert_eq!(
            p[0].len(),
            10 * MIB / 4 + 1,
            "remainder spread over the first segments"
        );
        assert_eq!(p[3].end, 10 * MIB + 2);
        // Never more segments than min size allows.
        assert_eq!(plan_segments(5 * MIB, 32, 2 * MIB).len(), 2);
        assert_eq!(
            plan_segments(5 * MIB, 0, 2 * MIB).len(),
            1,
            "0 connections treated as 1"
        );
    }

    proptest! {
        /// Segments are contiguous, non-overlapping, cover [0, total) exactly, respect
        /// the connection limit and (when split) the minimum size.
        #[test]
        fn plan_covers_the_file_exactly(total in 0u64..(1 << 40), n in 0u8..=32, min in 1u64..(64 * MIB)) {
            let plan = plan_segments(total, n, min);
            if total == 0 {
                prop_assert!(plan.is_empty());
                return Ok(());
            }
            prop_assert!(!plan.is_empty());
            prop_assert!(plan.len() <= usize::from(n.max(1)));
            prop_assert_eq!(plan[0].start, 0);
            prop_assert_eq!(plan.last().unwrap().end, total - 1);
            for (i, w) in plan.windows(2).enumerate() {
                prop_assert_eq!(w[1].start, w[0].end + 1, "gap or overlap at {}", i);
            }
            if plan.len() > 1 {
                prop_assert!(plan.iter().all(|s| s.len() >= min));
            }
            let sum: u64 = plan.iter().map(Segment::len).sum();
            prop_assert_eq!(sum, total);
            prop_assert!(plan.iter().enumerate().all(|(i, s)| s.index as usize == i));
        }
    }

    fn table(total: u64, n: u8) -> SegmentTable {
        SegmentTable::new(&plan_segments(total, n, 1))
    }

    #[test]
    fn reserve_clamps_to_the_segment_end() {
        let t = table(100, 2); // [0..=49], [50..=99]
        assert_eq!(t.claim_idle(), Some(0));
        assert_eq!(
            t.reserve(0, 30),
            Reservation {
                offset: 0,
                take: 30,
                finished: false
            }
        );
        assert_eq!(
            t.reserve(0, 30),
            Reservation {
                offset: 30,
                take: 20,
                finished: true
            }
        );
        assert_eq!(t.reserve(0, 5).take, 0);
        assert_eq!(t.next_range(0), None);
        assert_eq!(
            t.reserve(99, 5),
            Reservation {
                offset: 0,
                take: 0,
                finished: true
            }
        );
    }

    #[test]
    fn split_race_never_assigns_a_byte_twice() {
        let t = table(1000, 1);
        assert_eq!(t.claim_idle(), Some(0));
        assert_eq!(t.reserve(0, 100).take, 100);
        // Worker 0 asked for 0..=999 and is streaming. Now another worker steals.
        let new = t.split_largest(10).unwrap();
        assert_eq!(t.next_range(0), Some((100, 549)));
        assert_eq!(t.next_range(new), Some((550, 999)));
        // Worker 0's in-flight response keeps sending bytes beyond 549: they are cut.
        assert_eq!(
            t.reserve(0, 500),
            Reservation {
                offset: 100,
                take: 450,
                finished: true
            }
        );
        assert_eq!(
            t.reserve(new, 450),
            Reservation {
                offset: 550,
                take: 450,
                finished: true
            }
        );
        assert!(t.all_reserved());
        assert_eq!(t.reserved_bytes(), 1000);
        let (generation, segs) = t.snapshot();
        assert_eq!(generation, 1);
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn split_respects_threshold_and_ownership() {
        let t = table(100, 1);
        assert_eq!(
            t.split_largest(10),
            None,
            "inactive segments are claimed, not split"
        );
        assert_eq!(t.claim_idle(), Some(0));
        assert_eq!(t.split_largest(50), None, "100 is not > 2 x 50");
        assert!(t.split_largest(49).is_some());
        assert_eq!(t.claim_idle(), None, "the new half is already claimed");
        t.release(0);
        assert_eq!(t.claim_idle(), Some(0));
    }

    #[test]
    fn resumes_from_persisted_progress() {
        let segs = [
            Segment {
                index: 0,
                start: 0,
                end: 9,
                downloaded: 10,
            },
            Segment {
                index: 3,
                start: 10,
                end: 19,
                downloaded: 4,
            },
        ];
        let t = SegmentTable::new(&segs);
        assert_eq!(t.claim_idle(), Some(3), "finished segments are skipped");
        assert_eq!(t.next_range(3), Some((14, 19)));
        assert_eq!(t.reserved_bytes(), 14);
        t.split_largest(1).unwrap();
        let (_, snap) = t.snapshot();
        assert_eq!(snap.last().unwrap().index, 4, "new indexes never collide");
    }

    proptest! {
        /// Random interleavings of reserve/split: every byte is reserved exactly once.
        #[test]
        fn random_splits_cover_each_byte_once(total in 1u64..5000, ops in prop::collection::vec((0u8..3, 1u64..400), 1..200)) {
            let t = table(total, 1);
            let mut owned: Vec<u32> = vec![t.claim_idle().unwrap()];
            let mut covered = vec![0u8; usize::try_from(total).unwrap()];
            for (op, n) in ops {
                if op == 0 {
                    if let Some(i) = t.split_largest(5) { owned.push(i); }
                } else {
                    let idx = owned[usize::try_from(n).unwrap() % owned.len()];
                    let r = t.reserve(idx, n);
                    for b in r.offset..r.offset + r.take {
                        covered[usize::try_from(b).unwrap()] += 1;
                    }
                }
            }
            // Drain everything.
            for idx in owned {
                let r = t.reserve(idx, u64::MAX);
                for b in r.offset..r.offset + r.take {
                    covered[usize::try_from(b).unwrap()] += 1;
                }
            }
            prop_assert!(covered.iter().all(|&c| c == 1));
        }
    }
}
