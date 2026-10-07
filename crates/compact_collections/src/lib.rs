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
