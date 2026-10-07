#![no_main]

mod common;
use compact_std::{CompactBytes, CompactVec};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    common::init();
    let payload = &input[..input.len().min(2048)];
    let mut slots: CompactVec<Option<CompactBytes>> = CompactVec::new();
    for chunk in payload.chunks(8) { if slots.push(Some(CompactBytes::from_slice(chunk).unwrap())).is_err() { return; } }
    let mut body = CompactBytes::new();
    for chunk in slots.iter().flatten() { let _ = body.extend_from_slice(chunk.as_slice()); }
    let _ = body.as_slice();
});
