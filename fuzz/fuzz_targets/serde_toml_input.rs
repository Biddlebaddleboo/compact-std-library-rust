#![no_main]

use compact_std::prelude::*;
use libfuzzer_sys::fuzz_target;

#[derive(CompactDeserialize)]
struct ValuesDocument<'arena> {
    values: CompactVec<'arena, u64>,
}

fuzz_target!(|input: &[u8]| {
    let Ok(input) = std::str::from_utf8(input) else {
        return;
    };
    StdArena::with_capacity(64 * 1024, |arena| {
        if let Ok(document) = compact_std::toml::from_str_in::<ValuesDocument<'_>>(input, arena) {
            let _ = document.values.as_slice(arena);
        }
    })
    .unwrap();
});
