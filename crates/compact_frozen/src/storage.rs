//! Immutable graph descriptors and one-owner cage storage.

use compact_backend_std::{CageAllocation, CompactRuntime};
use compact_core::{checked_align_up, Error as CoreError};
use core::fmt;
use core::marker::PhantomData;
use core::mem::{align_of, size_of};
use core::ptr;
use core::slice;
use core::str;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_GRAPH_ID: AtomicU32 = AtomicU32::new(1);

/// Errors returned while constructing or reading a frozen graph.
#[derive(Debug)]
pub enum FrozenError {
    /// Cage allocation failed.
    Core(CoreError),
    /// A descriptor does not name a range within this graph.
    InvalidHandle,
    /// Frozen bytes were not valid UTF-8.
    InvalidUtf8,
    /// Graph offsets exceed the 32-bit cage address domain.
    OffsetOverflow,
    /// The process exhausted unique frozen graph identifiers.
    IdentityExhausted,
}

/// Result type used by frozen graph operations.
pub type FrozenResult<T> = core::result::Result<T, FrozenError>;
impl From<CoreError> for FrozenError {
    fn from(error: CoreError) -> Self {
        Self::Core(error)
    }
}
impl fmt::Display for FrozenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => error.fmt(f),
            Self::InvalidHandle => f.write_str("frozen descriptor is outside its graph"),
            Self::InvalidUtf8 => f.write_str("frozen string contains invalid UTF-8"),
            Self::OffsetOverflow => f.write_str("frozen graph exceeds the 32-bit offset domain"),
            Self::IdentityExhausted => f.write_str("frozen graph identity space is exhausted"),
        }
    }
}
impl std::error::Error for FrozenError {}

/// Values that can be copied into and shared from immutable graph storage.
///
/// # Safety
///
/// Implementors must be `Copy + Send + Sync + 'static`, have alignment at most
/// eight, contain no native pointers or interior mutability, require no
/// destructor, and be valid to relocate as initialized bytes. Offset-bearing
/// descriptors must only be interpreted against the graph that owns them.
pub unsafe trait FrozenValue: Copy + Send + Sync + 'static {}
macro_rules! frozen_scalars { ($($ty:ty),* $(,)?) => { $(unsafe impl FrozenValue for $ty {})* }; }
frozen_scalars!(
    (),
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    i8,
    i16,
    i32,
    i64,
    f32,
    f64
);
unsafe impl<T: FrozenValue, const N: usize> FrozenValue for [T; N] {}
unsafe impl<T: FrozenValue> FrozenValue for Option<T> {}
unsafe impl<A: FrozenValue, B: FrozenValue> FrozenValue for (A, B) {}
unsafe impl<A: FrozenValue, B: FrozenValue, C: FrozenValue> FrozenValue for (A, B, C) {}

/// A typed slice descriptor relative to one frozen graph.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenVec<T: FrozenValue> {
    offset: u32,
    len: u32,
    graph_id: u32,
    marker: PhantomData<T>,
}
unsafe impl<T: FrozenValue> FrozenValue for FrozenVec<T> {}
impl<T: FrozenValue> FrozenVec<T> {
    /// Return the number of frozen elements.
    pub const fn len(self) -> usize {
        self.len as usize
    }
    /// Return whether the descriptor is empty.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// An immutable UTF-8 string descriptor relative to one graph.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenString {
    offset: u32,
    len: u32,
    graph_id: u32,
}
unsafe impl FrozenValue for FrozenString {}
impl FrozenString {
    /// Return the UTF-8 byte length.
    pub const fn len(self) -> usize {
        self.len as usize
    }
    /// Return whether the string contains no bytes.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}
/// Immutable bytes descriptor relative to one graph.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenBytes {
    offset: u32,
    len: u32,
    graph_id: u32,
}
unsafe impl FrozenValue for FrozenBytes {}
impl FrozenBytes {
    /// Return the byte length.
    pub const fn len(self) -> usize {
        self.len as usize
    }
    /// Return whether the byte range is empty.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}
/// Immutable OS-string descriptor relative to one graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenOsString(FrozenBytes);
unsafe impl FrozenValue for FrozenOsString {}
/// Immutable path descriptor relative to one graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenPathBuf(FrozenBytes);
unsafe impl FrozenValue for FrozenPathBuf {}
/// Immutable map entries descriptor relative to one graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenMap<K: FrozenValue, V: FrozenValue>(FrozenVec<(K, V)>);
unsafe impl<K: FrozenValue, V: FrozenValue> FrozenValue for FrozenMap<K, V> {}
/// Immutable set descriptor relative to one graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenSet<T: FrozenValue>(FrozenVec<T>);
unsafe impl<T: FrozenValue> FrozenValue for FrozenSet<T> {}

/// Mutable native construction buffer for a frozen graph.
pub struct FrozenBuilder {
    words: Vec<u64>,
    graph_id: u32,
}
impl FrozenBuilder {
    /// Create an empty builder. The first eight bytes stay reserved for null.
    pub fn new() -> FrozenResult<Self> {
        let graph_id = NEXT_GRAPH_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map_err(|_| FrozenError::IdentityExhausted)?;
        Ok(Self {
            words: vec![0],
            graph_id,
        })
    }

