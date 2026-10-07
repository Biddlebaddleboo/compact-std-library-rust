//! Checked scalar layout helpers for future compact layout generators.

use crate::{Error, Result};

/// The smallest supported scalar word that can hold a requested bit width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageWord {
    /// Eight-bit storage.
    U8,
    /// Sixteen-bit storage.
    U16,
    /// Thirty-two-bit storage.
    U32,
    /// Sixty-four-bit storage.
    U64,
}

impl StorageWord {
    /// Return the word width in bits.
    pub const fn bits(self) -> u8 {
        match self {
            Self::U8 => 8,
            Self::U16 => 16,
            Self::U32 => 32,
            Self::U64 => 64,
        }
    }

    /// Return the word width in bytes.
    pub const fn bytes(self) -> usize {
        (self.bits() / 8) as usize
    }
}

/// Return the minimum number of bits needed to represent every value in
/// `0..=maximum`.
///
/// By convention, a maximum of zero requires zero bits because the domain has
/// one constant value. A field that stores a boolean still uses one bit.
pub const fn bits_required(maximum: u64) -> u8 {
    if maximum == 0 {
        0
    } else {
        (u64::BITS - maximum.leading_zeros()) as u8
    }
}

/// Select the smallest supported standard word for `bits`.
pub const fn smallest_word(bits: u8) -> Result<StorageWord> {
    match bits {
        1..=8 => Ok(StorageWord::U8),
        9..=16 => Ok(StorageWord::U16),
        17..=32 => Ok(StorageWord::U32),
        33..=64 => Ok(StorageWord::U64),
        _ => Err(Error::InvalidBitRange),
    }
}

/// Round `value` up to a multiple of a nonzero power-of-two `alignment`.
pub const fn checked_align_up(value: usize, alignment: usize) -> Result<usize> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(Error::AlignmentError);
    }
    match value.checked_add(alignment - 1) {
        Some(rounded) => Ok(rounded & !(alignment - 1)),
        None => Err(Error::OffsetOverflow),
    }
}
