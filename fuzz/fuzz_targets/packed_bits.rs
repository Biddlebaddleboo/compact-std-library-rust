#![no_main]

use compact_std::prelude::*;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    StdArena::with_capacity(32 * 1024, |arena| {
        let mut bits = CompactBitVec::new_in(arena);
        for byte in input.iter().copied().take(2048) {
            for bit in 0..8 {
                if bits.push_in(byte & (1 << bit) != 0, arena).is_err() {
                    return;
                }
            }
        }
        for index in 0..bits.len() {
            let _ = bits.get(index, arena);
        }
    })
    .unwrap();
});
