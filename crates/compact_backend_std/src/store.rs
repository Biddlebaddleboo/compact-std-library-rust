//! Persistent in-process arena ownership.

use core::marker::PhantomData;

use compact_core::{Arena, CompactValue, Error as CoreError, Offset32};

use crate::{StdBackendError, StdBacking};

/// A copyable, destructor-free value that can be stored as a compact root.
///
/// The first store contract is intentionally limited to `Copy` values. Such a
/// root has no destructor that could be skipped when its arena lifetime is
/// erased between accesses. A root may contain application-defined compact
/// offsets represented as ordinary integers, but resolving those offsets is
/// the caller's responsibility and must use the current callback's arena.
pub trait StoreRoot: CompactValue + Copy + 'static {}

impl<T> StoreRoot for T where T: CompactValue + Copy + 'static {}

/// A root handle branded to one [`CompactStore::with`] callback.
pub struct RootHandle<'arena, T: StoreRoot> {
    offset: Offset32<'arena, T>,
    marker: PhantomData<fn(&'arena mut ()) -> &'arena mut ()>,
}

impl<T: StoreRoot> Copy for RootHandle<'_, T> {}

impl<T: StoreRoot> Clone for RootHandle<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'arena, T: StoreRoot> RootHandle<'arena, T> {
    /// Resolve the root as an immutable value through its current arena.
    pub fn get<'view, 'memory>(
        self,
        arena: &'view Arena<'arena, 'memory>,
    ) -> compact_core::Result<&'view T> {
        arena.get(self.offset)
    }

    /// Resolve the root as a mutable value through its current arena.
    pub fn get_mut<'view, 'memory>(
        self,
        arena: &'view mut Arena<'arena, 'memory>,
    ) -> compact_core::Result<&'view mut T> {
        arena.get_mut(self.offset)
    }
}

#[derive(Clone, Copy)]
struct RawRoot {
    offset: u32,
}

/// An owned persistent arena whose root is rebranded on each access.
///
/// The backing allocation never moves while owned by the store. `with` and
/// `with_mut` create a fresh arena lifetime for every callback, so references
/// and handles cannot escape and remain usable after a later attachment.
pub struct CompactStore<T: StoreRoot> {
    backing: StdBacking,
    root: RawRoot,
    // Keep the store single-owner even when T itself happens to be Send/Sync.
    marker: PhantomData<*mut T>,
}

impl<T: StoreRoot> CompactStore<T> {
    /// Create a store and initialize its root inside a persistent arena.
    pub fn build<F>(capacity: usize, initialize: F) -> Result<Self, StdBackendError>
    where
        F: for<'arena, 'memory> FnOnce(
            &mut Arena<'arena, 'memory>,
        ) -> compact_core::Result<Offset32<'arena, T>>,
    {
        let mut backing = StdBacking::with_capacity(capacity)?;
        let mut root = None;
        backing.with_persistent_arena(|arena| {
            let offset = initialize(arena)?;
            if offset.is_null() {
                return Err(CoreError::InvalidOffset);
            }
            // Validate the initialized typed root before erasing its callback
            // lifetime into a private raw descriptor.
            arena.get(offset)?;
            root = Some(RawRoot {
                offset: offset.as_u32(),
            });
            Ok(())
        })??;
        let root = root.ok_or(StdBackendError::Core(CoreError::InvalidOffset))?;
        Ok(Self {
            backing,
            root,
            marker: PhantomData,
        })
    }

    /// Attach to the persistent allocator and expose the root for one read
    /// callback.
    pub fn with<R, F>(&mut self, action: F) -> Result<R, StdBackendError>
    where
        F: for<'arena, 'memory> FnOnce(&Arena<'arena, 'memory>, RootHandle<'arena, T>) -> R,
    {
        let root = self.root;
        let result = self.backing.with_persistent_arena(|arena| {
            let handle = rebrand_root(root);
            arena.get(handle.offset)?;
            Ok::<R, CoreError>(action(arena, handle))
        })?;
        result.map_err(StdBackendError::from)
    }

    /// Attach to the persistent allocator and expose the root for one mutable
    /// callback.
    pub fn with_mut<R, F>(&mut self, action: F) -> Result<R, StdBackendError>
    where
        F: for<'arena, 'memory> FnOnce(&mut Arena<'arena, 'memory>, RootHandle<'arena, T>) -> R,
    {
        let root = self.root;
        let result = self.backing.with_persistent_arena(|arena| {
            let handle = rebrand_root(root);
            arena.get(handle.offset)?;
            Ok::<R, CoreError>(action(arena, handle))
        })?;
        result.map_err(StdBackendError::from)
    }
}

fn rebrand_root<'arena, T: StoreRoot>(root: RawRoot) -> RootHandle<'arena, T> {
    // SAFETY: the raw descriptor is private to its store and was captured
    // from an initialized T in this exact persistent backing. StoreRoot is
    // Copy, so no release path can reclaim or reuse its non-owning slot.
    let offset = unsafe { Offset32::from_persistent_raw_unchecked(root.offset) };
    RootHandle {
        offset,
        marker: PhantomData,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn rejects_an_invalid_root_before_running_the_callback() {
        let mut store = CompactStore::<u32>::build(256, |arena| arena.alloc_value(7_u32)).unwrap();
        store.root.offset = 0;
        let called = Cell::new(false);

        let error = store.with(|_, _| called.set(true)).unwrap_err();

        assert!(matches!(
            error,
            StdBackendError::Core(CoreError::InvalidOffset)
        ));
        assert!(!called.get());
    }
}
