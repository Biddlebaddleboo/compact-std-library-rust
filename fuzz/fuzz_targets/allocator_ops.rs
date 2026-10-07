#![no_main]

mod common;
use compact_std::{CageAllocation, CompactRuntime};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    common::init();
    let mut owners: std::vec::Vec<Option<CageAllocation<u8>>> = std::iter::repeat_with(|| None).take(16).collect();
    for operation in input.chunks_exact(4).take(512) {
        let index = operation[1] as usize % owners.len();
        match operation[0] % 4 {
            0 => if owners[index].is_none() {
                let capacity = 1 + operation[2] as usize % 32;
                if let Ok(mut owner) = CompactRuntime::alloc_owned_slice::<u8>(capacity) {
                    let initialized = operation[3] as usize % (capacity + 1);
                    for offset in 0..initialized { if owner.push(operation[2].wrapping_add(offset as u8)).is_err() { break; } }
                    owners[index] = Some(owner);
                }
            },
            1 => owners[index] = None,
            2 => if let Some(owner) = owners[index].as_mut() { let _ = owner.try_resize(owner.capacity() + 1 + operation[2] as usize % 16); },
            _ => if let Some(owner) = owners[index].as_mut() {
                if !owner.is_empty() {
                    let slot = operation[2] as usize % owner.len();
                    if let Some(value) = owner.get_mut(slot) { *value ^= operation[3]; }
                }
            },
        }
        let _checksum = owners.iter().flatten().flat_map(|owner| owner.as_slice()).fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
    }
});
