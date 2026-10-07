//! Immutable frozen backing and typed offset handles.

use core::fmt;
use core::marker::PhantomData;
use core::mem::{align_of, size_of, MaybeUninit};
use core::ptr;
use std::sync::Mutex;

use compact_collections::CollectionError;

/// Errors returned while constructing or reading a frozen arena.
#[derive(Debug)]
pub enum FrozenError {
    /// A source compact owner failed validation.
    Collection(CollectionError),
    /// The host allocator could not reserve frozen backing memory.
    Allocation(std::collections::TryReserveError),
    /// A frozen byte range exceeded the V2.1 offset domain.
    OffsetOverflow,
    /// A handle belongs to a different arena or names an invalid range.
    InvalidHandle,
    /// Frozen bytes were not valid UTF-8 when viewed as a string.
    InvalidUtf8,
    /// The process exhausted the unique frozen-arena identity space.
    IdentityExhausted,
}

/// Result type used by frozen construction and access.
pub type FrozenResult<T> = core::result::Result<T, FrozenError>;

impl From<CollectionError> for FrozenError {
    fn from(error: CollectionError) -> Self {
        Self::Collection(error)
    }
}

impl fmt::Display for FrozenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Collection(error) => error.fmt(formatter),
            Self::Allocation(error) => {
                write!(formatter, "frozen backing allocation failed: {error}")
            }
            Self::OffsetOverflow => {
                formatter.write_str("frozen data exceeds the 32-bit offset domain")
            }
            Self::InvalidHandle => {
                formatter.write_str("frozen handle does not belong to this arena")
            }
            Self::InvalidUtf8 => formatter.write_str("frozen string contains invalid UTF-8"),
            Self::IdentityExhausted => {
                formatter.write_str("frozen arena identity space is exhausted")
            }
        }
    }
}

impl std::error::Error for FrozenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Collection(error) => Some(error),
            Self::Allocation(error) => Some(error),
            _ => None,
        }
    }
}

/// A type that may be copied into immutable frozen storage.
///
/// # Safety
///
/// Implementors must be `Copy + Send + Sync + 'static`, have alignment no
/// greater than 64 bytes, and be safe to relocate by copying their initialized
/// bytes. They must not contain references, owning native pointers, interior
/// mutability, address-sensitive state, or any resource that requires a
/// destructor. Every value passed to a frozen builder must already be a valid
/// initialized value of the implementor type. Implementors must remain safe
/// to share through immutable references for the lifetime of the backing.
pub unsafe trait FrozenValue: Copy + Send + Sync + 'static {}

macro_rules! frozen_scalars {
    // SAFETY: these scalar values are Copy, contain no pointers or resources,
    // and are valid to relocate and share through immutable references.
    ($($ty:ty),* $(,)?) => {
        $(unsafe impl FrozenValue for $ty {})*
    };
}

frozen_scalars!(
    (),
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64
);

// SAFETY: these compositions contain only values already satisfying the
// FrozenValue contract and add no pointers, mutation, or drop obligations.
unsafe impl<T: FrozenValue, const N: usize> FrozenValue for [T; N] {}
unsafe impl<T: FrozenValue> FrozenValue for Option<T> {}
unsafe impl<T: FrozenValue, E: FrozenValue> FrozenValue for core::result::Result<T, E> {}
unsafe impl<A: FrozenValue> FrozenValue for (A,) {}
unsafe impl<A: FrozenValue, B: FrozenValue> FrozenValue for (A, B) {}
unsafe impl<A: FrozenValue, B: FrozenValue, C: FrozenValue> FrozenValue for (A, B, C) {}
// SAFETY: this tuple composes only FrozenValue members and has no additional
// pointer, mutation, or destructor behavior.
unsafe impl<A: FrozenValue, B: FrozenValue, C: FrozenValue, D: FrozenValue> FrozenValue
    for (A, B, C, D)
{
}

static NEXT_ARENA_ID: Mutex<usize> = Mutex::new(1);

fn allocate_arena_id() -> FrozenResult<usize> {
    let mut next = NEXT_ARENA_ID
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = *next;
    *next = next.checked_add(1).ok_or(FrozenError::IdentityExhausted)?;
    Ok(id)
}

