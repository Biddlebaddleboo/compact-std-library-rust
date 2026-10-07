#![no_main]

mod common;
use compact_std::CompactBitVec;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    common::init();
    let mut bits = CompactBitVec::new();
    for byte in input.iter().copied().take(2048) {
        for bit in 0..8 { if bits.push(byte & (1 << bit) != 0).is_err() { return; } }
    }
    for index in 0..bits.len() { let _ = bits.get(index); }
});
