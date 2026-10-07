//! Compact UTF-8 string with a twelve-byte inline payload.

use compact_core::{Arena, ArenaAllocation, CompactValue};
use core::borrow::Borrow;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::Deref;

use crate::{CollectionError, Result};

const INLINE_CAPACITY: usize = 12;

enum StringRepr<'arena> {
    Inline {
        len: u8,
        bytes: [u8; INLINE_CAPACITY],
    },
    Heap(ArenaAllocation<'arena, u8>),
}

/// An arena-backed UTF-8 string with twelve inline bytes.
///
/// Inline strings allocate nothing. Long strings own a reclaimable byte
/// allocation; growth preserves the old string if replacement allocation
/// fails, and dropping the string releases heap storage.
pub struct CompactString<'arena> {
    repr: StringRepr<'arena>,
}

impl<'arena> CompactString<'arena> {
    /// Construct an empty inline string tied to `arena`.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        Self::empty()
    }

    /// Construct an empty inline string without arena allocation.
    pub const fn empty() -> Self {
        Self {
            repr: StringRepr::Inline {
                len: 0,
                bytes: [0; INLINE_CAPACITY],
            },
        }
    }

    /// Copy a UTF-8 string into compact arena storage.
    pub fn from_str_in(value: &str, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        if value.len() <= INLINE_CAPACITY {
            let mut bytes = [0; INLINE_CAPACITY];
            bytes[..value.len()].copy_from_slice(value.as_bytes());
            return Ok(Self {
                repr: StringRepr::Inline {
                    len: value.len() as u8,
                    bytes,
                },
            });
        }
        let mut allocation = arena.alloc_owned_slice::<u8>(value.len())?;
        allocation.extend_copy(value.as_bytes())?;
        Ok(Self {
            repr: StringRepr::Heap(allocation),
        })
    }

    /// Return the UTF-8 byte length.
    pub fn len(&self) -> usize {
        match &self.repr {
            StringRepr::Inline { len, .. } => *len as usize,
            StringRepr::Heap(allocation) => allocation.len(),
        }
    }

    /// Return whether the string is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return the current storage capacity in bytes.
    pub fn capacity(&self) -> usize {
        match &self.repr {
            StringRepr::Inline { .. } => INLINE_CAPACITY,
            StringRepr::Heap(allocation) => allocation.capacity(),
        }
    }

    /// Borrow the exact initialized UTF-8 bytes.
    pub fn as_bytes<'view>(&'view self, arena: &'view Arena<'arena, '_>) -> Result<&'view [u8]> {
        match &self.repr {
            StringRepr::Inline { len, bytes } => Ok(&bytes[..*len as usize]),
            StringRepr::Heap(allocation) => {
                arena.validate_owned(allocation)?;
                Ok(allocation.as_slice())
            }
        }
    }

    /// Borrow the string as a zero-copy native `&str`.
    pub fn as_str<'view>(&'view self, arena: &'view Arena<'arena, '_>) -> Result<&'view str> {
        core::str::from_utf8(self.as_bytes(arena)?).map_err(|_| CollectionError::InvalidUtf8)
    }

    /// Append UTF-8 text, preserving the old value if arena allocation fails.
    pub fn push_str_in(&mut self, value: &str, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if value.is_empty() {
            return Ok(());
        }
        let old_len = self.len();
        let required = old_len
            .checked_add(value.len())
            .ok_or(CollectionError::CapacityOverflow)?;

        if let StringRepr::Inline { len, bytes } = &mut self.repr {
            if required <= INLINE_CAPACITY {
                bytes[old_len..required].copy_from_slice(value.as_bytes());
                *len = required as u8;
                return Ok(());
            }
        }

        if let StringRepr::Heap(allocation) = &mut self.repr {
            arena.validate_owned(allocation)?;
            if required <= allocation.capacity() {
                allocation.extend_copy(value.as_bytes())?;
                return Ok(());
            }
            let new_capacity = required.max(allocation.capacity().saturating_mul(2).max(16));
            if arena.try_resize_owned(allocation, new_capacity)? {
                allocation.extend_copy(value.as_bytes())?;
                return Ok(());
            }
        }

        let new_capacity = required.max(self.capacity().saturating_mul(2).max(16));
        let mut replacement = arena.alloc_owned_slice::<u8>(new_capacity)?;
        match &self.repr {
            StringRepr::Inline { len, bytes } => {
                replacement.extend_copy(&bytes[..*len as usize])?;
            }
            StringRepr::Heap(allocation) => {
                replacement.extend_copy(allocation.as_slice())?;
            }
        }
        replacement.extend_copy(value.as_bytes())?;
        self.repr = StringRepr::Heap(replacement);
        Ok(())
    }

    /// Append one Unicode scalar value.
    pub fn push_char_in(&mut self, value: char, arena: &mut Arena<'arena, '_>) -> Result<()> {
        let mut encoded = [0_u8; 4];
        self.push_str_in(value.encode_utf8(&mut encoded), arena)
    }

    /// Clear the string while retaining any heap allocation for reuse.
    pub fn clear(&mut self) {
        match &mut self.repr {
            StringRepr::Inline { len, .. } => *len = 0,
            StringRepr::Heap(allocation) => allocation.truncate(0),
        }
    }

    /// Truncate at a UTF-8 character boundary.
    pub fn truncate_in(&mut self, new_len: usize, arena: &Arena<'arena, '_>) -> Result<()> {
        if new_len >= self.len() {
            return Ok(());
        }
        let value = self.as_str(arena)?;
        if !value.is_char_boundary(new_len) {
            return Err(CollectionError::Core(compact_core::Error::OutOfBounds));
        }
        if new_len <= INLINE_CAPACITY {
            let mut bytes = [0; INLINE_CAPACITY];
            bytes[..new_len].copy_from_slice(&value.as_bytes()[..new_len]);
            self.repr = StringRepr::Inline {
                len: new_len as u8,
                bytes,
            };
            return Ok(());
        }
        if let StringRepr::Heap(allocation) = &mut self.repr {
            allocation.truncate(new_len);
        }
        Ok(())
    }

    /// Release unused heap capacity while retaining the string contents.
    pub fn shrink_to_fit_in(&mut self, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if self.len() <= INLINE_CAPACITY {
            if let StringRepr::Heap(allocation) = &self.repr {
                arena.validate_owned(allocation)?;
                let mut bytes = [0; INLINE_CAPACITY];
                bytes[..allocation.len()].copy_from_slice(allocation.as_slice());
                let len = allocation.len() as u8;
                self.repr = StringRepr::Inline { len, bytes };
            }
            return Ok(());
        }
        let StringRepr::Heap(allocation) = &mut self.repr else {
            return Ok(());
        };
        arena.validate_owned(allocation)?;
        let len = allocation.len();
        if arena.try_resize_owned(allocation, len)? {
            return Ok(());
        }
        let mut replacement = arena.alloc_owned_slice::<u8>(len)?;
        replacement.extend_copy(allocation.as_slice())?;
        self.repr = StringRepr::Heap(replacement);
        Ok(())
    }

    /// Compare with an ordinary native string.
    pub fn eq_str(&self, other: &str, arena: &Arena<'arena, '_>) -> Result<bool> {
        Ok(self.as_str(arena)? == other)
    }

    /// Create a formatting writer that grows this string through `arena`.
    pub fn writer<'view, 'backing>(
        &'view mut self,
        arena: &'view mut Arena<'arena, 'backing>,
    ) -> CompactStringWriter<'view, 'arena, 'backing> {
        CompactStringWriter {
            text: self,
            arena,
            error: None,
        }
    }
}

