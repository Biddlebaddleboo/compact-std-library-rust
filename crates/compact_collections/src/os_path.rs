//! Platform-aware compact operating-system strings and paths.

use compact_core::{Arena, CompactValue};
use core::fmt;
use core::ops::Range;
use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

#[cfg(windows)]
use crate::CollectionError;
use crate::{CompactBytes, Result};

/// A compact owner of an operating-system string.
///
/// Unix stores exact OS bytes. Windows stores each exact UTF-16 code unit in
/// little-endian form. The representation is local to this build and is not a
/// portable serialization format.
pub struct CompactOsString<'arena> {
    raw: CompactBytes<'arena>,
}

impl<'arena> CompactOsString<'arena> {
    /// Construct an empty inline OS string.
    pub const fn new() -> Self {
        Self {
            raw: CompactBytes::new(),
        }
    }

    /// Construct an empty inline OS string tied to `arena`.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        Self::new()
    }

    /// Copy an OS string into compact storage without requiring UTF-8.
    pub fn from_os_str(value: &OsStr, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        let raw = compact_os_bytes(value, arena)?;
        Ok(Self { raw })
    }

    /// Copy any native OS string type into compact storage.
    pub fn from<S: AsRef<OsStr>>(value: S, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Self::from_os_str(value.as_ref(), arena)
    }

    /// Consume an OS string and copy its exact platform representation.
    pub fn from_os_string(value: OsString, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Self::from_os_str(&value, arena)
    }

    /// Borrow this value as a compact OS-string view.
    pub fn as_os_str(&self) -> CompactOsStr<'_> {
        CompactOsStr {
            raw: self.raw.as_slice(),
        }
    }

    /// Convert to a native owned OS string without lossy conversion.
    pub fn to_os_string(&self) -> OsString {
        self.as_os_str().to_os_string()
    }

    /// Return the number of platform units: bytes on Unix and wide units on
    /// Windows.
    pub fn len(&self) -> usize {
        platform_unit_len(self.raw.as_slice())
    }

    /// Return whether the OS string is empty.
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Return the compact byte capacity used for this platform representation.
    pub fn compact_byte_capacity(&self) -> usize {
        self.raw.capacity()
    }

    /// Append an OS string, growing through `arena` when required.
    pub fn push(&mut self, value: &OsStr, arena: &mut Arena<'arena, '_>) -> Result<()> {
        append_os_str(&mut self.raw, value, arena)
    }

    /// Clear the string while retaining compact byte capacity.
    pub fn clear(&mut self) {
        self.raw.clear();
    }

    /// Return a lossy UTF-8 view for display and diagnostics.
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        self.as_os_str().to_string_lossy()
    }
}

impl Default for CompactOsString<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CompactOsString<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.to_os_string().fmt(formatter)
    }
}

// SAFETY: the compact byte owner is movable and carries all allocation
// ownership. Its platform units do not contain address-sensitive references.
unsafe impl CompactValue for CompactOsString<'_> {}

/// A borrowed view over an exact platform OS-string representation.
#[derive(Clone, Copy)]
pub struct CompactOsStr<'view> {
    raw: &'view [u8],
}

impl<'view> CompactOsStr<'view> {
    /// Convert to a native owned OS string without lossy conversion.
    pub fn to_os_string(self) -> OsString {
        native_os_string(self.raw)
    }

    /// Return the number of platform units: bytes on Unix and wide units on
    /// Windows.
    pub fn len(self) -> usize {
        platform_unit_len(self.raw)
    }

    /// Return whether the OS string is empty.
    pub fn is_empty(self) -> bool {
        self.raw.is_empty()
    }

    /// Return a lossy UTF-8 view for display and diagnostics.
    pub fn to_string_lossy(self) -> Cow<'view, str> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            OsStr::from_bytes(self.raw).to_string_lossy()
        }
        #[cfg(not(unix))]
        {
            Cow::Owned(self.to_os_string().to_string_lossy().into_owned())
        }
    }

    /// Borrow the raw bytes on Unix targets.
    #[cfg(unix)]
    pub fn as_bytes(self) -> &'view [u8] {
        self.raw
    }
}

impl fmt::Debug for CompactOsStr<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.to_os_string().fmt(formatter)
    }
}

impl PartialEq for CompactOsStr<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl Eq for CompactOsStr<'_> {}

/// A compact path owner with std-compatible path operations.
pub struct CompactPathBuf<'arena> {
    inner: CompactOsString<'arena>,
}

impl<'arena> CompactPathBuf<'arena> {
    /// Construct an empty compact path.
    pub const fn new() -> Self {
        Self {
            inner: CompactOsString::new(),
        }
    }

