//! Power-of-two size-class free list for small per-request upload buffers
//! (#177).
//!
//! Every glyph run, ImageText request and trapezoid/triangle request used
//! to create and destroy its own 1–10 KiB host-visible buffer — 85–96% of
//! all `vkAllocateMemory`/`vkFreeMemory` calls, and on RADV every free walks
//! libdrm's process-wide VA hole list (`amdgpu_vamgr_free_va`). This pool
//! keeps retired buffers for reuse, rounded up to a power-of-two class so
//! requests of varying size share entries.
//!
//! The pool itself only does the bookkeeping. **It never decides when a
//! buffer is safe to reuse**: the engine calls [`SizeClassPool::put`] only
//! from the frame-retire walk in `RenderEngine::poll_retired`, after the
//! frame's fence has signalled — the same lifetime model as the `put_image`
//! `StagingPool`. A buffer handed out by [`SizeClassPool::take`] is
//! therefore never in flight.
//!
//! Bounds: each cached buffer is still a live VA range, and on amdgpu a live
//! range makes every other free more expensive. So each class keeps at most
//! [`class_cap`] idle entries (≈512 KiB per class, ≈3.3 MiB for the pool),
//! and an entry idle for [`IDLE_EVICT_AFTER`] is destroyed by
//! [`SizeClassPool::trim`]. Requests above [`MAX_CLASS_BYTES`] do not use
//! the pool.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// log2 of the smallest class. 4 KiB is the allocation granularity of every
/// driver we run on, so a smaller class would not save memory.
const MIN_CLASS_SHIFT: u32 = 12;
/// log2 of the largest class (256 KiB ≈ 8k glyph instances or ~5k
/// trapezoids in one request). Larger requests get a dedicated buffer.
const MAX_CLASS_SHIFT: u32 = 18;
/// Number of size classes: 4, 8, 16, 32, 64, 128 and 256 KiB.
pub(crate) const CLASS_COUNT: usize = (MAX_CLASS_SHIFT - MIN_CLASS_SHIFT + 1) as usize;
/// Largest request the pool serves, in bytes.
pub(crate) const MAX_CLASS_BYTES: u64 = 1 << MAX_CLASS_SHIFT;
/// Idle bytes one class may keep. Sets the per-class entry cap together
/// with [`CLASS_MAX_ENTRIES`] and [`CLASS_MIN_ENTRIES`].
const CLASS_BYTE_BUDGET: u64 = 512 * 1024;
/// Upper entry cap for the small classes. Covers the in-flight working set
/// of a busy client (~700 requests/s × a few frames in flight).
const CLASS_MAX_ENTRIES: usize = 64;
/// Lower entry cap for the large classes, so a repeating large request
/// still reuses its buffer.
const CLASS_MIN_ENTRIES: usize = 2;
/// An idle entry older than this is destroyed by [`SizeClassPool::trim`].
pub(crate) const IDLE_EVICT_AFTER: Duration = Duration::from_secs(2);

/// The class serving a request of `size` bytes, or `None` when the request
/// is larger than [`MAX_CLASS_BYTES`] and needs a dedicated buffer. A
/// zero-byte request maps to the smallest class.
pub(crate) fn class_for(size: u64) -> Option<usize> {
    if size > MAX_CLASS_BYTES {
        return None;
    }
    let shift = size.max(1).next_power_of_two().trailing_zeros();
    Some(shift.saturating_sub(MIN_CLASS_SHIFT) as usize)
}

/// Buffer size, in bytes, of every entry in `class`.
pub(crate) fn class_bytes(class: usize) -> u64 {
    debug_assert!(class < CLASS_COUNT);
    1 << (MIN_CLASS_SHIFT + class as u32)
}

/// Maximum idle entries kept in `class`.
pub(crate) fn class_cap(class: usize) -> usize {
    let by_budget = usize::try_from(CLASS_BYTE_BUDGET / class_bytes(class)).unwrap_or(usize::MAX);
    by_budget.clamp(CLASS_MIN_ENTRIES, CLASS_MAX_ENTRIES)
}

/// Lifetime counters, logged at shutdown.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SizeClassPoolStats {
    /// `take` calls served from an idle entry.
    pub(crate) hits: u64,
    /// `take` calls that found the class empty (the caller allocates).
    pub(crate) misses: u64,
    /// Requests above [`MAX_CLASS_BYTES`] (dedicated buffers).
    pub(crate) oversize: u64,
    /// Entries accepted by `put`.
    pub(crate) returned: u64,
    /// Entries `put` destroyed because the class was at its cap.
    pub(crate) rejected: u64,
    /// Entries `trim` destroyed for being idle too long.
    pub(crate) evicted: u64,
}

