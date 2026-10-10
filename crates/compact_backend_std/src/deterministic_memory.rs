//! Bounded thread-local reuse for released cage extents.
//!
//! The cache is deliberately an extent cache rather than a thread-owned cage
//! region. Extents remain covered by the allocator's live/pending byte count
//! until they are reused or published to the shared free lists. This keeps
//! remote drops independent of the allocating thread and leaves cage-region
//! ownership with `cage.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) const MAX_ACTIVE_LOCAL_CACHE_OWNERS: usize = 16;
pub(crate) const LOCAL_CACHE_CAPACITY: usize = 16;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReleaseExtent {
    pub(crate) start: u32,
    pub(crate) len: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct RecycledExtent {
    pub(crate) data_offset: core::num::NonZeroU32,
    pub(crate) prefix: u32,
    pub(crate) block_len: u32,
}

pub(crate) struct LocalCacheState {
    pub(crate) extents: [ReleaseExtent; LOCAL_CACHE_CAPACITY],
    pub(crate) len: usize,
}

impl LocalCacheState {
    pub(crate) const fn new() -> Self {
        Self {
            extents: [ReleaseExtent { start: 0, len: 0 }; LOCAL_CACHE_CAPACITY],
            len: 0,
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.extents[..self.len]
            .iter()
            .map(|extent| extent.len as usize)
            .sum()
    }

    pub(crate) fn clear(&mut self) {
        self.extents[..self.len].fill(ReleaseExtent::default());
        self.len = 0;
    }

    /// Reuse the most recently released exactly compatible extent.
    ///
    /// The owning thread is the only caller that can access this state.
    /// `commit` initializes the new header before the descriptor is removed
    /// and must not call user code or panic.
    pub(crate) fn take_compatible(
        &mut self,
        cached_bytes: &AtomicUsize,
        wanted_len: u32,
        mut compatible: impl FnMut(ReleaseExtent) -> Option<RecycledExtent>,
        mut commit: impl FnMut(RecycledExtent),
    ) -> Option<RecycledExtent> {
        if self.len > LOCAL_CACHE_CAPACITY {
            return None;
        }
        for index in (0..self.len).rev() {
            let extent = self.extents[index];
            if extent.len != wanted_len {
                continue;
            }
            let Some(recycled) = compatible(extent) else {
                continue;
            };
            if recycled.block_len != extent.len {
                continue;
            }
            commit(recycled);
            let len = self.len;
            self.extents.copy_within(index + 1..len, index);
            self.len = len - 1;
            self.extents[len - 1] = ReleaseExtent::default();
            cached_bytes.fetch_sub(extent.len as usize, Ordering::AcqRel);
            return Some(recycled);
        }
        None
    }

    /// Keep a bounded, size-class-compatible extent locally. Any returned
    /// extents must be synchronously published by the caller.
    pub(crate) fn push(
        &mut self,
        extent: ReleaseExtent,
        cached_bytes: &AtomicUsize,
        budget: usize,
        is_cacheable: impl FnOnce(ReleaseExtent) -> bool,
    ) -> [Option<ReleaseExtent>; 2] {
        if !is_cacheable(extent) {
            return [Some(extent), None];
        }

        if self.len > LOCAL_CACHE_CAPACITY {
            return [Some(extent), None];
        }

        let mut publish = [None, None];
        if self.len == LOCAL_CACHE_CAPACITY {
            let evicted = self.extents[0];
            self.extents.copy_within(1..LOCAL_CACHE_CAPACITY, 0);
            let len = LOCAL_CACHE_CAPACITY - 1;
            self.len = len;
            self.extents[len] = ReleaseExtent::default();
            cached_bytes.fetch_sub(evicted.len as usize, Ordering::AcqRel);
            publish[0] = Some(evicted);
        }

        if reserve_budget(cached_bytes, extent.len as usize, budget) {
            let len = self.len;
            self.extents[len] = extent;
            self.len = len + 1;
        } else if publish[0].is_some() {
            publish[1] = Some(extent);
        } else {
            publish[0] = Some(extent);
        }
        publish
    }
}

fn reserve_budget(cached_bytes: &AtomicUsize, bytes: usize, budget: usize) -> bool {
    let mut current = cached_bytes.load(Ordering::Acquire);
    loop {
        let Some(updated) = current
            .checked_add(bytes)
            .filter(|updated| *updated <= budget)
        else {
            return false;
        };
        match cached_bytes.compare_exchange_weak(
            current,
            updated,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(start: u32, len: u32) -> ReleaseExtent {
        ReleaseExtent { start, len }
    }

    #[test]
    fn local_cache_reuses_only_an_exact_compatible_extent() {
        let mut cache = LocalCacheState::new();
        let bytes = AtomicUsize::new(0);
        let pending = extent(64, 40);
        assert_eq!(cache.push(pending, &bytes, 128, |_| true), [None, None]);
        let recycled = RecycledExtent {
            data_offset: core::num::NonZeroU32::new(88).unwrap(),
            prefix: 0,
            block_len: 40,
        };
        let mut initialized = false;
        let found = cache.take_compatible(
            &bytes,
            40,
            |_| Some(recycled),
            |candidate| initialized = candidate.block_len == 40,
        );
        assert!(initialized);
        assert_eq!(found.map(|item| item.block_len), Some(40));
        assert_eq!(bytes.load(Ordering::Acquire), 0);
        assert_eq!(cache.len, 0);
    }

    #[test]
    fn local_cache_finds_an_exact_match_among_mixed_extent_sizes() {
        let mut cache = LocalCacheState::new();
        let bytes = AtomicUsize::new(0);
        let small = extent(64, 32);
        let wanted = extent(128, 40);
        assert_eq!(cache.push(wanted, &bytes, 128, |_| true), [None, None]);
        assert_eq!(cache.push(small, &bytes, 128, |_| true), [None, None]);

        let recycled = RecycledExtent {
            data_offset: core::num::NonZeroU32::new(wanted.start + 16).unwrap(),
            prefix: 0,
            block_len: wanted.len,
        };
        let found = cache.take_compatible(
            &bytes,
            wanted.len,
            |candidate| (candidate == wanted).then_some(recycled),
            |_| {},
        );

        assert_eq!(found.map(|item| item.block_len), Some(wanted.len));
        assert_eq!(cache.len, 1);
        assert_eq!(cache.extents[0], small);
        assert_eq!(bytes.load(Ordering::Acquire), small.len as usize);
    }

    #[test]
    fn local_cache_budget_and_capacity_return_overflow_without_losing_it() {
        let mut cache = LocalCacheState::new();
        let bytes = AtomicUsize::new(0);
        for index in 0..LOCAL_CACHE_CAPACITY {
            assert_eq!(
                cache.push(extent(64 + index as u32 * 40, 40), &bytes, 640, |_| true),
                [None, None]
            );
        }
        assert_eq!(bytes.load(Ordering::Acquire), 640);

        let pushed = cache.push(extent(704, 40), &bytes, 640, |_| true);
        assert_eq!(pushed[0], Some(extent(64, 40)));
        assert_eq!(pushed[1], None);
        assert_eq!(bytes.load(Ordering::Acquire), 640);

        let budget_limited = cache.push(extent(744, 112), &bytes, 640, |_| true);
        assert_eq!(
            budget_limited,
            [Some(extent(104, 40)), Some(extent(744, 112))]
        );
        assert_eq!(bytes.load(Ordering::Acquire), 600);
        assert_eq!(cache.len, LOCAL_CACHE_CAPACITY - 1);
    }

    #[test]
    fn cache_clear_releases_every_descriptor_for_reuse() {
        let mut cache = LocalCacheState::new();
        let bytes = AtomicUsize::new(0);
        assert_eq!(
            cache.push(extent(64, 40), &bytes, 128, |_| true),
            [None, None]
        );
        assert_eq!(bytes.load(Ordering::Acquire), 40);
        cache.clear();
        bytes.fetch_sub(40, Ordering::AcqRel);
        assert_eq!(cache.len, 0);
        assert_eq!(cache.bytes(), 0);
        assert_eq!(
            cache.push(extent(128, 40), &bytes, 128, |_| true),
            [None, None]
        );
    }
}
