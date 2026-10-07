#![no_main]

use compact_std::{CompactHashMap, StdArena};
use libfuzzer_sys::fuzz_target;
use std::hash::{BuildHasher, Hasher};

#[derive(Clone, Default)]
struct ConstantBuildHasher;

struct ConstantHasher;

impl Hasher for ConstantHasher {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, _bytes: &[u8]) {}
}

impl BuildHasher for ConstantBuildHasher {
    type Hasher = ConstantHasher;

    fn build_hasher(&self) -> Self::Hasher {
        ConstantHasher
    }
}

fuzz_target!(|input: &[u8]| {
    StdArena::with_capacity(64 * 1024, |arena| {
        let mut map = CompactHashMap::with_hasher(ConstantBuildHasher);
        for operation in input.chunks_exact(6).take(512) {
            let key = operation[1];
            match operation[0] % 3 {
                0 => {
                    let value = u32::from_le_bytes([
                        operation[2],
                        operation[3],
                        operation[4],
                        operation[5],
                    ]);
                    let _ = map.insert(key, value, arena);
                }
                1 => {
                    let _ = map.remove(&key, arena);
                }
                _ => {
                    let _ = map.get(&key, arena);
                }
            }
            let _ = map.iter(arena).map(|entries| entries.count());
        }
    })
    .unwrap();
});