/// Idle entries per size class, most recently returned at the back.
pub(crate) struct SizeClassPool<T> {
    /// Per class: `(entry, returned_at)`, oldest at the front. `take` pops
    /// the back (warmest entry); `trim` pops the front.
    idle: [VecDeque<(T, Instant)>; CLASS_COUNT],
    stats: SizeClassPoolStats,
}

impl<T> Default for SizeClassPool<T> {
    fn default() -> Self {
        Self {
            idle: std::array::from_fn(|_| VecDeque::new()),
            stats: SizeClassPoolStats::default(),
        }
    }
}

impl<T> SizeClassPool<T> {
    /// Take the most recently returned idle entry of `class`, or `None` if
    /// the class is empty (the caller allocates a fresh one).
    pub(crate) fn take(&mut self, class: usize) -> Option<T> {
        if let Some((entry, _)) = self.idle[class].pop_back() {
            self.stats.hits += 1;
            Some(entry)
        } else {
            self.stats.misses += 1;
            None
        }
    }

    /// Count a request too large for any class.
    pub(crate) fn note_oversize(&mut self) {
        self.stats.oversize += 1;
    }

    /// Return a retired entry to `class`. The caller guarantees the GPU is
    /// done with it. Over the class cap, the entry is dropped (destroyed).
    pub(crate) fn put(&mut self, class: usize, entry: T, now: Instant) {
        let idle = &mut self.idle[class];
        if idle.len() >= class_cap(class) {
            self.stats.rejected += 1;
            return; // `entry` drops here
        }
        self.stats.returned += 1;
        idle.push_back((entry, now));
    }

    /// Destroy every entry that has sat idle for [`IDLE_EVICT_AFTER`] or
    /// longer. Entries in use keep being returned at the back, so only a
    /// class's unused surplus ages out.
    pub(crate) fn trim(&mut self, now: Instant) {
        for idle in &mut self.idle {
            while idle
                .front()
                .is_some_and(|(_, at)| now.saturating_duration_since(*at) >= IDLE_EVICT_AFTER)
            {
                idle.pop_front();
                self.stats.evicted += 1;
            }
        }
    }

    /// Idle entries currently held in `class`.
    #[cfg(test)]
    pub(crate) fn idle_len(&self, class: usize) -> usize {
        self.idle[class].len()
    }

    /// Idle entries across all classes.
    pub(crate) fn idle_total(&self) -> usize {
        self.idle.iter().map(VecDeque::len).sum()
    }

    /// Idle bytes across all classes.
    pub(crate) fn idle_bytes(&self) -> u64 {
        self.idle
            .iter()
            .enumerate()
            .map(|(class, idle)| class_bytes(class) * idle.len() as u64)
            .sum()
    }

    pub(crate) fn stats(&self) -> SizeClassPoolStats {
        self.stats
    }

