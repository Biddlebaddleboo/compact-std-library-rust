//! Immutable, shareable representations copied from a mutable compact arena.
//!
//! Frozen data is created only through trusted builders. The backing contains
//! no allocator state and exposes no mutable access after construction.

#![forbid(unsafe_op_in_unsafe_fn)]

mod convert;
mod storage;

pub use convert::{freeze_in, FreezeIn};
pub use storage::{
    FrozenArena, FrozenBuilder, FrozenBytes, FrozenError, FrozenMap, FrozenOsString, FrozenPathBuf,
    FrozenResult, FrozenRoot, FrozenSet, FrozenString, FrozenValue, FrozenVec, FrozenVecDeque,
};
