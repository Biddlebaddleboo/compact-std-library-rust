#![cfg(all(feature = "serde", feature = "json", feature = "toml"))]

use compact_std::{
    CompactBytes, CompactDeserialize, CompactHashMap, CompactHashSet, CompactOsString,
    CompactPathBuf, CompactString, CompactVec, CompactVecDeque, Result, StdArena,
};
use std::path::Path;

#[derive(CompactDeserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ServicePaths<'arena> {
    config_path: CompactPathBuf<'arena>,
    log_path: Option<CompactPathBuf<'arena>>,
}

#[derive(CompactDeserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct ServiceConfig<'arena> {
    system_id: CompactString<'arena>,
    port: u16,
    #[serde(default)]
    names: CompactVec<'arena, CompactString<'arena>>,
    #[serde(default)]
    annotations: CompactVec<'arena, CompactString<'arena>>,
    #[serde(rename = "validation")]
    validation_inputs: CompactHashSet<'arena, CompactString<'arena>>,
    paths: ServicePaths<'arena>,
    optional_label: Option<CompactString<'arena>>,
    #[serde(skip)]
    local_cache: CompactString<'arena>,
}

#[derive(CompactDeserialize)]
#[serde(deny_unknown_fields)]
struct NodeRegistry<'arena> {
    nodes: CompactHashMap<'arena, CompactString<'arena>, u32>,
}

#[derive(CompactDeserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum NodeState {
    Ready,
    NeedsRepair,
}

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
