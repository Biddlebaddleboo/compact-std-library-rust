//! Platform-aware compact operating-system strings and paths.

use crate::{CompactBytes, Result};
use compact_core::CompactValue;
use core::fmt;
use core::ops::Range;
use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

/// A compact owner of an operating-system string.
pub struct CompactOsString {
    raw: CompactBytes,
}
impl CompactOsString {
    /// Construct an empty OS string.
    pub const fn new() -> Self {
        Self {
            raw: CompactBytes::new(),
        }
    }
    /// Copy an OS string into cage storage without requiring UTF-8.
    pub fn from_os_str(value: &OsStr) -> Result<Self> {
        Ok(Self {
            raw: compact_os_bytes(value)?,
        })
    }
    /// Copy any native OS string type into cage storage.
    pub fn from<S: AsRef<OsStr>>(value: S) -> Result<Self> {
        Self::from_os_str(value.as_ref())
    }
    /// Consume an OS string and copy its target representation.
    pub fn from_os_string(value: OsString) -> Result<Self> {
        Self::from_os_str(&value)
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
    /// Return platform units: bytes on Unix and UTF-16 units on Windows.
    pub fn len(&self) -> usize {
        platform_unit_len(self.raw.as_slice())
    }
    /// Return whether the string is empty.
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }
    /// Return compact byte capacity.
    pub fn compact_byte_capacity(&self) -> usize {
        self.raw.capacity()
    }
    /// Append an OS string.
    pub fn push(&mut self, value: &OsStr) -> Result<()> {
        append_os_str(&mut self.raw, value)
    }
    /// Clear while retaining compact byte capacity.
    pub fn clear(&mut self) {
        self.raw.clear();
    }
    /// Return a lossy UTF-8 view for display.
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        self.as_os_str().to_string_lossy()
    }
}
impl Default for CompactOsString {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Debug for CompactOsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.to_os_string().fmt(f)
    }
}
// SAFETY: the byte owner carries all allocation ownership and stores no native references.
unsafe impl CompactValue for CompactOsString {}

/// A borrowed view over an exact platform OS-string representation.
#[derive(Clone, Copy)]
pub struct CompactOsStr<'view> {
    raw: &'view [u8],
}
impl<'view> CompactOsStr<'view> {
    /// Return the target-local encoded bytes.
    #[doc(hidden)]
    pub fn as_encoded_bytes(self) -> &'view [u8] {
        self.raw
    }
    /// Convert to a native owned OS string without loss.
    pub fn to_os_string(self) -> OsString {
        native_os_string(self.raw)
    }
    /// Return platform units: bytes on Unix and UTF-16 units on Windows.
    pub fn len(self) -> usize {
        platform_unit_len(self.raw)
    }
    /// Return whether the OS string is empty.
    pub fn is_empty(self) -> bool {
        self.raw.is_empty()
    }
    /// Return a lossy UTF-8 view for display.
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.to_os_string().fmt(f)
    }
}
impl PartialEq for CompactOsStr<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}
impl Eq for CompactOsStr<'_> {}

/// Compact path owner with standard-library path operations.
pub struct CompactPathBuf {
    inner: CompactOsString,
}
impl CompactPathBuf {
    /// Construct an empty compact path.
    pub const fn new() -> Self {
        Self {
            inner: CompactOsString::new(),
        }
    }
    /// Copy a native path into compact storage.
    pub fn from_path(path: &Path) -> Result<Self> {
        Ok(Self {
            inner: CompactOsString::from_os_str(path.as_os_str())?,
        })
    }
    /// Copy any native path type into compact storage.
    pub fn from<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::from_path(path.as_ref())
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
    /// Convert to a native owned path without loss.
    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(self.inner.to_os_string())
    }
    /// Return the parent as a borrowed compact path view.
    pub fn parent(&self) -> Option<CompactPath<'_>> {
        self.as_path().parent()
    }
    /// Return the final file name.
    pub fn file_name(&self) -> Option<CompactOsStr<'_>> {
        self.as_path().file_name()
    }
    /// Return the final file stem.
    pub fn file_stem(&self) -> Option<CompactOsStr<'_>> {
        self.as_path().file_stem()
    }
    /// Return the final file extension.
    pub fn extension(&self) -> Option<CompactOsStr<'_>> {
        self.as_path().extension()
    }
    /// Iterate path components in standard order.
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
    /// Return whether this path starts with `base` by components.
    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        self.as_path().starts_with(base)
    }
    /// Return whether this path ends with `child` by components.
    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        self.as_path().ends_with(child)
    }
    /// Return a standard lossy display wrapper.
    pub fn display(&self) -> CompactPathDisplay {
        self.as_path().display()
    }
    /// Append a path using the platform's standard path rules.
    pub fn push<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let mut native = self.to_path_buf();
        native.push(path);
        self.replace_from_path(&native)
    }
    /// Remove the final path component, if present.
    pub fn pop(&mut self) -> Result<bool> {
        let mut native = self.to_path_buf();
        if !native.pop() {
            return Ok(false);
        }
        self.replace_from_path(&native)?;
        Ok(true)
    }
    /// Set the final path component.
    pub fn set_file_name<S: AsRef<OsStr>>(&mut self, name: S) -> Result<()> {
        let mut native = self.to_path_buf();
        native.set_file_name(name);
        self.replace_from_path(&native)
    }
    /// Set or remove the final path extension.
    pub fn set_extension<S: AsRef<OsStr>>(&mut self, extension: S) -> Result<bool> {
        let mut native = self.to_path_buf();
        let changed = native.set_extension(extension);
        if changed {
            self.replace_from_path(&native)?;
        }
        Ok(changed)
    }
    /// Join a path and return compact storage.
    pub fn join<P: AsRef<Path>>(&self, path: P) -> Result<Self> {
        let mut native = self.to_path_buf();
        native.push(path);
        Self::from_path(&native)
    }
    fn replace_from_path(&mut self, path: &Path) -> Result<()> {
        let replacement = CompactOsString::from_os_str(path.as_os_str())?;
        self.inner = replacement;
        Ok(())
    }
}
impl Default for CompactPathBuf {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Debug for CompactPathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.to_path_buf().fmt(f)
    }
}
// SAFETY: the path owns only compact bytes and scalar metadata.
unsafe impl CompactValue for CompactPathBuf {}

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

fn compact_os_bytes(value: &OsStr) -> Result<CompactBytes> {
    let mut bytes = CompactBytes::new();
    append_os_str(&mut bytes, value)?;
    Ok(bytes)
}

fn append_os_str(destination: &mut CompactBytes, value: &OsStr) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        destination.extend_from_slice(value.as_bytes())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let byte_len = value
            .encode_wide()
            .count()
            .checked_mul(2)
            .ok_or(compact_core::Error::OffsetOverflow)?;
        destination.reserve(byte_len)?;
        for unit in value.encode_wide() {
            destination.extend_from_slice(&unit.to_le_bytes())?;
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        destination.extend_from_slice(value.as_encoded_bytes())
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
        // SAFETY: these bytes were copied unchanged from this target's OsStr encoding.
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