    /// Destroy every idle entry. Call when nothing can be in flight.
    pub(crate) fn drain(&mut self) {
        for idle in &mut self.idle {
            idle.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use super::*;

    /// Entry that counts its own destruction.
    struct Probe {
        id: u32,
        drops: Rc<Cell<u32>>,
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    fn probe(id: u32, drops: &Rc<Cell<u32>>) -> Probe {
        Probe {
            id,
            drops: Rc::clone(drops),
        }
    }

    #[test]
    fn class_mapping_rounds_up_to_power_of_two() {
        assert_eq!(class_for(0), Some(0));
        assert_eq!(class_for(1), Some(0));
        assert_eq!(class_for(1024), Some(0));
        assert_eq!(class_for(4096), Some(0));
        assert_eq!(class_for(4097), Some(1));
        assert_eq!(class_for(8192), Some(1));
        assert_eq!(class_for(10 * 1024), Some(2));
        assert_eq!(class_for(128 * 1024 + 1), Some(6));
        assert_eq!(class_for(256 * 1024), Some(6));
        assert_eq!(class_for(256 * 1024 + 1), None);
        assert_eq!(class_for(u64::MAX), None);
        assert_eq!(CLASS_COUNT, 7);
        assert_eq!(class_bytes(0), 4096);
        assert_eq!(class_bytes(CLASS_COUNT - 1), MAX_CLASS_BYTES);
        // Every request fits the buffer of its class.
        for size in [0, 1, 4095, 4096, 4097, 65_536, 200_000, MAX_CLASS_BYTES] {
            let class = class_for(size).expect("in range");
            assert!(class_bytes(class) >= size.max(1));
            assert!(class == 0 || class_bytes(class - 1) < size);
        }
    }

    #[test]
    fn class_caps_are_bounded() {
        assert_eq!(class_cap(0), 64); // 4 KiB: 128 by budget, capped at 64
        assert_eq!(class_cap(1), 64); // 8 KiB: 64 by budget
        assert_eq!(class_cap(2), 32);
        assert_eq!(class_cap(6), 2); // 256 KiB: 2 by budget
        let worst: u64 = (0..CLASS_COUNT)
            .map(|c| class_bytes(c) * class_cap(c) as u64)
            .sum();
        assert!(worst <= 4 * 1024 * 1024, "pool may hoard {worst} bytes");
    }

    #[test]
    fn empty_class_misses_and_returned_entry_is_reused() {
        let drops = Rc::new(Cell::new(0));
        let mut pool = SizeClassPool::default();
        let t0 = Instant::now();
        assert!(pool.take(0).is_none());
        pool.put(0, probe(1, &drops), t0);
        // A different class does not see it.
        assert!(pool.take(1).is_none());
        let got = pool.take(0).expect("reused");
        assert_eq!(got.id, 1);
        assert_eq!(drops.get(), 0, "reuse must not destroy");
        assert!(pool.take(0).is_none(), "an entry is handed out once");
        let s = pool.stats();
        assert_eq!((s.hits, s.misses, s.returned), (1, 3, 1));
    }

    #[test]
    fn take_prefers_the_most_recently_returned_entry() {
        let drops = Rc::new(Cell::new(0));
        let mut pool = SizeClassPool::default();
        let t0 = Instant::now();
        pool.put(0, probe(1, &drops), t0);
        pool.put(0, probe(2, &drops), t0 + Duration::from_millis(10));
        assert_eq!(pool.take(0).expect("hit").id, 2);
        assert_eq!(pool.take(0).expect("hit").id, 1);
    }

    #[test]
    fn put_over_cap_destroys_the_entry() {
        let drops = Rc::new(Cell::new(0));
        let mut pool = SizeClassPool::default();
        let t0 = Instant::now();
        let class = CLASS_COUNT - 1;
        let cap = class_cap(class);
        for id in 0..=cap {
            pool.put(class, probe(u32::try_from(id).expect("id"), &drops), t0);
        }
        assert_eq!(pool.idle_len(class), cap);
        assert_eq!(drops.get(), 1, "the entry over the cap is destroyed");
        assert_eq!(pool.stats().rejected, 1);
        assert_eq!(pool.idle_bytes(), class_bytes(class) * cap as u64);
    }

    #[test]
    fn trim_evicts_only_entries_idle_past_the_limit() {
        let drops = Rc::new(Cell::new(0));
        let mut pool = SizeClassPool::default();
        let t0 = Instant::now();
        pool.put(0, probe(1, &drops), t0);
        pool.put(2, probe(2, &drops), t0);
        pool.put(0, probe(3, &drops), t0 + Duration::from_secs(1));

        pool.trim(t0 + IDLE_EVICT_AFTER - Duration::from_millis(1));
        assert_eq!(pool.idle_total(), 3, "nothing is old enough yet");

        pool.trim(t0 + IDLE_EVICT_AFTER);
        assert_eq!(drops.get(), 2, "both entries returned at t0 are evicted");
        assert_eq!(pool.idle_len(0), 1);
        assert_eq!(pool.idle_len(2), 0);
        assert_eq!(pool.stats().evicted, 2);
        assert_eq!(pool.take(0).expect("younger entry survives").id, 3);
    }

    #[test]
    fn drain_destroys_everything() {
        let drops = Rc::new(Cell::new(0));
        let mut pool = SizeClassPool::default();
        let t0 = Instant::now();
        for class in 0..CLASS_COUNT {
            pool.put(class, probe(0, &drops), t0);
        }
        pool.drain();
        assert_eq!(pool.idle_total(), 0);
        assert_eq!(drops.get(), u32::try_from(CLASS_COUNT).expect("count"));
    }
}
