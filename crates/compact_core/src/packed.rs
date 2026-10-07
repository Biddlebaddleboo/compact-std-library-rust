//! LSB-first bit fields over practical scalar backing words.

use crate::{Error, Result};

/// Read up to 64 LSB-first bits from an initialized byte range.
///
/// Bit zero is the least significant bit of `bytes[0]`. Fields may cross byte
/// boundaries. A zero-width field reads as zero when `bit_offset` is in the
/// inclusive range `0..=bytes.len() * 8`.
#[inline]
pub fn read_bits(bytes: &[u8], bit_offset: usize, width: u8) -> Result<u64> {
    let total_bits = bytes.len().checked_mul(8).ok_or(Error::OffsetOverflow)?;
    validate_byte_bit_range(total_bits, bit_offset, width)?;
    if width == 0 {
        return Ok(0);
    }
    if width == 1 {
        return Ok(u64::from(
            bytes[bit_offset / 8] & (1 << (bit_offset % 8)) != 0,
        ));
    }
    let first_byte = bit_offset / 8;
    let intra = bit_offset % 8;
    let bytes_needed = (intra + width as usize).div_ceil(8);
    if bytes_needed == 1 {
        return Ok(((bytes[first_byte] >> intra) as u64) & low_mask(width));
    }
    if bytes_needed <= 8 {
        let word = load_lsb_span(&bytes[first_byte..first_byte + bytes_needed]);
        let mask = low_mask(width);
        return Ok((word >> intra) & mask);
    }

    // A 64-bit field at a nonzero intra-byte offset spans nine bytes. Keep a
    // simple fallback for this uncommon edge case rather than truncating it.
    let mut value = 0_u64;
    for index in 0..width as usize {
        let position = bit_offset + index;
        if bytes[position / 8] & (1 << (position % 8)) != 0 {
            value |= 1_u64 << index;
        }
    }
    Ok(value)
}

/// Write up to 64 LSB-first bits to an initialized byte range, preserving
/// every bit outside the selected field.
#[inline]
pub fn write_bits(bytes: &mut [u8], bit_offset: usize, width: u8, value: u64) -> Result<()> {
    let total_bits = bytes.len().checked_mul(8).ok_or(Error::OffsetOverflow)?;
    validate_byte_bit_range(total_bits, bit_offset, width)?;
    let value_mask = low_mask(width);
    if value & !value_mask != 0 {
        return Err(Error::ValueDoesNotFit);
    }
    if width == 0 {
        return Ok(());
    }
    if width == 1 {
        let mask = 1_u8 << (bit_offset % 8);
        let byte = &mut bytes[bit_offset / 8];
        if value == 0 {
            *byte &= !mask;
        } else {
            *byte |= mask;
        }
        return Ok(());
    }
    let first_byte = bit_offset / 8;
    let intra = bit_offset % 8;
    let bytes_needed = (intra + width as usize).div_ceil(8);
    if bytes_needed == 1 {
        let mask = (value_mask << intra) as u8;
        let byte = &mut bytes[first_byte];
        *byte = (*byte & !mask) | (((value << intra) as u8) & mask);
        return Ok(());
    }
    if bytes_needed <= 8 {
        let span = &mut bytes[first_byte..first_byte + bytes_needed];
        let old = load_lsb_span(span);
        let field_mask = value_mask << intra;
        let updated = (old & !field_mask) | ((value << intra) & field_mask);
        store_lsb_span(span, updated);
        return Ok(());
    }

    // A 64-bit field starting mid-byte spans nine bytes; preserve the existing
    // correct fallback for that shape.
    for index in 0..width as usize {
        let position = bit_offset + index;
        let mask = 1_u8 << (position % 8);
        let byte = &mut bytes[position / 8];
        if value & (1_u64 << index) == 0 {
            *byte &= !mask;
        } else {
            *byte |= mask;
        }
    }
    Ok(())
}

#[inline]
fn low_mask(width: u8) -> u64 {
    match width {
        0 => 0,
        64 => u64::MAX,
        _ => (1_u64 << width) - 1,
    }
}

#[inline]
fn load_lsb_span(bytes: &[u8]) -> u64 {
    debug_assert!(bytes.len() <= 8);
    let mut native = [0_u8; 8];
    #[cfg(target_endian = "little")]
    native[..bytes.len()].copy_from_slice(bytes);
    #[cfg(target_endian = "big")]
    for (index, byte) in bytes.iter().rev().enumerate() {
        native[8 - bytes.len() + index] = *byte;
    }
    u64::from_ne_bytes(native)
}

#[inline]
fn store_lsb_span(bytes: &mut [u8], value: u64) {
    debug_assert!(bytes.len() <= 8);
    let native = value.to_ne_bytes();
    #[cfg(target_endian = "little")]
    bytes.copy_from_slice(&native[..bytes.len()]);
    #[cfg(target_endian = "big")]
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = native[7 - index];
    }
}

fn validate_byte_bit_range(total_bits: usize, offset: usize, width: u8) -> Result<()> {
    if width > 64 {
        return Err(Error::InvalidBitRange);
    }
    let end = offset
        .checked_add(width as usize)
        .ok_or(Error::OffsetOverflow)?;
    if end > total_bits {
        return Err(Error::InvalidBitRange);
    }
    Ok(())
}

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
