#![no_std]
#![forbid(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)]
//! Backend-independent compact cage representation and packing primitives.

mod abi;
mod allocation;
mod bytes;
mod error;
mod layout;
mod offset;
mod packed;

pub use abi::{
    CompactAbiVersion, ABI_VERSION, MAX_CAGE_BYTES, MIN_CAGE_BYTES, NULL_OFFSET, OFFSET_WIDTH_BYTES,
};
pub use allocation::CompactValue;
pub use bytes::ByteRange32;
pub use error::{Error, Result};
pub use layout::{bits_required, checked_align_up, smallest_word, StorageWord};
pub use offset::{Offset32, OffsetSlice32};
pub use packed::{read_bits, validate_bit_range, write_bits, BitField, PackedWord};

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    #[test]
    fn cage_descriptors_keep_their_compact_layout() {
        assert_eq!(size_of::<Offset32<u64>>(), 4);
        assert_eq!(size_of::<OffsetSlice32<u64>>(), 8);
        assert_eq!(size_of::<ByteRange32>(), 8);
    }

    #[test]
    fn null_offset_is_reserved() {
        let offset = Offset32::<u8>::null();
        assert!(offset.is_null());
        assert_eq!(offset.as_u32(), NULL_OFFSET);
    }

    #[test]
    fn checked_layout_helpers_cover_boundaries() {
        assert_eq!(checked_align_up(17, 8).unwrap(), 24);
        assert_eq!(bits_required(0), 0);
        assert_eq!(bits_required(255), 8);
    }
}