    /// Construct an empty compact path tied to `arena`.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        Self::new()
    }

    /// Copy a native path into compact storage.
    pub fn from_path(path: &Path, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Ok(Self {
            inner: CompactOsString::from_os_str(path.as_os_str(), arena)?,
        })
    }

    /// Copy any native path type into compact storage.
    pub fn from<P: AsRef<Path>>(path: P, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Self::from_path(path.as_ref(), arena)
    }

    /// Borrow this value as a compact path view.
    pub fn as_path(&self) -> CompactPath<'_> {
        CompactPath {
            raw: self.inner.raw.as_slice(),
        }
    }

    /// Borrow the underlying compact OS string.
    pub fn as_os_str(&self) -> CompactOsStr<'_> {
        self.inner.as_os_str()
    }

    /// Convert to a native owned path without lossy conversion.
    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(self.inner.to_os_string())
    }

    /// Return the parent as a borrowed compact path view.
    pub fn parent(&self) -> Option<CompactPath<'_>> {
        self.as_path().parent()
    }

    /// Return the final file name as a borrowed compact OS-string view.
    pub fn file_name(&self) -> Option<CompactOsStr<'_>> {
        self.as_path().file_name()
    }

    /// Return the final file stem as a borrowed compact OS-string view.
    pub fn file_stem(&self) -> Option<CompactOsStr<'_>> {
        self.as_path().file_stem()
    }

    /// Return the final file extension as a borrowed compact OS-string view.
    pub fn extension(&self) -> Option<CompactOsStr<'_>> {
        self.as_path().extension()
    }

    /// Iterate through platform path components in standard-library order.
    pub fn components(&self) -> CompactComponents<'_> {
        self.as_path().components()
    }

    /// Return whether the path is absolute.
    pub fn is_absolute(&self) -> bool {
        self.as_path().is_absolute()
    }

    /// Return whether the path is relative.
    pub fn is_relative(&self) -> bool {
        self.as_path().is_relative()
    }

    /// Return whether this path starts with `base` by standard components.
    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        self.as_path().starts_with(base)
    }

    /// Return whether this path ends with `child` by standard components.
    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        self.as_path().ends_with(child)
    }

    /// Return a display wrapper using standard lossy path formatting.
    pub fn display(&self) -> CompactPathDisplay {
        self.as_path().display()
    }

    /// Append `path` with the platform's standard path rules.
    pub fn push<P: AsRef<Path>>(&mut self, path: P, arena: &mut Arena<'arena, '_>) -> Result<()> {
        let mut native = self.to_path_buf();
        native.push(path);
        self.replace_from_path(&native, arena)
    }

    /// Remove the final path component, if present.
    pub fn pop(&mut self, arena: &mut Arena<'arena, '_>) -> Result<bool> {
        let mut native = self.to_path_buf();
        if !native.pop() {
            return Ok(false);
        }
        self.replace_from_path(&native, arena)?;
        Ok(true)
    }

    /// Set the final path component.
    pub fn set_file_name<S: AsRef<OsStr>>(
        &mut self,
        file_name: S,
        arena: &mut Arena<'arena, '_>,
    ) -> Result<()> {
        let mut native = self.to_path_buf();
        native.set_file_name(file_name);
        self.replace_from_path(&native, arena)
    }

    /// Set or remove the final path extension.
    pub fn set_extension<S: AsRef<OsStr>>(
        &mut self,
        extension: S,
        arena: &mut Arena<'arena, '_>,
    ) -> Result<bool> {
        let mut native = self.to_path_buf();
        let changed = native.set_extension(extension);
        if changed {
            self.replace_from_path(&native, arena)?;
        }
        Ok(changed)
    }

    /// Join `path` and return the result in compact storage.
    pub fn join<P: AsRef<Path>>(&self, path: P, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        let mut native = self.to_path_buf();
        native.push(path);
        Self::from_path(&native, arena)
    }

    fn replace_from_path(&mut self, path: &Path, arena: &mut Arena<'arena, '_>) -> Result<()> {
        let replacement = CompactOsString::from_os_str(path.as_os_str(), arena)?;
        self.inner = replacement;
        Ok(())
    }
}

impl Default for CompactPathBuf<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CompactPathBuf<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.to_path_buf().fmt(formatter)
    }
}

// SAFETY: path bytes are owned by the nested compact OS string and preserve
// the same move/drop guarantees.
unsafe impl CompactValue for CompactPathBuf<'_> {}

/// A borrowed compact path view.
#[derive(Clone, Copy)]
pub struct CompactPath<'view> {
    raw: &'view [u8],
}

