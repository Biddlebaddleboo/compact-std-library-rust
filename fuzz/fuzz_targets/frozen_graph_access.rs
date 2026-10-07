#![no_main]

mod common;
use compact_std::{CompactString, CompactVec, FrozenBuilder, FrozenString, FrozenVec};
use libfuzzer_sys::fuzz_target;

#[derive(Clone, Copy, compact_std::FrozenValue)]
struct CompactGraph { names: FrozenVec<FrozenString> }

fuzz_target!(|input: &[u8]| {
    common::init();
    let Ok(names) = compact_std::json::from_slice::<CompactVec<CompactString>>(input) else { return; };
    let Ok(mut builder) = FrozenBuilder::new() else { return; };
    let mut strings = std::vec::Vec::new();
    for name in &names {
        let Ok(value) = builder.store_str(name.as_str()) else { return; };
        strings.push(value);
    }
    let Ok(names) = builder.store_slice(&strings) else { return; };
    let Ok(graph) = builder.finish(CompactGraph { names }) else { return; };
    if let Ok(names) = graph.slice(graph.root().names) { for name in names { let _ = graph.str(*name); } }
});
