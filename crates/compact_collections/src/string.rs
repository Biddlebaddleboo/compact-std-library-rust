//! Compact UTF-8 string with a twelve-byte inline payload.

use compact_backend_std::{CageAllocation, CompactRuntime};
use compact_core::{CompactValue, Error as CoreError};
use core::borrow::Borrow;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::Deref;

use crate::{CollectionError, Result};

const INLINE_CAPACITY: usize = 12;

enum StringRepr {
    Inline {
        len: u8,
        bytes: [u8; INLINE_CAPACITY],
    },
    Heap(CageAllocation<u8>),
}

/// A compact UTF-8 string with twelve inline bytes and a four-byte heap owner.
pub struct CompactString {
    repr: StringRepr,
}

impl CompactString {
    /// Construct an empty inline string.
    pub const fn new() -> Self {
        Self::empty()
    }
    /// Construct an empty inline string.
    pub const fn empty() -> Self {
        Self {
            repr: StringRepr::Inline {
                len: 0,
                bytes: [0; INLINE_CAPACITY],
            },
        }
    }
    /// Copy UTF-8 text into compact storage.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
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
        let mut allocation = CompactRuntime::alloc_owned_slice::<u8>(value.len())?;
        allocation.extend_copy(value.as_bytes())?;
        Ok(Self {
            repr: StringRepr::Heap(allocation),
        })
    }
    /// Return the UTF-8 byte length.
    pub fn len(&self) -> usize {
        match &self.repr {
            StringRepr::Inline { len, .. } => *len as usize,
            StringRepr::Heap(a) => a.len(),
        }
    }
    /// Return whether the string is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Return current storage capacity in bytes.
    pub fn capacity(&self) -> usize {
        match &self.repr {
            StringRepr::Inline { .. } => INLINE_CAPACITY,
            StringRepr::Heap(a) => a.capacity(),
        }
    }
    /// Borrow the exact UTF-8 bytes.
    pub fn as_bytes(&self) -> &[u8] {
        match &self.repr {
            StringRepr::Inline { len, bytes } => &bytes[..*len as usize],
            StringRepr::Heap(a) => a.as_slice(),
        }
    }
    /// Borrow the string as a native `&str`.
    pub fn as_str(&self) -> &str {
        // SAFETY: constructors copy from `str`; mutation appends UTF-8 and truncates on character boundaries.
        unsafe { core::str::from_utf8_unchecked(self.as_bytes()) }
    }
    /// Return a pointer valid until this string is mutated or dropped.
    pub fn as_ptr(&self) -> *const u8 {
        self.as_bytes().as_ptr()
    }
    /// Expose the UTF-8 bytes for a synchronous native call.
    pub fn with_ffi_bytes<R>(&self, call: impl FnOnce(&[u8]) -> R) -> R {
        call(self.as_bytes())
    }
    /// Append UTF-8 text. Allocation failure leaves the prior string intact.
    pub fn push_str(&mut self, value: &str) -> Result<()> {
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
            if required <= allocation.capacity() {
                allocation.extend_copy(value.as_bytes())?;
                return Ok(());
            }
            let cap = required.max(allocation.capacity().saturating_mul(2).max(16));
            if allocation.try_resize(cap)? {
                allocation.extend_copy(value.as_bytes())?;
                return Ok(());
            }
        }
        let new_capacity = required.max(self.capacity().saturating_mul(2).max(16));
        let mut replacement = CompactRuntime::alloc_owned_slice::<u8>(new_capacity)?;
        replacement.extend_copy(self.as_bytes())?;
        replacement.extend_copy(value.as_bytes())?;
        self.repr = StringRepr::Heap(replacement);
        Ok(())
    }
    /// Append one Unicode scalar value.
    pub fn push_char(&mut self, value: char) -> Result<()> {
        let mut bytes = [0; 4];
        self.push_str(value.encode_utf8(&mut bytes))
    }
    /// Clear while retaining any heap allocation.
    pub fn clear(&mut self) {
        match &mut self.repr {
            StringRepr::Inline { len, .. } => *len = 0,
            StringRepr::Heap(a) => a.truncate(0),
        }
    }
    /// Truncate at a UTF-8 character boundary.
    pub fn truncate(&mut self, new_len: usize) -> Result<()> {
        if new_len >= self.len() {
            return Ok(());
        }
        if !self.as_str().is_char_boundary(new_len) {
            return Err(CollectionError::Core(CoreError::OutOfBounds));
        }
        if new_len <= INLINE_CAPACITY {
            let mut bytes = [0; INLINE_CAPACITY];
            bytes[..new_len].copy_from_slice(&self.as_bytes()[..new_len]);
            self.repr = StringRepr::Inline {
                len: new_len as u8,
                bytes,
            };
        } else if let StringRepr::Heap(a) = &mut self.repr {
            a.truncate(new_len);
        }
        Ok(())
    }
    /// Release unused heap capacity, moving short contents back inline.
    pub fn shrink_to_fit(&mut self) -> Result<()> {
        if self.len() <= INLINE_CAPACITY {
            if let StringRepr::Heap(a) = &self.repr {
                let len = a.len();
                let mut bytes = [0; INLINE_CAPACITY];
                bytes[..len].copy_from_slice(a.as_slice());
                self.repr = StringRepr::Inline {
                    len: len as u8,
                    bytes,
                };
            }
            return Ok(());
        }
        let StringRepr::Heap(allocation) = &mut self.repr else {
            return Ok(());
        };
        let len = allocation.len();
        if allocation.try_resize(len)? {
            return Ok(());
        }
        let mut replacement = CompactRuntime::alloc_owned_slice::<u8>(len)?;
        replacement.extend_copy(allocation.as_slice())?;
        self.repr = StringRepr::Heap(replacement);
        Ok(())
    }
    /// Compare with an ordinary native string.
    pub fn eq_str(&self, other: &str) -> bool {
        self.as_str() == other
    }
    /// Create a formatting writer that appends to this string.
    pub fn writer(&mut self) -> CompactStringWriter<'_> {
        CompactStringWriter {
            text: self,
            error: None,
        }
    }
}