impl<'view> CompactPath<'view> {
    /// Borrow this path as a compact OS-string view.
    pub fn as_os_str(self) -> CompactOsStr<'view> {
        CompactOsStr { raw: self.raw }
    }

    /// Convert to a native owned path without lossy conversion.
    pub fn to_path_buf(self) -> PathBuf {
        PathBuf::from(native_os_string(self.raw))
    }

    /// Return the parent as a borrowed compact path view.
    pub fn parent(self) -> Option<Self> {
        let parent_raw = self.with_native_path(|native| {
            native
                .parent()
                .map(|parent| encoded_os_str(parent.as_os_str()))
        })?;
        self.raw.starts_with(&parent_raw).then(|| Self {
            raw: &self.raw[..parent_raw.len()],
        })
    }

    /// Return the final file name as a borrowed compact OS-string view.
    pub fn file_name(self) -> Option<CompactOsStr<'view>> {
        let file_name = self.with_native_path(|native| native.file_name().map(encoded_os_str))?;
        self.find_last_encoded(&file_name)
    }

    /// Return the final file stem as a borrowed compact OS-string view.
    pub fn file_stem(self) -> Option<CompactOsStr<'view>> {
        let (name, stem) = self.with_native_path(|native| {
            Some((
                encoded_os_str(native.file_name()?),
                encoded_os_str(native.file_stem()?),
            ))
        })?;
        let name_range = find_last_subslice(self.raw, &name)?;
        let start = name_range.start;
        let end = start.checked_add(stem.len())?;
        (end <= name_range.end && self.raw[start..end] == stem).then(|| CompactOsStr {
            raw: &self.raw[start..end],
        })
    }

    /// Return the final file extension as a borrowed compact OS-string view.
    pub fn extension(self) -> Option<CompactOsStr<'view>> {
        let (name, extension) = self.with_native_path(|native| {
            Some((
                encoded_os_str(native.file_name()?),
                encoded_os_str(native.extension()?),
            ))
        })?;
        let name_range = find_last_subslice(self.raw, &name)?;
        let start = name_range.end.checked_sub(extension.len())?;
        (start >= name_range.start && self.raw[start..name_range.end] == extension).then(|| {
            CompactOsStr {
                raw: &self.raw[start..name_range.end],
            }
        })
    }

    /// Iterate through platform path components in standard-library order.
    pub fn components(self) -> CompactComponents<'view> {
        CompactComponents {
            path: self,
            next_component: 0,
            search_from: 0,
        }
    }

    /// Return whether the path is absolute.
    pub fn is_absolute(self) -> bool {
        self.with_native_path(Path::is_absolute)
    }

    /// Return whether the path is relative.
    pub fn is_relative(self) -> bool {
        self.with_native_path(Path::is_relative)
    }

    /// Return whether this path starts with `base` by standard components.
    pub fn starts_with<P: AsRef<Path>>(self, base: P) -> bool {
        self.with_native_path(|native| native.starts_with(base))
    }

    /// Return whether this path ends with `child` by standard components.
    pub fn ends_with<P: AsRef<Path>>(self, child: P) -> bool {
        self.with_native_path(|native| native.ends_with(child))
    }

    /// Return a display wrapper using standard lossy path formatting.
    pub fn display(self) -> CompactPathDisplay {
        CompactPathDisplay {
            path: self.to_path_buf(),
        }
    }

    fn find_last_encoded(self, encoded: &[u8]) -> Option<CompactOsStr<'view>> {
        let range = find_last_subslice(self.raw, encoded)?;
        Some(CompactOsStr {
            raw: &self.raw[range],
        })
    }

    fn with_native_path<R>(self, action: impl FnOnce(&Path) -> R) -> R {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            action(Path::new(OsStr::from_bytes(self.raw)))
        }
        #[cfg(not(unix))]
        {
            let native = self.to_path_buf();
            action(&native)
        }
    }
}

/// A compact path component kind, matching the categories in
/// [`std::path::Component`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompactComponentKind {
    /// A Windows drive or UNC prefix.
    Prefix,
    /// A root separator.
    RootDir,
    /// A current-directory component.
    CurDir,
    /// A parent-directory component.
    ParentDir,
    /// A normal path component.
    Normal,
}

/// A borrowed compact path component.
#[derive(Clone, Copy)]
pub struct CompactComponent<'view> {
    kind: CompactComponentKind,
    value: CompactOsStr<'view>,
}

impl<'view> CompactComponent<'view> {
    /// Return the component category.
    pub const fn kind(self) -> CompactComponentKind {
        self.kind
    }

    /// Return the component value.
    pub const fn as_os_str(self) -> CompactOsStr<'view> {
        self.value
    }
}

