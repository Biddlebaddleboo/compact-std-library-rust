//! Explicit helpers for crossing native interfaces.

#![forbid(unsafe_op_in_unsafe_fn)]

use core::mem::ManuallyDrop;

/// A copied native byte allocation that can outlive its compact source.
///
/// Treat the fields as an opaque ownership token. Keep them unchanged and call
/// [`compact_std_ffi_bytes_free`] exactly once. Temporary synchronous views
/// should use `CompactBytes::with_ffi_bytes` or
/// `CompactString::with_ffi_bytes` instead.
#[repr(C)]
#[derive(Debug)]
pub struct FfiByteBuffer {
    /// Native allocation pointer. It is non-null, including for empty buffers.
    ptr: *mut u8,
    /// Initialized byte count.
    len: usize,
    /// Original `Vec` capacity required by the matching free function.
    capacity: usize,
}

impl FfiByteBuffer {
    /// Copy bytes into a native `Vec` allocation suitable for transfer to C,
    /// JNI, or Swift.
    pub fn copy_from_slice(bytes: &[u8]) -> Result<Self, std::collections::TryReserveError> {
        let mut owned = Vec::new();
        owned.try_reserve_exact(bytes.len())?;
        owned.extend_from_slice(bytes);
        let owned = ManuallyDrop::new(owned);
        Ok(Self {
            ptr: owned.as_ptr().cast_mut(),
            len: owned.len(),
            capacity: owned.capacity(),
        })
    }

    /// Borrow this allocation's bytes in Rust.
    ///
    /// The borrow lasts only as long as this ownership token remains alive.
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: construction stores a live Vec allocation with `len` fully
        // initialized bytes; this token cannot be cloned and does not free it
        // until the explicit unsafe free operation.
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Return the native allocation pointer.
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr.cast_const()
    }

    /// Return the initialized byte count.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return whether this buffer is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Reclaim the native allocation represented by this token.
    ///
    /// # Safety
    ///
    /// `self` must be the unchanged value returned by
    /// [`FfiByteBuffer::copy_from_slice`], and no copy of its raw fields may
    /// have already been freed. The pointer, length, and capacity must still
    /// describe the original allocation.
    pub unsafe fn free(self) {
        // SAFETY: required by this method's contract; reconstruction uses the
        // exact pointer, length, and capacity emitted from the original Vec.
        drop(unsafe { Vec::from_raw_parts(self.ptr, self.len, self.capacity) });
    }
}

/// Free a copied byte buffer previously returned by
/// [`FfiByteBuffer::copy_from_slice`].
///
/// # Safety
///
/// The argument must be unchanged from the value returned by
/// [`FfiByteBuffer::copy_from_slice`] and must not have been freed already.
#[no_mangle]
pub unsafe extern "C" fn compact_std_ffi_bytes_free(buffer: FfiByteBuffer) {
    // SAFETY: this function forwards its documented ownership preconditions.
    unsafe { buffer.free() };
}

#[cfg(test)]
mod tests {
    use super::{compact_std_ffi_bytes_free, FfiByteBuffer};

    #[test]
    fn exported_buffer_owns_a_native_copy_until_the_matching_free() {
        let buffer = FfiByteBuffer::copy_from_slice(b"native boundary").unwrap();
        assert_eq!(buffer.as_slice(), b"native boundary");
        assert_eq!(buffer.as_ptr(), buffer.as_slice().as_ptr());
        assert_eq!(buffer.len(), 15);
        // SAFETY: this is the original, unchanged token and it is freed once.
        unsafe { compact_std_ffi_bytes_free(buffer) };
    }

    #[test]
    fn empty_exported_buffer_has_a_valid_zero_length_allocation() {
        let buffer = FfiByteBuffer::copy_from_slice(b"").unwrap();
        assert!(buffer.is_empty());
        assert!(buffer.as_slice().is_empty());
        // SAFETY: this is the original, unchanged token and it is freed once.
        unsafe { compact_std_ffi_bytes_free(buffer) };
    }
}