    /// Copy a slice of frozen values into this graph.
    pub fn store_slice<T: FrozenValue>(&mut self, values: &[T]) -> FrozenResult<FrozenVec<T>> {
        let offset = self.reserve::<T>(values.len())?;
        if !values.is_empty() {
            // SAFETY: reserve keeps the destination aligned, in-bounds, and initialized.
            unsafe {
                let base = self.words.as_mut_ptr().cast::<u8>();
                let destination = base.add(offset).cast::<T>();
                for (index, value) in values.iter().copied().enumerate() {
                    ptr::write(destination.add(index), value);
                }
            }
        }
        Ok(FrozenVec {
            offset: offset as u32,
            len: u32::try_from(values.len()).map_err(|_| FrozenError::OffsetOverflow)?,
            graph_id: self.graph_id,
            marker: PhantomData,
        })
    }

    /// Store a UTF-8 string and return its descriptor.
    pub fn store_str(&mut self, value: &str) -> FrozenResult<FrozenString> {
        let bytes = self.store_bytes(value.as_bytes())?;
        Ok(FrozenString {
            offset: bytes.offset,
            len: bytes.len,
            graph_id: self.graph_id,
        })
    }
    /// Store arbitrary bytes.
    pub fn store_bytes(&mut self, value: &[u8]) -> FrozenResult<FrozenBytes> {
        let offset = self.reserve::<u8>(value.len())?;
        if !value.is_empty() {
            // SAFETY: reserve allocated this exact byte range.
            unsafe {
                ptr::copy_nonoverlapping(
                    value.as_ptr(),
                    self.words.as_mut_ptr().cast::<u8>().add(offset),
                    value.len(),
                )
            };
        }
        Ok(FrozenBytes {
            offset: offset as u32,
            len: u32::try_from(value.len()).map_err(|_| FrozenError::OffsetOverflow)?,
            graph_id: self.graph_id,
        })
    }
    /// Store a platform-local OS string representation.
    pub fn store_os_string(&mut self, value: &std::ffi::OsStr) -> FrozenResult<FrozenOsString> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Ok(FrozenOsString(self.store_bytes(value.as_bytes())?))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let bytes: Vec<u8> = value.encode_wide().flat_map(u16::to_le_bytes).collect();
            Ok(FrozenOsString(self.store_bytes(&bytes)?))
        }
    }
    /// Store a platform-local path representation.
    pub fn store_path(&mut self, value: &std::path::Path) -> FrozenResult<FrozenPathBuf> {
        Ok(FrozenPathBuf(self.store_os_string(value.as_os_str())?.0))
    }

    /// Finish the graph by copying the root and all payloads into one cage block.
    pub fn finish<T: FrozenValue>(mut self, root: T) -> FrozenResult<FrozenGraph<T>> {
        let root = self.store_slice(core::slice::from_ref(&root))?;
        let capacity = self.words.len();
        let mut storage = CompactRuntime::alloc_owned_slice::<u64>(capacity)?;
        storage.extend_copy(&self.words)?;
        Ok(FrozenGraph {
            storage,
            used_bytes: capacity * size_of::<u64>(),
            root_offset: root.offset,
            graph_id: self.graph_id,
            marker: PhantomData,
        })
    }

    fn reserve<T: FrozenValue>(&mut self, count: usize) -> FrozenResult<usize> {
        let alignment = align_of::<T>();
        if alignment > 8 {
            return Err(FrozenError::OffsetOverflow);
        }
        let used = self
            .words
            .len()
            .checked_mul(size_of::<u64>())
            .ok_or(FrozenError::OffsetOverflow)?;
        let offset = checked_align_up(used, alignment).map_err(|_| FrozenError::OffsetOverflow)?;
        let bytes = count
            .checked_mul(size_of::<T>())
            .ok_or(FrozenError::OffsetOverflow)?;
        let end = offset
            .checked_add(bytes)
            .ok_or(FrozenError::OffsetOverflow)?;
        if end > u32::MAX as usize {
            return Err(FrozenError::OffsetOverflow);
        }
        let words = end.div_ceil(size_of::<u64>());
        if words > self.words.len() {
            self.words
                .try_reserve(words - self.words.len())
                .map_err(|_| FrozenError::Core(CoreError::AllocationFailed))?;
            self.words.resize(words, 0);
        }
        Ok(offset)
    }
}

