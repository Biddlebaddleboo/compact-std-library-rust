#![no_main]

mod common;
use compact_std::CompactVec;
use libfuzzer_sys::fuzz_target;

#[derive(compact_std::CompactDeserialize)]
struct ValuesDocument { values: CompactVec<u64> }

fuzz_target!(|input: &[u8]| {
    common::init();
    let Ok(input) = std::str::from_utf8(input) else { return; };
    if let Ok(document) = compact_std::toml::from_str::<ValuesDocument>(input) { let _ = document.values.as_slice(); }
});
