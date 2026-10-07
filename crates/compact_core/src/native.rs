//! Small native-reference construction helpers used after arena validation.

/// Form a shared native reference.
///
/// # Safety
///
/// `pointer` must be non-null, aligned, dereferenceable for one initialized
/// `T`, and exclusively protected from mutation for `'a`. Its storage must
/// remain alive for `'a`; the caller must have proven bounds, initialization,
/// aliasing, and backing stability.
pub(crate) unsafe fn reference<'a, T>(pointer: *const T) -> &'a T {
    // SAFETY: all pointer and lifetime requirements are the caller's contract.
    unsafe { &*pointer }
}

/// Form an exclusive native reference.
///
/// # Safety
///
/// `pointer` must be non-null, aligned, dereferenceable for one initialized
/// `T`, and uniquely accessible for `'a`. Its storage must remain alive for
/// `'a`; the caller must have proven bounds, initialization, aliasing, and
/// backing stability.
pub(crate) unsafe fn reference_mut<'a, T>(pointer: *mut T) -> &'a mut T {
    // SAFETY: all pointer and lifetime requirements are the caller's contract.
    unsafe { &mut *pointer }
}

/// Form a shared native slice.
///
/// # Safety
///
/// `pointer` must be non-null and aligned, and must refer to `len` initialized
/// contiguous `T` elements within one allocation. Their total byte length must
/// be no greater than `isize::MAX`; they must remain immutable and alive for
/// `'a`. The caller must have proven bounds and backing stability.
pub(crate) unsafe fn slice<'a, T>(pointer: *const T, len: usize) -> &'a [T] {
    // SAFETY: all slice validity requirements are the caller's contract.
    unsafe { core::slice::from_raw_parts(pointer, len) }
}

/// Form an exclusive native slice.
///
/// # Safety
///
/// `pointer` must be non-null and aligned, and must refer to `len` initialized
/// contiguous `T` elements within one allocation. Their total byte length must
/// be no greater than `isize::MAX`; they must be uniquely accessible and alive
/// for `'a`. The caller must have proven bounds and backing stability.
pub(crate) unsafe fn slice_mut<'a, T>(pointer: *mut T, len: usize) -> &'a mut [T] {
    // SAFETY: all slice validity requirements are the caller's contract.
    unsafe { core::slice::from_raw_parts_mut(pointer, len) }
}
