use std::sync::OnceLock;

static INIT: OnceLock<()> = OnceLock::new();
pub fn init() {
    INIT.get_or_init(|| {
        compact_std::CompactRuntime::init(compact_std::CageConfig::new(64 * 1024 * 1024))
            .expect("initialize the fuzz process cage");
    });
}
