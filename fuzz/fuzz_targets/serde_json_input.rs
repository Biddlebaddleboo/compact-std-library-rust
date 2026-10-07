#![no_main]

use compact_std::prelude::*;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    StdArena::with_capacity(64 * 1024, |arena| {
        if let Ok(values) =
            compact_std::json::from_slice_in::<CompactVec<'_, CompactString<'_>>>(input, arena)
        {
            if let Ok(values) = values.as_slice(arena) {
                for value in values {
                    let _ = value.as_str(arena);
                }
            }
        }
    })
    .unwrap();
});
