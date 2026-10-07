#![cfg(all(feature = "serde", feature = "json", feature = "toml"))]

use compact_std::{
    freeze_in, CompactBytes, CompactDeserialize, CompactFreeze, CompactHashMap, CompactHashSet,
    CompactOsString, CompactPathBuf, CompactString, CompactVec, CompactVecDeque, FrozenArena,
    FrozenBuilder, FrozenError, FrozenResult, FrozenValue, Result, StdArena,
};
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

#[derive(CompactDeserialize, CompactFreeze)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ServicePaths<'arena> {
    config_path: CompactPathBuf<'arena>,
    log_path: Option<CompactPathBuf<'arena>>,
}

#[derive(CompactDeserialize, CompactFreeze)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct ServiceConfig<'arena> {
    system_id: CompactString<'arena>,
    port: u16,
    #[serde(default)]
    names: CompactVec<'arena, CompactString<'arena>>,
    #[serde(default)]
    annotations: CompactVec<'arena, CompactString<'arena>>,
    #[serde(default)]
    metadata: CompactHashMap<'arena, CompactString<'arena>, u32>,
    #[serde(rename = "validation")]
    validation_inputs: CompactHashSet<'arena, CompactString<'arena>>,
    paths: ServicePaths<'arena>,
    optional_label: Option<CompactString<'arena>>,
    #[serde(skip)]
    local_cache: CompactString<'arena>,
}

#[derive(CompactDeserialize, CompactFreeze)]
#[serde(deny_unknown_fields)]
struct NodeRegistry<'arena> {
    nodes: CompactHashMap<'arena, CompactString<'arena>, u32>,
}

#[derive(CompactDeserialize, CompactFreeze, Copy, Clone, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum NodeState {
    Ready,
    NeedsRepair,
}

#[repr(align(64))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct AlignedFrozen(u64);

// SAFETY: this is a Copy scalar wrapper with no references, mutability, or
// destructor; its alignment is exactly the builder's supported maximum.
unsafe impl FrozenValue for AlignedFrozen {}

#[derive(Clone, Copy)]
struct FrozenUnit;

// SAFETY: this unit value has no fields, references, mutability, or destructor.
unsafe impl FrozenValue for FrozenUnit {}

#[test]
fn toml_fixture_deserializes_nested_compact_configuration() {
    let input = include_str!("fixtures/local-service-orchestrator.toml");
    StdArena::with_capacity(32 * 1024, |arena| -> Result<()> {
        let config = compact_std::toml::from_str_in::<ServiceConfig<'_>>(input, arena)
            .unwrap_or_else(|error| panic!("TOML compact deserialization failed: {error}"));

        assert_eq!(config.system_id.as_str(arena)?, "orchestrator.local");
        assert_eq!(config.port, 8088);
        assert_eq!(config.names.len(), 2);
        assert!(config.annotations.is_empty());
        assert_eq!(config.names.get(0, arena)?.unwrap().as_str(arena)?, "api");
        assert_eq!(config.validation_inputs.len(), 2);
        assert!(config.optional_label.is_none());
        assert!(config.local_cache.is_empty());
        assert_eq!(
            config.paths.config_path.to_path_buf(),
            Path::new("/etc/orchestrator/service.toml")
        );
        assert_eq!(
            config.paths.log_path.as_ref().unwrap().to_path_buf(),
            Path::new("/var/log/orchestrator")
        );
        Ok(())
    })
    .unwrap()
    .unwrap();
}

#[test]
fn json_fixture_deserializes_compact_node_registry_and_containers() {
    let input = include_str!("fixtures/node-registry.json");
    StdArena::with_capacity(32 * 1024, |arena| -> Result<()> {
        let registry =
            compact_std::json::from_slice_in::<NodeRegistry<'_>>(input.as_bytes(), arena)
                .map_err(|_| compact_std::CollectionError::InvalidCompactValue)?;
        let key = CompactString::from_str_in("gateway", arena)?;
        assert_eq!(registry.nodes.get(&key, arena)?, Some(&17));

        let bytes = compact_std::json::from_str_in::<CompactBytes<'_>>("[1, 2, 255]", arena)
            .map_err(|_| compact_std::CollectionError::InvalidCompactValue)?;
        assert_eq!(bytes.as_slice(), &[1, 2, 255]);

        let sequence = compact_std::json::from_str_in::<
            CompactVecDeque<'_, (u32, CompactString<'_>)>,
        >("[[3, \"ready\"]]", arena)
        .map_err(|_| compact_std::CollectionError::InvalidCompactValue)?;
        assert_eq!(sequence.len(), 1);
        let (code, label) = sequence.front(arena)?.unwrap();
        assert_eq!(*code, 3);
        assert_eq!(label.as_str(arena)?, "ready");

        let os_name = compact_std::json::from_str_in::<CompactOsString<'_>>("\"node-a\"", arena)
            .map_err(|_| compact_std::CollectionError::InvalidCompactValue)?;
        assert_eq!(os_name.to_os_string(), "node-a");

        let state = compact_std::json::from_str_in::<NodeState>("\"needs-repair\"", arena)
            .map_err(|_| compact_std::CollectionError::InvalidCompactValue)?;
        assert_eq!(state, NodeState::NeedsRepair);
        Ok(())
    })
    .unwrap()
    .unwrap();
}

