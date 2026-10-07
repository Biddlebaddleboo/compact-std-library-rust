#![no_main]

use compact_std::prelude::*;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let payload = &input[..input.len().min(2048)];
    let chunks = (payload.len() + 7) / 8;
    StdArena::with_capacity(64 * 1024, |arena| {
        let mut slots: CompactVec<'_, Option<CompactBytes<'_>>> =
            CompactVec::with_capacity_in(chunks, arena).unwrap();
        for _ in 0..chunks {
            slots.push_in(None, arena).unwrap();
        }

        for (sequence, chunk) in payload.chunks(8).enumerate() {
            let index = (sequence * 7) % chunks;
            let empty = slots
                .get(index, arena)
                .unwrap()
                .is_some_and(Option::is_none);
            if empty {
                let chunk = CompactBytes::from_slice_in(chunk, arena).unwrap();
                *slots.get_mut(index, arena).unwrap().unwrap() = Some(chunk);
            }
        }

        let mut body = CompactBytes::new_in(arena);
        for chunk in slots.as_slice(arena).unwrap().iter().flatten() {
            let _ = body.extend_from_slice_in(chunk.as_slice(), arena);
        }
        let _ = body.as_slice();
    })
    .unwrap();
});