/// Mutable construction state for a new frozen backing.
///
/// It is consumed by [`finish_root`](Self::finish_root); only the finished
/// arena exposes immutable access.
pub struct FrozenBuilder {
    id: usize,
    words: Vec<FrozenWord>,
    used: usize,
}

impl FrozenBuilder {
    /// Create a builder with a fresh arena identity.
    pub fn new() -> FrozenResult<Self> {
        Ok(Self {
            id: allocate_arena_id()?,
            words: Vec::new(),
            // Zero is reserved as the null offset, matching the mutable arena.
            used: 1,
        })
    }

    /// Copy a slice of freeze-safe values into aligned immutable storage.
    pub fn store_slice<T: FrozenValue>(&mut self, values: &[T]) -> FrozenResult<FrozenVec<T>> {
        let len = u32::try_from(values.len()).map_err(|_| FrozenError::OffsetOverflow)?;
        let (offset, end) = self.reserve::<T>(values.len())?;
        if !values.is_empty() {
            let base = self.words.as_mut_ptr().cast::<u8>();
            // SAFETY: reserve ensures the complete byte range exists, the
            // offset meets T's alignment, and FrozenValue permits relocation.
            unsafe {
                let destination = base.add(offset).cast::<T>();
                if size_of::<T>() == 0 {
                    ptr::write(destination, values[0]);
                } else {
                    for (index, value) in values.iter().copied().enumerate() {
                        ptr::write(destination.add(index), value);
                    }
                }
            }
        }
        self.used = end;
        Ok(FrozenVec {
            id: self.id,
            offset: offset as u32,
            len,
            marker: PhantomData,
        })
    }

    /// Copy UTF-8 text into immutable frozen storage.
    pub fn store_str(&mut self, value: &str) -> FrozenResult<FrozenString> {
        let bytes = self.store_slice(value.as_bytes())?;
        Ok(FrozenString {
            bytes: FrozenBytes { bytes },
        })
    }

    /// Copy arbitrary bytes into immutable frozen storage.
    pub fn store_bytes(&mut self, value: &[u8]) -> FrozenResult<FrozenBytes> {
        Ok(FrozenBytes {
            bytes: self.store_slice(value)?,
        })
    }

    /// Store the root value and commit this builder into a frozen arena.
    ///
    /// If root storage cannot be reserved, the builder remains valid and can
    /// still be dropped without affecting the source arena.
    pub fn finish_root<T: FrozenValue>(
        mut self,
        root: T,
    ) -> FrozenResult<(FrozenArena, FrozenRoot<T>)> {
        let handle = self.store_slice(core::slice::from_ref(&root))?;
        let arena = FrozenArena {
            id: self.id,
            words: self.words,
            used: self.used,
        };
        let root = FrozenRoot { value: handle };
        Ok((arena, root))
    }

    fn reserve<T: FrozenValue>(&mut self, len: usize) -> FrozenResult<(usize, usize)> {
        let alignment = align_of::<T>();
        if alignment > align_of::<FrozenWord>() {
            return Err(FrozenError::OffsetOverflow);
        }
        let offset = checked_align_up(self.used, alignment).ok_or(FrozenError::OffsetOverflow)?;
        let byte_len = size_of::<T>()
            .checked_mul(len)
            .ok_or(FrozenError::OffsetOverflow)?;
        let end = offset
            .checked_add(byte_len)
            .ok_or(FrozenError::OffsetOverflow)?;
        if end > u32::MAX as usize {
            return Err(FrozenError::OffsetOverflow);
        }
        let word_bytes = size_of::<FrozenWord>();
        let word_count = end
            .checked_add(word_bytes - 1)
            .ok_or(FrozenError::OffsetOverflow)?
            / word_bytes;
        if word_count > self.words.len() {
            self.words
                .try_reserve(word_count - self.words.len())
                .map_err(FrozenError::Allocation)?;
            self.words.resize_with(word_count, FrozenWord::uninit);
        }
        Ok((offset, end))
    }
}

#[repr(align(64))]
struct FrozenWord {
    _bytes: [MaybeUninit<u8>; 64],
}

