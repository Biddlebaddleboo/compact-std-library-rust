#![no_main]

mod common;
use compact_std::{CompactHashMap, CompactValue};
use libfuzzer_sys::fuzz_target;
use std::collections::BTreeMap;
use std::hash::{BuildHasher, Hasher};

#[derive(Clone, Copy, Default)]
struct ConstantBuildHasher;
struct ConstantHasher;
impl Hasher for ConstantHasher { fn finish(&self) -> u64 { 0 } fn write(&mut self, _bytes: &[u8]) {} }
impl BuildHasher for ConstantBuildHasher { type Hasher = ConstantHasher; fn build_hasher(&self) -> Self::Hasher { ConstantHasher } }
unsafe impl CompactValue for ConstantBuildHasher {}

fuzz_target!(|input: &[u8]| {
    common::init();
    let mut map = CompactHashMap::with_hasher(ConstantBuildHasher);
    let mut model = BTreeMap::new();
    for operation in input.chunks_exact(6).take(512) {
        let key = operation[1];
        match operation[0] % 3 {
            0 => {
                let value = u32::from_le_bytes([operation[2], operation[3], operation[4], operation[5]]);
                assert_eq!(map.insert(key, value).unwrap(), model.insert(key, value));
            }
            1 => assert_eq!(map.remove(&key), model.remove(&key)),
            _ => assert_eq!(map.get(&key), model.get(&key)),
        }
        assert_eq!(map.iter().count(), model.len());
        for (key, value) in &model {
            assert_eq!(map.get(key), Some(value));
        }
    }
});