/// Immutable graph owning exactly one cage allocation.
pub struct FrozenGraph<T: FrozenValue> {
    storage: CageAllocation<u64>,
    used_bytes: usize,
    root_offset: u32,
    graph_id: u32,
    marker: PhantomData<T>,
}
impl<T: FrozenValue> FrozenGraph<T> {
    /// Borrow the immutable root value.
    pub fn root(&self) -> &T {
        self.reference(self.root_offset)
            .expect("builder produced a valid root")
    }
    /// Return bytes owned by the graph, including alignment padding.
    pub const fn used_bytes(&self) -> usize {
        self.used_bytes
    }
    /// Read a typed frozen slice descriptor.
    pub fn slice<U: FrozenValue>(&self, values: FrozenVec<U>) -> FrozenResult<&[U]> {
        let bytes = (values.len as usize)
            .checked_mul(size_of::<U>())
            .ok_or(FrozenError::InvalidHandle)?;
        self.check_range(values.graph_id, values.offset, bytes)?;
        if values.len == 0 {
            return Ok(&[]);
        }
        if values.offset as usize % align_of::<U>() != 0 {
            return Err(FrozenError::InvalidHandle);
        }
        // SAFETY: the builder wrote every element at this aligned, in-bounds range.
        Ok(unsafe {
            slice::from_raw_parts(
                self.byte_ptr(values.offset).cast::<U>(),
                values.len as usize,
            )
        })
    }
    /// Read a frozen UTF-8 string.
    pub fn str(&self, value: FrozenString) -> FrozenResult<&str> {
        self.check_range(value.graph_id, value.offset, value.len as usize)?;
        // SAFETY: the descriptor is bounds-checked; construction stored the original str bytes.
        str::from_utf8(unsafe {
            slice::from_raw_parts(self.byte_ptr(value.offset), value.len as usize)
        })
        .map_err(|_| FrozenError::InvalidUtf8)
    }
    /// Read frozen bytes.
    pub fn bytes(&self, value: FrozenBytes) -> FrozenResult<&[u8]> {
        self.check_range(value.graph_id, value.offset, value.len as usize)?;
        // SAFETY: the descriptor is bounds-checked against the owned graph block.
        Ok(unsafe { slice::from_raw_parts(self.byte_ptr(value.offset), value.len as usize) })
    }
    /// Read a frozen OS string as its target-local bytes.
    pub fn os_string_bytes(&self, value: FrozenOsString) -> FrozenResult<&[u8]> {
        self.bytes(value.0)
    }
    /// Read a frozen path as its target-local bytes.
    pub fn path_bytes(&self, value: FrozenPathBuf) -> FrozenResult<&[u8]> {
        self.bytes(value.0)
    }
    /// Read key-value entries from a frozen map descriptor.
    pub fn map_entries<K: FrozenValue, V: FrozenValue>(
        &self,
        value: FrozenMap<K, V>,
    ) -> FrozenResult<&[(K, V)]> {
        self.slice(value.0)
    }
    /// Read entries from a frozen set descriptor.
    pub fn set_entries<U: FrozenValue>(&self, value: FrozenSet<U>) -> FrozenResult<&[U]> {
        self.slice(value.0)
    }

    fn reference<U: FrozenValue>(&self, offset: u32) -> FrozenResult<&U> {
        self.check_range(self.graph_id, offset, size_of::<U>())?;
        if offset as usize % align_of::<U>() != 0 {
            return Err(FrozenError::InvalidHandle);
        }
        // SAFETY: bounds and alignment are checked against builder-created bytes.
        Ok(unsafe { &*self.byte_ptr(offset).cast::<U>() })
    }
    fn check_range(&self, graph_id: u32, offset: u32, bytes: usize) -> FrozenResult<()> {
        let end = (offset as usize)
            .checked_add(bytes)
            .ok_or(FrozenError::InvalidHandle)?;
        if graph_id != self.graph_id || offset == 0 || end > self.used_bytes {
            return Err(FrozenError::InvalidHandle);
        }
        Ok(())
    }
    fn byte_ptr(&self, offset: u32) -> *const u8 {
        self.storage
            .as_slice()
            .as_ptr()
            .cast::<u8>()
            .wrapping_add(offset as usize)
    }
}
