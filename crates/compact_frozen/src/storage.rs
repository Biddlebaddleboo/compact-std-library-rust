//! Immutable graph descriptors and one-owner cage storage.

use compact_backend_std::{CageAllocation, CompactRuntime};
use compact_core::{checked_align_up, Error as CoreError};
use core::fmt;
use core::marker::PhantomData;
use core::mem::{align_of, size_of};
use core::ptr;
use core::slice;
use core::str;

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

/// An eight-byte typed slice descriptor stored inside one frozen graph.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenVec<T: FrozenValue> {
    offset: u32,
    len: u32,
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

/// An eight-byte immutable UTF-8 string descriptor stored inside one graph.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenString {
    offset: u32,
    len: u32,
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
/// An eight-byte immutable byte descriptor stored inside one graph.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenBytes {
    offset: u32,
    len: u32,
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
}
impl FrozenBuilder {
    /// Create an empty builder. The first eight bytes stay reserved for null.
    pub fn new() -> FrozenResult<Self> {
        Ok(Self { words: vec![0] })
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
            marker: PhantomData,
        })
    }

    /// Store a UTF-8 string and return its descriptor.
    pub fn store_str(&mut self, value: &str) -> FrozenResult<FrozenString> {
        let bytes = self.store_bytes(value.as_bytes())?;
        Ok(FrozenString {
            offset: bytes.offset,
            len: bytes.len,
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
        let used_bytes = capacity
            .checked_mul(size_of::<u64>())
            .and_then(|bytes| u32::try_from(bytes).ok())
            .ok_or(FrozenError::OffsetOverflow)?;
        let mut storage = CompactRuntime::alloc_owned_slice::<u64>(capacity)?;
        storage.extend_copy(&self.words)?;
        Ok(FrozenGraph {
            storage,
            used_bytes,
            root_offset: root.offset,
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
    used_bytes: u32,
    root_offset: u32,
    marker: PhantomData<T>,
}

/// Borrowed graph view for repeated traversal without resolving cage storage
/// again for every descriptor.
#[derive(Clone, Copy)]
pub struct FrozenGraphView<'g> {
    bytes: &'g [u8],
}

impl<T: FrozenValue> FrozenGraph<T> {
    /// Borrow the immutable root value.
    pub fn root(&self) -> &T {
        self.reference(self.root_offset)
            .expect("builder produced a valid root")
    }
    /// Return bytes owned by the graph, including alignment padding.
    pub const fn used_bytes(&self) -> usize {
        self.used_bytes as usize
    }
    /// Resolve a borrowed view for repeated descriptor reads.
    pub fn view(&self) -> FrozenGraphView<'_> {
        let bytes = self.storage.as_byte_slice();
        let bytes = bytes
            .get(..self.used_bytes as usize)
            .expect("builder produced a complete graph byte range");
        FrozenGraphView { bytes }
    }
    /// Read a typed frozen slice descriptor.
    pub fn slice<U: FrozenValue>(&self, values: &FrozenVec<U>) -> FrozenResult<&[U]> {
        self.check_descriptor(values)?;
        let bytes = (values.len as usize)
            .checked_mul(size_of::<U>())
            .ok_or(FrozenError::InvalidHandle)?;
        self.check_range(values.offset, bytes)?;
        if values.len == 0 {
            return Ok(&[]);
        }
        if values.offset as usize % align_of::<U>() != 0 {
            return Err(FrozenError::InvalidHandle);
        }
        // SAFETY: the builder wrote every element at this aligned, in-bounds range.
        Ok(unsafe {
            slice::from_raw_parts(
                self.byte_ptr(values.offset)?.as_ptr().cast::<U>(),
                values.len as usize,
            )
        })
    }
    /// Read a frozen UTF-8 string.
    pub fn str(&self, value: &FrozenString) -> FrozenResult<&str> {
        self.check_descriptor(value)?;
        self.check_range(value.offset, value.len as usize)?;
        // SAFETY: the descriptor is bounds-checked; construction stored the original str bytes.
        str::from_utf8(
            self.byte_ptr(value.offset)?
                .get(..value.len as usize)
                .ok_or(FrozenError::InvalidHandle)?,
        )
        .map_err(|_| FrozenError::InvalidUtf8)
    }
    /// Read frozen bytes.
    pub fn bytes(&self, value: &FrozenBytes) -> FrozenResult<&[u8]> {
        self.check_descriptor(value)?;
        self.check_range(value.offset, value.len as usize)?;
        self.byte_ptr(value.offset)?
            .get(..value.len as usize)
            .ok_or(FrozenError::InvalidHandle)
    }
    /// Read a frozen OS string as its target-local bytes.
    pub fn os_string_bytes(&self, value: &FrozenOsString) -> FrozenResult<&[u8]> {
        self.check_descriptor(value)?;
        self.bytes(&value.0)
    }
    /// Read a frozen path as its target-local bytes.
    pub fn path_bytes(&self, value: &FrozenPathBuf) -> FrozenResult<&[u8]> {
        self.check_descriptor(value)?;
        self.bytes(&value.0)
    }
    /// Read key-value entries from a frozen map descriptor.
    pub fn map_entries<K: FrozenValue, V: FrozenValue>(
        &self,
        value: &FrozenMap<K, V>,
    ) -> FrozenResult<&[(K, V)]> {
        self.check_descriptor(value)?;
        self.slice(&value.0)
    }
    /// Read entries from a frozen set descriptor.
    pub fn set_entries<U: FrozenValue>(&self, value: &FrozenSet<U>) -> FrozenResult<&[U]> {
        self.check_descriptor(value)?;
        self.slice(&value.0)
    }

    fn reference<U: FrozenValue>(&self, offset: u32) -> FrozenResult<&U> {
        self.check_range(offset, size_of::<U>())?;
        if offset as usize % align_of::<U>() != 0 {
            return Err(FrozenError::InvalidHandle);
        }
        // SAFETY: bounds and alignment are checked against builder-created bytes.
        Ok(unsafe { &*self.byte_ptr(offset)?.as_ptr().cast::<U>() })
    }
    fn check_descriptor<U>(&self, descriptor: &U) -> FrozenResult<()> {
        let base = self.storage.as_byte_slice().as_ptr() as usize;
        let graph_end = base
            .checked_add(self.used_bytes as usize)
            .ok_or(FrozenError::InvalidHandle)?;
        let descriptor_start = descriptor as *const U as usize;
        let descriptor_end = descriptor_start
            .checked_add(size_of::<U>())
            .ok_or(FrozenError::InvalidHandle)?;
        if descriptor_start < base || descriptor_end > graph_end {
            return Err(FrozenError::InvalidHandle);
        }
        Ok(())
    }
    fn check_range(&self, offset: u32, bytes: usize) -> FrozenResult<()> {
        let end = (offset as usize)
            .checked_add(bytes)
            .ok_or(FrozenError::InvalidHandle)?;
        if offset == 0 || end > self.used_bytes as usize {
            return Err(FrozenError::InvalidHandle);
        }
        Ok(())
    }
    fn byte_ptr(&self, offset: u32) -> FrozenResult<&[u8]> {
        self.storage
            .as_byte_slice()
            .get(offset as usize..)
            .ok_or(FrozenError::InvalidHandle)
    }
}

impl<'g> FrozenGraphView<'g> {
    /// Read a typed frozen slice descriptor from this graph.
    pub fn slice<U: FrozenValue>(&self, values: &FrozenVec<U>) -> FrozenResult<&'g [U]> {
        self.check_descriptor(values)?;
        let bytes = (values.len as usize)
            .checked_mul(size_of::<U>())
            .ok_or(FrozenError::InvalidHandle)?;
        self.check_range(values.offset, bytes)?;
        if values.len == 0 {
            return Ok(&[]);
        }
        if values.offset as usize % align_of::<U>() != 0 {
            return Err(FrozenError::InvalidHandle);
        }
        let ptr = self.byte_ptr(values.offset)?.as_ptr().cast::<U>();
        // SAFETY: descriptor identity, target bounds, and alignment are checked;
        // the graph view's borrow keeps the owning bytes alive.
        Ok(unsafe { slice::from_raw_parts(ptr, values.len as usize) })
    }

    /// Read a frozen UTF-8 string descriptor from this graph.
    pub fn str(&self, value: &FrozenString) -> FrozenResult<&'g str> {
        self.check_descriptor(value)?;
        self.check_range(value.offset, value.len as usize)?;
        str::from_utf8(
            self.byte_ptr(value.offset)?
                .get(..value.len as usize)
                .ok_or(FrozenError::InvalidHandle)?,
        )
        .map_err(|_| FrozenError::InvalidUtf8)
    }

    /// Read a frozen byte descriptor from this graph.
    pub fn bytes(&self, value: &FrozenBytes) -> FrozenResult<&'g [u8]> {
        self.check_descriptor(value)?;
        self.check_range(value.offset, value.len as usize)?;
        self.byte_ptr(value.offset)?
            .get(..value.len as usize)
            .ok_or(FrozenError::InvalidHandle)
    }

    /// Read a frozen OS string as its target-local bytes.
    pub fn os_string_bytes(&self, value: &FrozenOsString) -> FrozenResult<&'g [u8]> {
        self.check_descriptor(value)?;
        self.bytes(&value.0)
    }

    /// Read a frozen path as its target-local bytes.
    pub fn path_bytes(&self, value: &FrozenPathBuf) -> FrozenResult<&'g [u8]> {
        self.check_descriptor(value)?;
        self.bytes(&value.0)
    }

    /// Read key-value entries from a frozen map descriptor.
    pub fn map_entries<K: FrozenValue, V: FrozenValue>(
        &self,
        value: &FrozenMap<K, V>,
    ) -> FrozenResult<&'g [(K, V)]> {
        self.check_descriptor(value)?;
        self.slice(&value.0)
    }

    /// Read entries from a frozen set descriptor.
    pub fn set_entries<U: FrozenValue>(&self, value: &FrozenSet<U>) -> FrozenResult<&'g [U]> {
        self.check_descriptor(value)?;
        self.slice(&value.0)
    }

    fn check_descriptor<U>(&self, descriptor: &U) -> FrozenResult<()> {
        let base = self.bytes.as_ptr() as usize;
        let end = base
            .checked_add(self.bytes.len())
            .ok_or(FrozenError::InvalidHandle)?;
        let descriptor_start = descriptor as *const U as usize;
        let descriptor_end = descriptor_start
            .checked_add(size_of::<U>())
            .ok_or(FrozenError::InvalidHandle)?;
        if descriptor_start < base || descriptor_end > end {
            return Err(FrozenError::InvalidHandle);
        }
        Ok(())
    }

    fn check_range(&self, offset: u32, bytes: usize) -> FrozenResult<()> {
        let end = (offset as usize)
            .checked_add(bytes)
            .ok_or(FrozenError::InvalidHandle)?;
        if offset == 0 || end > self.bytes.len() {
            return Err(FrozenError::InvalidHandle);
        }
        Ok(())
    }

    fn byte_ptr(&self, offset: u32) -> FrozenResult<&'g [u8]> {
        self.bytes
            .get(offset as usize..)
            .ok_or(FrozenError::InvalidHandle)
    }
}