impl FrozenWord {
    fn uninit() -> Self {
        Self {
            _bytes: [MaybeUninit::uninit(); 64],
        }
    }
}

fn checked_align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment.checked_sub(1)?)
        .map(|aligned| aligned & !(alignment - 1))
}

/// Immutable byte backing containing only trusted frozen values.
///
/// No API can map arbitrary input bytes into this type. The internal word
/// vector is never mutably exposed after construction, and `FrozenValue` has
/// no destructor obligations. This gives the type automatic `Send + Sync`
/// behavior without changing any mutable arena owner.
pub struct FrozenArena {
    id: usize,
    words: Vec<FrozenWord>,
    used: usize,
}

impl FrozenArena {
    /// Return the number of bytes occupied by the frozen value graph.
    pub fn used_bytes(&self) -> usize {
        self.used
    }

    fn checked_ptr<T: FrozenValue>(
        &self,
        id: usize,
        offset: u32,
        len: u32,
    ) -> FrozenResult<*const T> {
        if id != self.id {
            return Err(FrozenError::InvalidHandle);
        }
        let offset = offset as usize;
        let byte_len = size_of::<T>()
            .checked_mul(len as usize)
            .ok_or(FrozenError::InvalidHandle)?;
        let end = offset
            .checked_add(byte_len)
            .ok_or(FrozenError::InvalidHandle)?;
        if offset == 0 || end > self.used || align_of::<T>() > align_of::<FrozenWord>() {
            return Err(FrozenError::InvalidHandle);
        }
        let base = self.words.as_ptr().cast::<u8>();
        // SAFETY: `offset` is nonzero and `end <= used`; builders retain at
        // least one aligned word so even zero-sized values have a valid base.
        let pointer = unsafe { base.add(offset).cast::<T>() };
        if (pointer as usize) % align_of::<T>() != 0 {
            return Err(FrozenError::InvalidHandle);
        }
        Ok(pointer)
    }

    fn slice<T: FrozenValue>(&self, id: usize, offset: u32, len: u32) -> FrozenResult<&[T]> {
        let pointer = self.checked_ptr::<T>(id, offset, len)?;
        // SAFETY: the trusted builder initialized every element in this range
        // as T before finish, the handle's arena ID/range/alignment were
        // checked, and FrozenArena never exposes mutation after finish.
        Ok(unsafe { core::slice::from_raw_parts(pointer, len as usize) })
    }
}

impl fmt::Debug for FrozenArena {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrozenArena")
            .field("used", &self.used)
            .finish_non_exhaustive()
    }
}

/// A frozen root value tied to one immutable arena identity.
pub struct FrozenRoot<T: FrozenValue> {
    value: FrozenVec<T>,
}

impl<T: FrozenValue> Copy for FrozenRoot<T> {}
impl<T: FrozenValue> Clone for FrozenRoot<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: FrozenValue> FrozenRoot<T> {
    /// Borrow the root value from its matching frozen arena.
    pub fn get<'arena>(&self, arena: &'arena FrozenArena) -> FrozenResult<&'arena T> {
        arena
            .slice(self.value.id, self.value.offset, self.value.len)?
            .first()
            .ok_or(FrozenError::InvalidHandle)
    }
}

/// A frozen, immutable sequence handle.
pub struct FrozenVec<T: FrozenValue> {
    id: usize,
    offset: u32,
    len: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T: FrozenValue> Copy for FrozenVec<T> {}
impl<T: FrozenValue> Clone for FrozenVec<T> {
    fn clone(&self) -> Self {
        *self
    }
}

// SAFETY: the descriptor is Copy metadata; the arena owns its immutable
// payload, and the type parameter is restricted to FrozenValue.
unsafe impl<T: FrozenValue> FrozenValue for FrozenVec<T> {}

impl<T: FrozenValue> FrozenVec<T> {
    /// Return the sequence length.
    pub fn len(self) -> usize {
        self.len as usize
    }

