#![no_main]

use compact_std::prelude::*;
use libfuzzer_sys::fuzz_target;

#[derive(CompactDeserialize, CompactFreeze)]
struct CompactGraph<'arena> {
    names: CompactVec<'arena, CompactString<'arena>>,
}

fuzz_target!(|input: &[u8]| {
    StdArena::with_capacity(64 * 1024, |arena| {
        let Ok(graph) = compact_std::json::from_slice_in::<CompactGraph<'_>>(input, arena) else {
            return;
        };
        let Ok((frozen, root)) = freeze_in(&graph, arena) else {
            return;
        };
        let Ok(graph) = root.get(&frozen) else {
            return;
        };
        if let Ok(names) = graph.names().as_slice(&frozen) {
            for name in names {
                let _ = name.as_str(&frozen);
            }
        }
    })
    .unwrap();
});
