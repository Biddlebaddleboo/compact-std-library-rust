use compact_backend_std::StdArena;
use compact_collections::{CompactComponentKind, CompactOsString, CompactPathBuf};
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
#[test]
fn unix_os_strings_preserve_non_utf8_bytes_exactly() {
    use std::os::unix::ffi::OsStringExt;

    let bytes = b"logs/\xff-entry".to_vec();
    let native = OsString::from_vec(bytes.clone());
    StdArena::with_capacity(4096, |arena| {
        let compact = CompactOsString::from(&native, arena).unwrap();
        assert_eq!(compact.to_os_string(), native);
        assert_eq!(compact.as_os_str().as_bytes(), bytes);
        assert_eq!(compact.len(), bytes.len());
        assert!(compact.to_string_lossy().contains('\u{fffd}'));

        let path = Path::new(&native);
        let compact_path = CompactPathBuf::from(path, arena).unwrap();
        assert_eq!(compact_path.to_path_buf(), PathBuf::from(path));
        assert_eq!(compact_path.file_name().unwrap().as_bytes(), b"\xff-entry");
        assert_eq!(compact_path.as_os_str().as_bytes(), bytes);
    })
    .unwrap();
}

#[test]
fn path_queries_match_std_path_components_and_borrowed_parts() {
    let samples = [
        "alpha/beta.tar.gz",
        "/root/./alpha//beta.tar.gz",
        "../.hidden",
        "foo/../bar",
        "foo/",
        "foo.foo",
        ".config",
    ];
    StdArena::with_capacity(16 * 1024, |arena| {
        for sample in samples {
            let native = Path::new(sample);
            let compact = CompactPathBuf::from(native, arena).unwrap();
            let view = compact.as_path();
            assert_eq!(compact.to_path_buf(), PathBuf::from(native));
            assert_eq!(compact.is_absolute(), native.is_absolute());
            assert_eq!(compact.is_relative(), native.is_relative());
            assert_eq!(compact.starts_with("alpha"), native.starts_with("alpha"));
            assert_eq!(compact.ends_with("bar"), native.ends_with("bar"));
            assert_eq!(
                view.parent().map(|path| path.to_path_buf()),
                native.parent().map(Path::to_path_buf)
            );
            assert_eq!(
                compact.file_name().map(|name| name.to_os_string()),
                native.file_name().map(OsStr::to_os_string)
            );
            assert_eq!(
                compact.file_stem().map(|name| name.to_os_string()),
                native.file_stem().map(OsStr::to_os_string)
            );
            assert_eq!(
                compact.extension().map(|name| name.to_os_string()),
                native.extension().map(OsStr::to_os_string)
            );

            let compact_components: Vec<_> = compact
                .components()
                .map(|component| (component.kind(), component.as_os_str().to_os_string()))
                .collect();
            let native_components: Vec<_> = native
                .components()
                .map(|component| {
                    (
                        component_kind(component),
                        component.as_os_str().to_os_string(),
                    )
                })
                .collect();
            assert_eq!(compact_components, native_components, "path: {sample:?}");
            assert_eq!(compact.display().to_string(), native.display().to_string());
        }
    })
    .unwrap();
}

#[test]
fn path_mutation_and_join_match_std_path_buf() {
    StdArena::with_capacity(16 * 1024, |arena| {
        let mut compact = CompactPathBuf::from("root", arena).unwrap();
        let mut native = PathBuf::from("root");

        compact.push("nested", arena).unwrap();
        native.push("nested");
        compact.set_file_name("record.log", arena).unwrap();
        native.set_file_name("record.log");
        assert_eq!(
            compact.set_extension("txt", arena).unwrap(),
            native.set_extension("txt")
        );
        assert_eq!(compact.to_path_buf(), native);
        assert_eq!(
            compact.join("child", arena).unwrap().to_path_buf(),
            native.join("child")
        );
        assert_eq!(compact.pop(arena).unwrap(), native.pop());
        assert_eq!(compact.to_path_buf(), native);

        let mut os_string = CompactOsString::from(OsStr::new("prefix"), arena).unwrap();
        os_string.push(OsStr::new("-suffix"), arena).unwrap();
        assert_eq!(os_string.to_os_string(), OsString::from("prefix-suffix"));
        os_string.clear();
        assert!(os_string.is_empty());
    })
    .unwrap();
}

#[test]
fn arena_exhaustion_leaves_path_and_os_string_unchanged() {
    let long = "x".repeat(128);
    StdArena::with_capacity(compact_core::MIN_ARENA_BYTES, |arena| {
        let mut path = CompactPathBuf::from("base", arena).unwrap();
        assert!(path.push(&long, arena).is_err());
        assert_eq!(path.to_path_buf(), PathBuf::from("base"));

        let mut os_string = CompactOsString::from(OsStr::new("a"), arena).unwrap();
        assert!(os_string.push(OsStr::new(&long), arena).is_err());
        assert_eq!(os_string.to_os_string(), OsString::from("a"));
    })
    .unwrap();
}

#[cfg(windows)]
#[test]
fn windows_os_strings_preserve_non_unicode_wide_units_exactly() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    let units = [0xd800, 0x0061, 0xdc00];
    let native = OsString::from_wide(&units);
    StdArena::with_capacity(4096, |arena| {
        let compact = CompactOsString::from(&native, arena).unwrap();
        let round_trip: Vec<_> = compact.to_os_string().encode_wide().collect();
        assert_eq!(round_trip, units);
        assert_eq!(compact.len(), units.len());

        let path = PathBuf::from(native.clone());
        let compact_path = CompactPathBuf::from(&path, arena).unwrap();
        let round_trip: Vec<_> = compact_path
            .to_path_buf()
            .as_os_str()
            .encode_wide()
            .collect();
        assert_eq!(round_trip, units);
    })
    .unwrap();
}

fn component_kind(component: Component<'_>) -> CompactComponentKind {
    match component {
        Component::Prefix(_) => CompactComponentKind::Prefix,
        Component::RootDir => CompactComponentKind::RootDir,
        Component::CurDir => CompactComponentKind::CurDir,
        Component::ParentDir => CompactComponentKind::ParentDir,
        Component::Normal(_) => CompactComponentKind::Normal,
    }
}