    /// Return whether the sequence is empty.
    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Borrow the initialized immutable elements.
    pub fn as_slice<'arena>(&self, arena: &'arena FrozenArena) -> FrozenResult<&'arena [T]> {
        arena.slice(self.id, self.offset, self.len)
    }

    /// Borrow the element at `index`, if present.
    pub fn get<'arena>(
        &self,
        index: usize,
        arena: &'arena FrozenArena,
    ) -> FrozenResult<Option<&'arena T>> {
        Ok(self.as_slice(arena)?.get(index))
    }
}

/// A frozen UTF-8 string handle.
#[derive(Clone, Copy)]
pub struct FrozenString {
    bytes: FrozenBytes,
}

// SAFETY: the descriptor is Copy metadata over an immutable byte range.
unsafe impl FrozenValue for FrozenString {}

impl FrozenString {
    /// Borrow the string from its matching frozen arena.
    pub fn as_str<'arena>(&self, arena: &'arena FrozenArena) -> FrozenResult<&'arena str> {
        core::str::from_utf8(self.bytes.as_slice(arena)?).map_err(|_| FrozenError::InvalidUtf8)
    }

    /// Return the UTF-8 byte count.
    pub fn len(self) -> usize {
        self.bytes.len()
    }

    /// Return whether the string is empty.
    pub fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }
}

/// A frozen arbitrary byte sequence handle.
#[derive(Clone, Copy)]
pub struct FrozenBytes {
    bytes: FrozenVec<u8>,
}

// SAFETY: the descriptor is Copy metadata over an immutable byte range.
unsafe impl FrozenValue for FrozenBytes {}

impl FrozenBytes {
    /// Borrow the bytes from their matching frozen arena.
    pub fn as_slice<'arena>(&self, arena: &'arena FrozenArena) -> FrozenResult<&'arena [u8]> {
        self.bytes.as_slice(arena)
    }

    /// Return the number of bytes.
    pub fn len(self) -> usize {
        self.bytes.len()
    }

    /// Return whether the sequence is empty.
    pub fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }
}

/// A frozen deque represented by values in logical order.
#[derive(Clone, Copy)]
pub struct FrozenVecDeque<T: FrozenValue> {
    pub(crate) values: FrozenVec<T>,
}

// SAFETY: the descriptor is Copy metadata over an immutable sequence.
unsafe impl<T: FrozenValue> FrozenValue for FrozenVecDeque<T> {}

impl<T: FrozenValue> FrozenVecDeque<T> {
    /// Return the number of frozen values.
    pub fn len(self) -> usize {
        self.values.len()
    }

    /// Return whether the deque is empty.
    pub fn is_empty(self) -> bool {
        self.values.is_empty()
    }

    /// Borrow the values in logical deque order.
    pub fn as_slice<'arena>(&self, arena: &'arena FrozenArena) -> FrozenResult<&'arena [T]> {
        self.values.as_slice(arena)
    }
}

/// A frozen map represented by immutable key/value pairs.
#[derive(Clone, Copy)]
pub struct FrozenMap<K: FrozenValue, V: FrozenValue> {
    pub(crate) entries: FrozenVec<(K, V)>,
}

// SAFETY: the descriptor is Copy metadata over immutable key/value pairs.
unsafe impl<K: FrozenValue, V: FrozenValue> FrozenValue for FrozenMap<K, V> {}

impl<K: FrozenValue, V: FrozenValue> FrozenMap<K, V> {
    /// Return the number of frozen entries.
    pub fn len(self) -> usize {
        self.entries.len()
    }

    /// Return whether the map is empty.
    pub fn is_empty(self) -> bool {
        self.entries.is_empty()
    }

    /// Borrow the frozen key/value pairs.
    pub fn iter<'arena>(
        &self,
        arena: &'arena FrozenArena,
    ) -> FrozenResult<core::slice::Iter<'arena, (K, V)>> {
        Ok(self.entries.as_slice(arena)?.iter())
    }

    /// Find a value using immutable linear lookup.
    pub fn get<'arena>(
        &self,
        key: &K,
        arena: &'arena FrozenArena,
    ) -> FrozenResult<Option<&'arena V>>
    where
        K: Eq,
    {
        Ok(self
            .entries
            .as_slice(arena)?
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value))
    }

    /// Find a key with a caller-supplied comparison and return its pair.
    pub fn find_by<'arena, F>(
        &self,
        arena: &'arena FrozenArena,
        mut matches: F,
    ) -> FrozenResult<Option<(&'arena K, &'arena V)>>
    where
        F: FnMut(&K) -> bool,
    {
        Ok(self
            .entries
            .as_slice(arena)?
            .iter()
            .find(|(key, _)| matches(key))
            .map(|(key, value)| (key, value)))
    }
}