/// Formatting adapter for a compact string.
pub struct CompactStringWriter<'a> {
    text: &'a mut CompactString,
    error: Option<CollectionError>,
}

impl CompactStringWriter<'_> {
    /// Append text and preserve allocation errors.
    pub fn write_str(&mut self, value: &str) -> Result<()> {
        self.text.push_str(value)
    }
    /// Append a Unicode scalar value.
    pub fn write_char(&mut self, value: char) -> Result<()> {
        self.text.push_char(value)
    }
    /// Format values and preserve cage allocation errors.
    pub fn write_fmt(&mut self, arguments: fmt::Arguments<'_>) -> Result<()> {
        self.error = None;
        match fmt::write(self, arguments) {
            Ok(()) => Ok(()),
            Err(_) => match self.error.take() {
                Some(e) => Err(e),
                None => panic!("a formatter returned an error"),
            },
        }
    }
}
impl fmt::Write for CompactStringWriter<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.text.push_str(value).map_err(|e| {
            self.error = Some(e);
            fmt::Error
        })
    }
}
impl Deref for CompactString {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<str> for CompactString {
    fn as_ref(&self) -> &str {
        self
    }
}
impl Borrow<str> for CompactString {
    fn borrow(&self) -> &str {
        self
    }
}
impl fmt::Display for CompactString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl Default for CompactString {
    fn default() -> Self {
        Self::empty()
    }
}

impl core::str::FromStr for CompactString {
    type Err = crate::CollectionError;

    fn from_str(value: &str) -> Result<Self> {
        CompactString::from_str(value)
    }
}
impl fmt::Debug for CompactString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}
impl PartialEq for CompactString {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl Eq for CompactString {}
impl PartialEq<str> for CompactString {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl PartialEq<&str> for CompactString {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl PartialOrd for CompactString {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for CompactString {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}
impl Hash for CompactString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}
// SAFETY: inline bytes move with the string and heap bytes have one cage owner.
unsafe impl CompactValue for CompactString {}

impl<T: fmt::Display + ?Sized> crate::TryToCompactString for T {
    fn try_to_compact_string(&self) -> Result<CompactString> {
        let mut text = CompactString::new();
        text.writer().write_fmt(format_args!("{self}"))?;
        Ok(text)
    }
}
