//! Immutable graphs stored in one process-cage allocation.

#![forbid(unsafe_op_in_unsafe_fn)]

mod storage;

pub use storage::{
    FrozenBuilder, FrozenBytes, FrozenError, FrozenGraph, FrozenGraphView, FrozenMap,
    FrozenOsString, FrozenPathBuf, FrozenResult, FrozenSet, FrozenString, FrozenValue, FrozenVec,
};
