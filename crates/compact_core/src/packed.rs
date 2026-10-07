//! LSB-first bit fields over practical scalar backing words.

use crate::{Error, Result};

mod sealed {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
    impl Sealed for u32 {}
    impl Sealed for u64 {}
}

/// A scalar word supported by the compact packed-field helpers.
pub trait PackedWord: sealed::Sealed + Copy {
    /// Width of the scalar in bits.
    const BITS: u8;

    /// Convert this word to its zero-extended numeric value.
    fn to_u64(self) -> u64;

    /// Convert a value already known to fit in this word.
    fn from_u64(value: u64) -> Self;
}

macro_rules! packed_word {
    ($ty:ty, $bits:expr) => {
        impl PackedWord for $ty {
            const BITS: u8 = $bits;

            fn to_u64(self) -> u64 {
                self as u64
            }

            fn from_u64(value: u64) -> Self {
                value as Self
            }
        }
    };
}

packed_word!(u8, 8);
packed_word!(u16, 16);
packed_word!(u32, 32);
packed_word!(u64, 64);

/// Validate a nonempty bit range against a scalar word width.
pub const fn validate_bit_range(offset: u8, width: u8, word_bits: u8) -> Result<()> {
    if width == 0 || word_bits == 0 || word_bits > 64 || offset >= word_bits {
        return Err(Error::InvalidBitRange);
    }
    match offset.checked_add(width) {
        Some(end) if end <= word_bits => Ok(()),
        _ => Err(Error::InvalidBitRange),
    }
}

/// A checked bit range inside a packed scalar word.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BitField {
    offset: u8,
    width: u8,
}

impl BitField {
    /// Create an LSB-first field at `offset` with `width` bits.
    pub const fn new(offset: u8, width: u8, word_bits: u8) -> Result<Self> {
        match validate_bit_range(offset, width, word_bits) {
            Ok(()) => Ok(Self { offset, width }),
            Err(error) => Err(error),
        }
    }

    /// Return the field's bit offset.
    pub const fn offset(self) -> u8 {
        self.offset
    }

    /// Return the field's width.
    pub const fn width(self) -> u8 {
        self.width
    }

    fn validate_for<W: PackedWord>(self) -> Result<()> {
        validate_bit_range(self.offset, self.width, W::BITS)
    }

    fn value_mask(self) -> u64 {
        if self.width == 64 {
            u64::MAX
        } else {
            (1_u64 << self.width) - 1
        }
    }

    /// Extract the field as a zero-extended `u64`.
    pub fn extract<W: PackedWord>(self, word: W) -> Result<u64> {
        self.validate_for::<W>()?;
        Ok((word.to_u64() >> self.offset) & self.value_mask())
    }

    /// Insert `value`, preserving every bit outside this field.
    pub fn insert<W: PackedWord>(self, word: W, value: u64) -> Result<W> {
        self.validate_for::<W>()?;
        let value_mask = self.value_mask();
        if value & !value_mask != 0 {
            return Err(Error::ValueDoesNotFit);
        }
        let shifted_mask = if self.width == 64 {
            u64::MAX
        } else {
            value_mask << self.offset
        };
        let updated = (word.to_u64() & !shifted_mask) | (value << self.offset);
        Ok(W::from_u64(updated))
    }

    /// Read a one-bit field as a boolean.
    pub fn read_bool<W: PackedWord>(self, word: W) -> Result<bool> {
        if self.width != 1 {
            return Err(Error::InvalidBitRange);
        }
        Ok(self.extract(word)? != 0)
    }

    /// Write a one-bit boolean while preserving neighboring bits.
    pub fn insert_bool<W: PackedWord>(self, word: W, value: bool) -> Result<W> {
        if self.width != 1 {
            return Err(Error::InvalidBitRange);
        }
        self.insert(word, u64::from(value))
    }
}