/// A frozen set represented by immutable values.
#[derive(Clone, Copy)]
pub struct FrozenSet<T: FrozenValue> {
    pub(crate) values: FrozenVec<T>,
}

// SAFETY: the descriptor is Copy metadata over an immutable sequence.
unsafe impl<T: FrozenValue> FrozenValue for FrozenSet<T> {}

impl<T: FrozenValue> FrozenSet<T> {
    /// Return the number of frozen values.
    pub fn len(self) -> usize {
        self.values.len()
    }

    /// Return whether the set is empty.
    pub fn is_empty(self) -> bool {
        self.values.is_empty()
    }

    /// Borrow the frozen values.
    pub fn iter<'arena>(
        &self,
        arena: &'arena FrozenArena,
    ) -> FrozenResult<core::slice::Iter<'arena, T>> {
        Ok(self.values.as_slice(arena)?.iter())
    }

    /// Check membership using immutable linear lookup.
    pub fn contains(&self, value: &T, arena: &FrozenArena) -> FrozenResult<bool>
    where
        T: Eq,
    {
        Ok(self.values.as_slice(arena)?.contains(value))
    }

    /// Check membership with a caller-supplied comparison.
    pub fn contains_by<F>(&self, arena: &FrozenArena, matches: F) -> FrozenResult<bool>
    where
        F: FnMut(&T) -> bool,
    {
        Ok(self.values.as_slice(arena)?.iter().any(matches))
    }
}

/// A frozen operating-system string using the source target's compact code
/// units.
#[derive(Clone, Copy)]
pub struct FrozenOsString {
    pub(crate) bytes: FrozenBytes,
}

// SAFETY: this platform-specific descriptor contains only frozen offsets.
unsafe impl FrozenValue for FrozenOsString {}

impl FrozenOsString {
    /// Copy this value into a native OS string.
    pub fn to_os_string(&self, arena: &FrozenArena) -> FrozenResult<std::ffi::OsString> {
        let bytes = self.bytes.as_slice(arena)?;
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let mut native = Vec::new();
            native
                .try_reserve_exact(bytes.len())
                .map_err(FrozenError::Allocation)?;
            native.extend_from_slice(bytes);
            Ok(std::ffi::OsString::from_vec(native))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            if bytes.len() % 2 != 0 {
                return Err(FrozenError::InvalidHandle);
            }
            let mut units = Vec::new();
            units
                .try_reserve_exact(bytes.len() / 2)
                .map_err(FrozenError::Allocation)?;
            units.extend(
                bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]])),
            );
            Ok(std::ffi::OsString::from_wide(&units))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let mut encoded = Vec::new();
            encoded
                .try_reserve_exact(bytes.len())
                .map_err(FrozenError::Allocation)?;
            encoded.extend_from_slice(bytes);
            // SAFETY: FrozenOsString bytes can only originate from the
            // platform encoding copied from a valid CompactOsStr.
            Ok(unsafe { std::ffi::OsString::from_encoded_bytes_unchecked(encoded) })
        }
    }
}

/// A frozen path buffer using platform-native code units.
#[derive(Clone, Copy)]
pub struct FrozenPathBuf {
    pub(crate) inner: FrozenOsString,
}

// SAFETY: the wrapper contains only an immutable platform-specific descriptor.
unsafe impl FrozenValue for FrozenPathBuf {}

impl FrozenPathBuf {
    /// Copy this value into a native path buffer.
    pub fn to_path_buf(&self, arena: &FrozenArena) -> FrozenResult<std::path::PathBuf> {
        Ok(std::path::PathBuf::from(self.inner.to_os_string(arena)?))
    }
}
