use compact_std::prelude::*;
use compact_std::{CompactHashMap, CompactHashSet, CompactPathBuf, FrozenString, FrozenVec};

#[compact]
struct PackedRecord {
    active: bool,
    #[max = 7]
    retries: u8,
}

#[derive(CompactDeserialize)]
struct ServiceConfig {
    name: CompactString,
    tags: CompactVec<CompactString>,
}

#[derive(Clone, Copy, FrozenValue)]
struct CatalogRoot {
    title: FrozenString,
    ids: FrozenVec<u32>,
}

fn main() -> std::result::Result<(), std::boxed::Box<dyn std::error::Error>> {
    CompactRuntime::init(CageConfig::new(16 * 1024 * 1024))?;

    let mut values = Vec::new();
    values.push(10_u32)?;
    values.push(20_u32)?;
    let boxed = Box::new(42_u64)?;
    assert_eq!(*boxed, 42);

    let mut map = CompactHashMap::new();
    map.insert(1_u32, CompactString::from_str("gateway")?)?;
    let mut set = CompactHashSet::new();
    set.insert(1_u32)?;
    let path = CompactPathBuf::from("/etc/gateway/config.toml")?;
    assert!(path.is_absolute());

    let config = compact_std::toml::from_str::<ServiceConfig>(
        "name = 'gateway'\ntags = ['api', 'metrics']\n",
    )?;
    assert_eq!(config.name.as_str(), "gateway");
    assert_eq!(config.tags.len(), 2);

    let mut packed = PackedRecord {
        active: true,
        retries: 3,
    }
    .compact()?;
    packed.set_retries(4)?;
    assert!(packed.active()?);

    let mut scratch = ScratchRegion::new(64)?;
    assert_eq!(*scratch.alloc_value(7_u32)?, 7);
    assert_eq!(scratch.alloc_bytes(8)?, &[0; 8]);

    let mut builder = FrozenBuilder::new()?;
    let title = builder.store_str("catalog")?;
    let ids = builder.store_slice(&[3_u32, 5, 8])?;
    let graph = builder.finish(CatalogRoot { title, ids })?;
    assert_eq!(graph.str(&graph.root().title)?, "catalog");
    assert_eq!(graph.slice(&graph.root().ids)?, &[3, 5, 8]);

    let bytes = CompactBytes::from_slice(b"ffi")?;
    bytes.with_ffi_bytes(|view| assert_eq!(view, b"ffi"));

    let _ = compact_std::json::from_str::<CompactVec<CompactString>>(r#"["a","b"]"#)?;
    println!("V2.4 cage bytes used: {}", CompactRuntime::used_bytes()?);
    Ok(())
}
