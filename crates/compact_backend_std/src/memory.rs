//! Fixed stable memory allocation using portable `std` facilities.

use std::collections::TryReserveError;
use std::fmt;
use std::mem::MaybeUninit;

use compact_core::{
    with_arena, with_arena_attached, with_arena_persistent, Arena, Error as CoreError,
    Result as CoreResult, StableBacking,
};

/// An owned, fixed-size standard-library backing allocation.
///
/// The internal vector is allocated once, has a private fixed length, and is
/// never resized. Moving this owner moves only the vector handle; its heap
/// allocation remains at the same address until the owner is dropped.
pub struct StdBacking {
    memory: Vec<MaybeUninit<u8>>,
    persistent_initialized: bool,
}

impl StdBacking {
    /// Allocate fixed backing memory with the requested capacity.
    pub fn with_capacity(capacity: usize) -> core::result::Result<Self, StdBackendError> {
        if capacity < compact_core::MIN_ARENA_BYTES {
            return Err(StdBackendError::Core(CoreError::InvalidCapacity));
        }
        if capacity as u64 > compact_core::MAX_ARENA_BYTES {
            return Err(StdBackendError::Core(CoreError::BackingTooLarge));
        }

        let mut memory = Vec::new();
        memory
            .try_reserve_exact(capacity)
            .map_err(StdBackendError::Allocation)?;
        // SAFETY: MaybeUninit<u8> permits every bit pattern, including an
        // uninitialized payload. The vector reserved at least `capacity`
        // elements above; set_len only exposes that reserved region and does
        // not claim the bytes contain initialized u8 values.
        unsafe { memory.set_len(capacity) };
        Ok(Self {
            memory,
            persistent_initialized: false,
        })
    }

    /// Return the fixed usable capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.memory.len()
    }

    /// Run `action` in a fresh arena scope over this backing allocation.
    pub fn with_arena<R, F>(&mut self, action: F) -> CoreResult<R>
    where
        F: for<'arena, 'memory> FnOnce(&mut Arena<'arena, 'memory>) -> R,
    {
        self.persistent_initialized = false;
        with_arena(self, action)
    }

    /// Initialize or reattach the persistent allocator state in this backing.
    ///
    /// Unlike [`with_arena`](Self::with_arena), this keeps non-owning compact
    /// data and allocator metadata available between callback invocations.
    /// The higher-ranked callback prevents arena-branded borrows from escaping.
    pub(crate) fn with_persistent_arena<R, F>(&mut self, action: F) -> CoreResult<R>
    where
        F: for<'arena, 'memory> FnOnce(&mut Arena<'arena, 'memory>) -> R,
    {
        if self.persistent_initialized {
            // SAFETY: only this private fixed backing can set the flag, and its
            // allocation is never resized or exposed while attached.
            unsafe { with_arena_attached(self, action) }
        } else {
            let result = with_arena_persistent(self, action)?;
            self.persistent_initialized = true;
            Ok(result)
        }
    }
}

// SAFETY: `memory` is allocated once, never resized through the private API,
// and its mutable slice borrow prevents moving or dropping the vector while
// the arena uses the returned region.
unsafe impl StableBacking for StdBacking {
    fn bytes_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        &mut self.memory
    }
}

/// Convenience namespace for a temporary std-backed arena.
pub struct StdArena;

impl StdArena {
    /// Allocate fixed backing memory, invoke `action`, then release the
    /// backing when the callback returns.
    pub fn with_capacity<R, F>(
        capacity: usize,
        action: F,
    ) -> core::result::Result<R, StdBackendError>
    where
        F: for<'arena, 'memory> FnOnce(&mut Arena<'arena, 'memory>) -> R,
    {
        let mut backing = StdBacking::with_capacity(capacity)?;
        Ok(backing.with_arena(action)?)
    }
}

/// Errors from allocating or using std-backed memory.
#[derive(Debug)]
pub enum StdBackendError {
    /// The core rejected the requested capacity or arena operation.
    Core(CoreError),
    /// The system allocator could not reserve the requested memory.
    Allocation(TryReserveError),
}

impl fmt::Display for StdBackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => error.fmt(formatter),
            Self::Allocation(error) => {
                write!(formatter, "failed to allocate compact backing: {error}")
            }
        }
    }
}

impl std::error::Error for StdBackendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Core(error) => Some(error),
            Self::Allocation(error) => Some(error),
        }
    }
}

impl From<CoreError> for StdBackendError {
    fn from(error: CoreError) -> Self {
        Self::Core(error)
    }
}