/// A formatting adapter that appends to a [`CompactString`] using an explicit
/// arena for any required growth.
pub struct CompactStringWriter<'view, 'arena, 'backing> {
    text: &'view mut CompactString<'arena>,
    arena: &'view mut Arena<'arena, 'backing>,
    error: Option<CollectionError>,
}

impl CompactStringWriter<'_, '_, '_> {
    /// Append UTF-8 text and preserve the arena allocation error.
    pub fn write_str_in(&mut self, value: &str) -> Result<()> {
        self.text.push_str_in(value, self.arena)
    }

    /// Append one Unicode scalar value and preserve the arena allocation error.
    pub fn write_char_in(&mut self, value: char) -> Result<()> {
        self.text.push_char_in(value, self.arena)
    }

    /// Format values into the compact string and preserve arena errors.
    ///
    /// A `fmt::Display` implementation that returns `fmt::Error` without an
    /// arena failure follows the behavior of standard formatting and panics.
    pub fn write_fmt_in(&mut self, arguments: fmt::Arguments<'_>) -> Result<()> {
        self.error = None;
        match fmt::write(self, arguments) {
            Ok(()) => Ok(()),
            Err(_) => match self.error.take() {
                Some(error) => Err(error),
                None => panic!("a formatter returned an error"),
            },
        }
    }
}

impl fmt::Write for CompactStringWriter<'_, '_, '_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.write_str_in(value).map_err(|error| {
            self.error = Some(error);
            fmt::Error
        })
    }
}

impl Deref for CompactString<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        let bytes = match &self.repr {
            StringRepr::Inline { len, bytes } => &bytes[..*len as usize],
            StringRepr::Heap(allocation) => allocation.as_slice(),
        };
        // SAFETY: all constructors accept `str`, append preserves UTF-8, and
        // truncation checks character boundaries before changing the prefix.
        unsafe { core::str::from_utf8_unchecked(bytes) }
    }
}

impl AsRef<str> for CompactString<'_> {
    fn as_ref(&self) -> &str {
        self
    }
}

impl Borrow<str> for CompactString<'_> {
    fn borrow(&self) -> &str {
        self
    }
}

impl fmt::Display for CompactString<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, formatter)
    }
}

impl fmt::Debug for CompactString<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

impl PartialEq for CompactString<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl Eq for CompactString<'_> {}

impl PartialEq<str> for CompactString<'_> {
    fn eq(&self, other: &str) -> bool {
        self.deref() == other
    }
}

impl PartialEq<&str> for CompactString<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.deref() == *other
    }
}

impl PartialOrd for CompactString<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CompactString<'_> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.deref().cmp(other.deref())
    }
}

impl Hash for CompactString<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.deref().hash(state);
    }
}

// SAFETY: the inline bytes move with the wrapper and heap storage is a unique
// ArenaAllocation token whose bytes have no address-sensitive state.
unsafe impl CompactValue for CompactString<'_> {}
