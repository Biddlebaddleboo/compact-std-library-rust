//! Compact ownership wrappers and collections for [`compact_core`] arenas.
//!
//! Values stored by the generic containers implement `Copy`. This keeps the
//! arena's reset-at-scope-end model explicit and avoids silently skipping
//! native destructors. Compact strings and byte collections manage only
//! arena-relative byte allocations.

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
