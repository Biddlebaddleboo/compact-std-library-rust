//! Compact owning collections backed by the process-wide cage.

#![forbid(unsafe_op_in_unsafe_fn)]

mod bitvec;
mod boxed;
mod compact_bytes;
mod deque;
mod enum_value;
mod error;
mod hash_map;
mod intern;
mod os_path;
mod slab;
mod small;
mod string;
mod vec;

pub use bitvec::CompactBitVec;
pub use boxed::{CompactBox, CompactOption};
pub use compact_bytes::{CompactBytes, COMPACT_BYTES_INLINE_CAPACITY};
pub use deque::{CompactRing, CompactVecDeque, CompactVecDequeIter};
pub use enum_value::CompactEnum;
pub use error::{CollectionError, Result};
pub use hash_map::{
    CompactBuildHasher, CompactHashMap, CompactHashMapIter, CompactHashMapIterMut, CompactHashSet,
};
pub use intern::{CompactInterner, InternId};
pub use os_path::{
    CompactComponent, CompactComponentKind, CompactComponents, CompactOsStr, CompactOsString,
    CompactPath, CompactPathBuf, CompactPathDisplay,
};
pub use slab::{CompactSlab, SlabHandle};
pub use small::CompactSmallVec;
pub use string::{CompactString, CompactStringWriter};
pub use vec::CompactVec;

/// Fallible collection traits that do not hide cage allocation failure.
pub mod try_traits {
    use crate::{CompactString, Result};

    /// Fallibly clone a value into another compact owner.
    pub trait TryClone {
        type Cloned;
        fn try_clone(&self) -> Result<Self::Cloned>;
    }
    /// Fallibly extend a collection from an iterator.
    pub trait TryExtend<T> {
        fn try_extend<I: IntoIterator<Item = T>>(&mut self, iter: I) -> Result<()>;
    }
    /// Fallibly build a compact collection from an iterator.
    pub trait TryFromIterator<T>: Sized {
        fn try_from_iter<I: IntoIterator<Item = T>>(iter: I) -> Result<Self>;
    }
    /// Fallibly convert displayable text to a compact UTF-8 string.
    pub trait TryToCompactString {
        fn try_to_compact_string(&self) -> Result<CompactString>;
    }
}

pub use try_traits::{TryClone, TryExtend, TryFromIterator, TryToCompactString};

/// Internal helpers used by exported macros.
#[doc(hidden)]
pub mod __private {
    use crate::{CompactString, Result};
    use core::fmt;

    /// Format arguments into compact UTF-8 storage.
    pub fn format_args(arguments: fmt::Arguments<'_>) -> Result<CompactString> {
        let mut text = CompactString::new();
        text.writer().write_fmt(arguments)?;
        Ok(text)
    }
}

/// Format values directly into fallible cage-backed UTF-8 storage.
#[macro_export]
macro_rules! compact_format {
    ($($argument:tt)*) => {
        $crate::__private::format_args(::core::format_args!($($argument)*))
    };
}
