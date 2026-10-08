use compact_std::{
    CompactRuntime, FrozenBuilder, FrozenBytes, FrozenGraph, FrozenString, FrozenVec,
};
use core::mem::size_of;
use std::sync::{Mutex, MutexGuard, OnceLock};

static INIT: OnceLock<()> = OnceLock::new();
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn init() -> MutexGuard<'static, ()> {
    let guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    INIT.get_or_init(|| {
        CompactRuntime::init(compact_std::CageConfig::new(64 * 1024 * 1024)).unwrap()
    });
    guard
}

#[compact_std::compact]
struct LogicalRecord {
    active: bool,
    #[max = 7]
    retries: u8,
    name: std::string::String,
}

#[compact_std::compact]
enum Mode {
    Idle,
    Running,
    Stopped,
}

#[compact_std::compact]
enum SingleMode {
    Only,
}

#[compact_std::compact]
struct ZeroWidthFields {
    #[max = 0]
    value: u8,
    mode: SingleMode,
}

#[compact_std::compact(soa)]
struct Sample {
    id: u32,
    enabled: bool,
}

#[test]
fn packed_layouts_encode_and_update() {
    let _guard = init();
    let source = LogicalRecord {
        active: true,
        retries: 5,
        name: "worker".into(),
    };
    let mut packed = source.compact().unwrap();
    assert!(packed.active().unwrap());
    assert_eq!(packed.retries().unwrap(), 5);
    assert_eq!(packed.name(), "worker");
    packed.set_retries(7).unwrap();
    packed.set_name("scheduler").unwrap();
    assert_eq!(packed.retries().unwrap(), 7);
    assert_eq!(packed.name(), "scheduler");

    let state = Mode::Running.compact().unwrap();
    assert_eq!(state.get().unwrap() as u8, Mode::Running as u8);

    let zero = ZeroWidthFields {
        value: 0,
        mode: SingleMode::Only,
    }
    .compact()
    .unwrap();
    assert_eq!(zero.value().unwrap(), 0);
    assert!(matches!(zero.mode().unwrap(), SingleMode::Only));

    let mut columns = SampleSoa::new();
    columns
        .push(Sample {
            id: 12,
            enabled: true,
        })
        .unwrap();
    assert_eq!(columns.get(0).unwrap().unwrap().id, 12);
    assert!(columns.get(0).unwrap().unwrap().enabled);
}

#[derive(Clone, Copy, compact_std::FrozenValue)]
struct CatalogRoot {
    title: FrozenString,
    ids: FrozenVec<u32>,
    empty: FrozenString,
    unicode: FrozenString,
    bytes: FrozenBytes,
}

#[test]
fn frozen_descriptors_keep_eight_byte_representations() {
    assert_eq!(size_of::<FrozenString>(), 8);
    assert_eq!(size_of::<FrozenBytes>(), 8);
    assert_eq!(size_of::<FrozenVec<u32>>(), 8);
}

#[test]
fn frozen_graph_owns_one_immutable_offset_domain() {
    let _guard = init();
    let mut builder = FrozenBuilder::new().unwrap();
    let title = builder.store_str("edge-router").unwrap();
    let ids = builder.store_slice(&[4_u32, 8, 15, 16, 23, 42]).unwrap();
    let empty = builder.store_str("").unwrap();
    let unicode = builder.store_str("café 🦀").unwrap();
    let bytes = builder.store_bytes(&[0, 255, 1, 128]).unwrap();
    let graph: FrozenGraph<CatalogRoot> = builder
        .finish(CatalogRoot {
            title,
            ids,
            empty,
            unicode,
            bytes,
        })
        .unwrap();
    assert_eq!(graph.str(&graph.root().title).unwrap(), "edge-router");
    assert_eq!(
        graph.slice(&graph.root().ids).unwrap(),
        &[4, 8, 15, 16, 23, 42]
    );
    assert_eq!(graph.str(&graph.root().empty).unwrap(), "");
    assert_eq!(graph.str(&graph.root().unicode).unwrap(), "café 🦀");
    assert_eq!(graph.bytes(&graph.root().bytes).unwrap(), &[0, 255, 1, 128]);
    assert!(graph.used_bytes() >= 8);

    let view = graph.view();
    assert_eq!(view.str(&graph.root().title).unwrap(), "edge-router");
    assert_eq!(
        view.slice(&graph.root().ids).unwrap(),
        &[4, 8, 15, 16, 23, 42]
    );

    let other = FrozenBuilder::new().unwrap();
    let other_graph = other.finish(0_u32).unwrap();
    assert!(other_graph.str(&title).is_err());
    assert!(other_graph.str(&graph.root().title).is_err());
    let copied = graph.root().title;
    assert!(graph.str(&copied).is_err());
    assert!(view.str(&copied).is_err());
    assert!(other_graph.view().str(&graph.root().title).is_err());

    std::thread::scope(|scope| {
        for _ in 0..4 {
            let graph = &graph;
            scope.spawn(move || assert_eq!(view.str(&graph.root().title).unwrap(), "edge-router"));
        }
    });
}

