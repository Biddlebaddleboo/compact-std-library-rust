//! Compact ownership wrappers and collections for [`compact_core`] arenas.
//!
//! Generic owning containers require [`compact_core::CompactValue`], an unsafe
//! contract for values that can move between arena slots and be destroyed
//! while their arena backing remains alive. Owning allocation tokens reclaim
//! their storage on drop.

mod bitvec;
mod boxed;
mod enum_value;
mod error;
mod intern;
mod slab;
mod small;
mod string;
mod vec;

pub use bitvec::CompactBitVec;
pub use boxed::{CompactBox, CompactOption};
pub use enum_value::CompactEnum;
pub use error::{CollectionError, Result};
pub use intern::{CompactInterner, InternId};
pub use slab::{CompactSlab, SlabHandle};
pub use small::CompactSmallVec;
pub use string::CompactString;
pub use vec::CompactVec;
