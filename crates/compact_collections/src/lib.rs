//! Compact ownership wrappers and collections for [`compact_core`] arenas.
//!
//! Generic owning containers require [`compact_core::CompactValue`], an unsafe
//! contract for values that can move between arena slots and be destroyed
//! while their arena backing remains alive. Owning allocation tokens reclaim
//! their storage on drop.

mod arena_traits;
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

pub use arena_traits::{CloneIn, ExtendIn, FromIteratorIn, ToCompactStringIn};
pub use bitvec::CompactBitVec;
pub use boxed::{CompactBox, CompactOption};
pub use compact_bytes::{CompactBytes, COMPACT_BYTES_INLINE_CAPACITY};
pub use deque::{CompactRing, CompactVecDeque, CompactVecDequeIter};
pub use enum_value::CompactEnum;
pub use error::{CollectionError, Result};
pub use hash_map::{
    CompactHashMap, CompactHashMapEntry, CompactHashMapIter, CompactHashMapIterMut, CompactHashSet,
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

/// Implementation details referenced by exported macros.
#[doc(hidden)]
pub mod __private {
    use compact_core::Arena;
    use core::fmt;

    use crate::{CompactString, Result};

    /// Format directly into a compact string with an explicit arena.
    pub fn format_args_in<'arena>(
        arena: &mut Arena<'arena, '_>,
        arguments: fmt::Arguments<'_>,
    ) -> Result<CompactString<'arena>> {
        let mut text = CompactString::empty();
        text.writer(arena).write_fmt_in(arguments)?;
        Ok(text)
    }
}

/// Format a value into arena-backed UTF-8 storage.
///
/// The result is fallible because the compact string grows through `arena`.
#[macro_export]
macro_rules! format_in {
    ($arena:expr, $($argument:tt)*) => {
        $crate::__private::format_args_in(
            $arena,
            ::core::format_args!($($argument)*),
        )
    };
}
