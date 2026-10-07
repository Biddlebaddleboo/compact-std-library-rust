#![no_main]

use compact_std::prelude::*;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    StdArena::with_capacity(64 * 1024, |arena| {
        let mut owners: std::vec::Vec<Option<ArenaAllocation<'_, u8>>> =
            std::iter::repeat_with(|| None).take(16).collect();
        for operation in input.chunks_exact(4).take(512) {
            let index = operation[1] as usize % owners.len();
            match operation[0] % 4 {
                0 => {
                    if owners[index].is_none() {
                        let capacity = 1 + operation[2] as usize % 32;
                        if let Ok(mut owner) = arena.alloc_owned_slice::<u8>(capacity) {
                            let initialized = operation[3] as usize % (capacity + 1);
                            for offset in 0..initialized {
                                if owner.push(operation[2].wrapping_add(offset as u8)).is_err() {
                                    break;
                                }
                            }
                            owners[index] = Some(owner);
                        }
                    }
                }
                1 => owners[index] = None,
                2 => {
                    if let Some(owner) = owners[index].as_mut() {
                        let capacity = owner.capacity() + 1 + operation[2] as usize % 16;
                        let _ = arena.try_resize_owned(owner, capacity);
                    }
                }
                _ => {
                    if let Some(owner) = owners[index].as_mut() {
                        if !owner.is_empty() {
                            let element = operation[2] as usize % owner.len();
                            if let Some(slot) = owner.get_mut(element) {
                                *slot ^= operation[3];
                            }
                        }
                    }
                }
            }
            let _checksum = owners
                .iter()
                .flatten()
                .flat_map(|owner| owner.as_slice())
                .fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
        }
    })
    .unwrap();
});