#[test]
fn malformed_duplicate_unknown_and_exhausted_inputs_return_errors() {
    StdArena::with_capacity(32 * 1024, |arena| {
        assert!(compact_std::json::from_str_in::<NodeRegistry<'_>>("{", arena).is_err());
        assert!(compact_std::json::from_str_in::<NodeRegistry<'_>>(
            "{\"nodes\": {}, \"nodes\": {}}",
            arena
        )
        .is_err());
        assert!(compact_std::json::from_str_in::<ServicePaths<'_>>(
            "{\"configPath\": \"/tmp/a\", \"unknown\": 1}",
            arena
        )
        .is_err());
        assert!(
            compact_std::json::from_str_in::<NodeRegistry<'_>>("{\"nodes\": {}}", arena).is_ok()
        );
    })
    .unwrap();

    StdArena::with_capacity(compact_std::MIN_ARENA_BYTES, |arena| {
        let exhausted = compact_std::json::from_str_in::<CompactVec<'_, CompactString<'_>>>(
            "[\"a string far larger than inline storage\", \"another large string\"]",
            arena,
        );
        assert!(exhausted.is_err());
        assert!(compact_std::json::from_str_in::<CompactString<'_>>("\"ok\"", arena).is_ok());
    })
    .unwrap();
}

#[test]
fn frozen_configuration_is_immutable_and_shareable_across_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<FrozenArena>();

    let input = include_str!("fixtures/local-service-orchestrator.toml");
    let (frozen, root) = StdArena::with_capacity(32 * 1024, |arena| {
        let config = compact_std::toml::from_str_in::<ServiceConfig<'_>>(input, arena)
            .unwrap_or_else(|error| panic!("TOML compact deserialization failed: {error}"));
        let graph = freeze_in(&config, arena)?;

        // Freeze copies from the still-valid mutable graph.
        assert_eq!(config.system_id.as_str(arena)?, "orchestrator.local");
        Ok::<_, FrozenError>(graph)
    })
    .unwrap()
    .unwrap();

    let shared = Arc::new(frozen);
    let barrier = Arc::new(Barrier::new(4));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let arena = Arc::clone(&shared);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let config = root.get(&arena).unwrap();
                assert_eq!(
                    config.system_id().as_str(&arena).unwrap(),
                    "orchestrator.local"
                );
                assert_eq!(config.names().len(), 2);
                assert_eq!(
                    config
                        .names()
                        .get(0, &arena)
                        .unwrap()
                        .unwrap()
                        .as_str(&arena)
                        .unwrap(),
                    "api"
                );
                assert!(config
                    .validation_inputs()
                    .contains_by(&arena, |value| {
                        value
                            .as_str(&arena)
                            .map(|value| value == "ready")
                            .unwrap_or(false)
                    })
                    .unwrap());
                assert_eq!(
                    config
                        .metadata()
                        .find_by(&arena, |key| {
                            key.as_str(&arena)
                                .map(|key| key == "workers")
                                .unwrap_or(false)
                        })
                        .unwrap()
                        .map(|(_, value)| *value),
                    Some(4)
                );
                assert_eq!(
                    config.paths().config_path().to_path_buf(&arena).unwrap(),
                    Path::new("/etc/orchestrator/service.toml")
                );
            })
        })
        .collect();
    for reader in readers {
        reader.join().unwrap();
    }

    let config = root.get(&shared).unwrap();
    let system_id = config.system_id();
    let mut other_builder = compact_std::FrozenBuilder::new().unwrap();
    let other_root = other_builder.store_str("unrelated").unwrap();
    let (other, _) = other_builder.finish_root(other_root).unwrap();
    assert!(matches!(
        system_id.as_str(&other),
        Err(FrozenError::InvalidHandle)
    ));

    let _typed_result: FrozenResult<&str> = system_id.as_str(&shared);
}

#[test]
fn frozen_backing_checks_identity_and_supports_zst_and_max_alignment() {
    let mut builder = FrozenBuilder::new().unwrap();
    let aligned = builder.store_slice(&[AlignedFrozen(91)]).unwrap();
    let units = builder.store_slice(&[FrozenUnit, FrozenUnit]).unwrap();
    let empty = builder.store_slice::<u8>(&[]).unwrap();
    let (arena, root) = builder.finish_root((aligned, units, empty)).unwrap();

    let (aligned, units, empty) = *root.get(&arena).unwrap();
    let values = aligned.as_slice(&arena).unwrap();
    assert_eq!(values[0], AlignedFrozen(91));
    assert_eq!(values.as_ptr() as usize % 64, 0);
    assert_eq!(units.as_slice(&arena).unwrap().len(), 2);
    assert!(empty.as_slice(&arena).unwrap().is_empty());

    let mut other_builder = FrozenBuilder::new().unwrap();
    let other_values = other_builder.store_slice(&[AlignedFrozen(12)]).unwrap();
    let (other_arena, _) = other_builder.finish_root(other_values).unwrap();
    assert!(matches!(
        aligned.as_slice(&other_arena),
        Err(FrozenError::InvalidHandle)
    ));
}
