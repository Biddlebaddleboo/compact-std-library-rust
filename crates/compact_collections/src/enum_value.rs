//! Contract implemented by generated fieldless compact enums.

/// A fieldless enum with a compact, checked discriminant mapping.
pub trait CompactEnum: Sized {
    /// Number of bits required for every declared variant.
    const BITS: u8;

    /// Encode this declared variant as a compact integer.
    fn compact_bits(&self) -> u64;

    /// Decode a compact integer, returning `None` for undeclared values.
    fn from_compact_bits(value: u64) -> Option<Self>;
}
