#![no_main]

mod common;
use compact_std::{CompactString, CompactVec};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    common::init();
    if let Ok(values) = compact_std::json::from_slice::<CompactVec<CompactString>>(input) {
        for value in &values { let _ = value.as_str(); }
    }
});