/// An iterator over borrowed compact path components.
pub struct CompactComponents<'view> {
    path: CompactPath<'view>,
    next_component: usize,
    search_from: usize,
}

impl<'view> Iterator for CompactComponents<'view> {
    type Item = CompactComponent<'view>;

    fn next(&mut self) -> Option<Self::Item> {
        let (kind, encoded) = self.path.with_native_path(|native| {
            let component = native.components().nth(self.next_component)?;
            let kind = match component {
                Component::Prefix(_) => CompactComponentKind::Prefix,
                Component::RootDir => CompactComponentKind::RootDir,
                Component::CurDir => CompactComponentKind::CurDir,
                Component::ParentDir => CompactComponentKind::ParentDir,
                Component::Normal(_) => CompactComponentKind::Normal,
            };
            Some((kind, encoded_os_str(component.as_os_str())))
        })?;
        let range = find_subslice_from(self.path.raw, &encoded, self.search_from)?;
        self.next_component += 1;
        self.search_from = range.end;
        Some(CompactComponent {
            kind,
            value: CompactOsStr {
                raw: &self.path.raw[range],
            },
        })
    }
}

/// A display wrapper that owns a native temporary path for formatting.
pub struct CompactPathDisplay {
    path: PathBuf,
}

impl fmt::Display for CompactPathDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.path.display().fmt(formatter)
    }
}

fn compact_os_bytes<'arena>(
    value: &OsStr,
    arena: &mut Arena<'arena, '_>,
) -> Result<CompactBytes<'arena>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        CompactBytes::from_slice_in(value.as_bytes(), arena)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let byte_len = value
            .encode_wide()
            .count()
            .checked_mul(2)
            .ok_or(CollectionError::CapacityOverflow)?;
        let mut bytes = CompactBytes::with_capacity_in(byte_len, arena)?;
        for unit in value.encode_wide() {
            bytes.extend_from_slice_in(&unit.to_le_bytes(), arena)?;
        }
        Ok(bytes)
    }
    #[cfg(not(any(unix, windows)))]
    {
        CompactBytes::from_slice_in(value.as_encoded_bytes(), arena)
    }
}

fn append_os_str<'arena, 'memory>(
    destination: &mut CompactBytes<'arena>,
    value: &OsStr,
    arena: &mut Arena<'arena, 'memory>,
) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        destination.extend_from_slice_in(value.as_bytes(), arena)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let byte_len = value
            .encode_wide()
            .count()
            .checked_mul(2)
            .ok_or(CollectionError::CapacityOverflow)?;
        destination.reserve_in(byte_len, arena)?;
        for unit in value.encode_wide() {
            destination
                .extend_from_slice_in(&unit.to_le_bytes(), arena)
                .expect("reserved OS-string capacity covers all wide units");
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        destination.extend_from_slice_in(value.as_encoded_bytes(), arena)
    }
}

fn native_os_string(raw: &[u8]) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(raw.to_vec())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        debug_assert_eq!(raw.len() % 2, 0);
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        OsString::from_wide(&units)
    }
    #[cfg(not(any(unix, windows)))]
    {
        // SAFETY: fallback bytes are copied only from this target's
        // `OsStr::as_encoded_bytes` representation and remain unchanged.
        unsafe { OsString::from_encoded_bytes_unchecked(raw.to_vec()) }
    }
}

fn encoded_os_str(value: &OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        value.as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        value.encode_wide().flat_map(u16::to_le_bytes).collect()
    }
    #[cfg(not(any(unix, windows)))]
    {
        value.as_encoded_bytes().to_vec()
    }
}

fn platform_unit_len(raw: &[u8]) -> usize {
    #[cfg(windows)]
    {
        debug_assert_eq!(raw.len() % 2, 0);
        raw.len() / 2
    }
    #[cfg(not(windows))]
    {
        raw.len()
    }
}

fn find_last_subslice(haystack: &[u8], needle: &[u8]) -> Option<Range<usize>> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let step = if cfg!(windows) { 2 } else { 1 };
    (0..=haystack.len() - needle.len())
        .rev()
        .filter(|index| index % step == 0)
        .find(|index| haystack[*index..*index + needle.len()] == *needle)
        .map(|start| start..start + needle.len())
}

fn find_subslice_from(haystack: &[u8], needle: &[u8], from: usize) -> Option<Range<usize>> {
    if needle.is_empty() || needle.len() > haystack.len() || from > haystack.len() {
        return None;
    }
    let step = if cfg!(windows) { 2 } else { 1 };
    let start = from + (step - from % step) % step;
    (start..=haystack.len() - needle.len())
        .step_by(step)
        .find(|index| haystack[*index..*index + needle.len()] == *needle)
        .map(|start| start..start + needle.len())
}
