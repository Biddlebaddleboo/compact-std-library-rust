use compact_std::prelude::*;
use std::ffi::OsStr;
use std::path::Path;

#[test]
fn randomized_hash_aliases_are_available_from_the_prelude() {
    StdArena::with_capacity(4096, |arena| -> Result<()> {
        let mut counts = HashMap::new();
        counts.insert(42_u32, 1_u32, arena)?;
        counts.insert(42, 2, arena)?;
        assert_eq!(counts.get(&42, arena)?, Some(&2));

        let mut ids = HashSet::new();
        assert!(ids.insert(42_u32, arena)?);
        assert!(!ids.insert(42, arena)?);
        assert!(ids.contains(&42, arena)?);
        Ok(())
    })
    .unwrap()
    .unwrap();
}

#[test]
fn compact_os_and_path_aliases_are_available_from_the_prelude() {
    StdArena::with_capacity(4096, |arena| -> Result<()> {
        let mut path = PathBuf::from(Path::new("var/log"), arena)?;
        path.push("service", arena)?;
        assert_eq!(
            path.file_name().unwrap().to_os_string(),
            OsStr::new("service")
        );

        let mut name = OsString::from(OsStr::new("service"), arena)?;
        name.push(OsStr::new(".log"), arena)?;
        assert_eq!(name.to_os_string(), std::ffi::OsString::from("service.log"));
        Ok(())
    })
    .unwrap()
    .unwrap();
}