#[test]
fn ffi_borrow_is_scoped_to_the_compact_owner() {
    let _guard = init();
    let bytes = compact_std::CompactBytes::from_slice(b"native boundary").unwrap();
    let observed = bytes.with_ffi_bytes(|view| {
        assert_eq!(view.as_ptr(), bytes.as_ptr());
        view.to_vec()
    });
    assert_eq!(observed, b"native boundary");
}

#[cfg(feature = "json")]
#[derive(compact_std::CompactDeserialize)]
struct JsonConfig {
    service: compact_std::CompactString,
    retries: u32,
    labels: compact_std::CompactVec<compact_std::CompactString>,
}

#[cfg(feature = "json")]
fn default_attempts() -> u32 {
    3
}

#[cfg(feature = "json")]
#[derive(compact_std::CompactDeserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DerivedAttributes {
    server_name: compact_std::CompactString,
    #[serde(skip)]
    ignored: bool,
    #[serde(default = "default_attempts")]
    attempts: u32,
}

#[cfg(feature = "json")]
#[test]
fn json_and_toml_build_cage_backed_fields_directly() {
    let _guard = init();
    let json = compact_std::json::from_str::<JsonConfig>(
        r#"{"service":"gateway","retries":3,"labels":["api","metrics"]}"#,
    )
    .unwrap();
    assert_eq!(json.service.as_str(), "gateway");
    assert_eq!(json.retries, 3);
    assert_eq!(json.labels[1].as_str(), "metrics");

    #[cfg(feature = "toml")]
    {
        let toml = compact_std::toml::from_str::<JsonConfig>(
            "service = 'worker'\nretries = 4\nlabels = ['queue', 'cache']\n",
        )
        .unwrap();
        assert_eq!(toml.service.as_str(), "worker");
        assert_eq!(toml.labels[0].as_str(), "queue");
    }
}

#[cfg(feature = "json")]
#[test]
fn malformed_json_releases_partially_built_compact_values() {
    let _guard = init();
    let before = CompactRuntime::used_bytes().unwrap();
    let result = compact_std::json::from_str::<JsonConfig>(
        r#"{"service":"partially allocated","retries":"bad","labels":["temporary"]}"#,
    );
    assert!(result.is_err());
    assert_eq!(CompactRuntime::used_bytes().unwrap(), before);
}

#[cfg(feature = "json")]
#[test]
fn serde_derive_applies_rename_defaults_skip_and_unknown_field_rules() {
    let _guard = init();
    let value =
        compact_std::json::from_str::<DerivedAttributes>(r#"{"serverName":"api"}"#).unwrap();
    assert_eq!(value.server_name.as_str(), "api");
    assert!(!value.ignored);
    assert_eq!(value.attempts, 3);

    let sequence = compact_std::json::from_str::<DerivedAttributes>(r#"["worker",false]"#).unwrap();
    assert_eq!(sequence.server_name.as_str(), "worker");
    assert!(!sequence.ignored);
    assert_eq!(sequence.attempts, 3);

    assert!(compact_std::json::from_str::<DerivedAttributes>(
        r#"{"serverName":"api","extra":true}"#
    )
    .is_err());
}
