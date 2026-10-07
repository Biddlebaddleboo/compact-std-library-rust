#![no_main]

mod common;

use compact_std::{CageAllocation, CompactRuntime, CompactValue};
use libfuzzer_sys::fuzz_target;

#[repr(align(64))]
#[derive(Clone, Copy)]
struct Aligned64(u8);
unsafe impl CompactValue for Aligned64 {}

enum Owner {
    Byte(CageAllocation<u8>),
    Word(CageAllocation<u64>),
    Aligned(CageAllocation<Aligned64>),
}

impl Owner {
    fn capacity(&self) -> usize {
        match self {
            Self::Byte(owner) => owner.capacity(),
            Self::Word(owner) => owner.capacity(),
            Self::Aligned(owner) => owner.capacity(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Byte(owner) => owner.len(),
            Self::Word(owner) => owner.len(),
            Self::Aligned(owner) => owner.len(),
        }
    }

    fn offset(&self) -> u32 {
        match self {
            Self::Byte(owner) => owner.offset().as_u32(),
            Self::Word(owner) => owner.offset().as_u32(),
            Self::Aligned(owner) => owner.offset().as_u32(),
        }
    }

    fn push(&mut self, value: u8) -> core::result::Result<(), compact_std::CoreError> {
        match self {
            Self::Byte(owner) => owner.push(value),
            Self::Word(owner) => owner.push(value as u64),
            Self::Aligned(owner) => owner.push(Aligned64(value)),
        }
    }

    fn get(&self, index: usize) -> Option<u8> {
        match self {
            Self::Byte(owner) => owner.get(index).copied(),
            Self::Word(owner) => owner.get(index).map(|value| *value as u8),
            Self::Aligned(owner) => owner.get(index).map(|value| value.0),
        }
    }

    fn xor(&mut self, index: usize, value: u8) {
        match self {
            Self::Byte(owner) => *owner.get_mut(index).unwrap() ^= value,
            Self::Word(owner) => *owner.get_mut(index).unwrap() ^= value as u64,
            Self::Aligned(owner) => owner.get_mut(index).unwrap().0 ^= value,
        }
    }

    fn try_resize(
        &mut self,
        capacity: usize,
    ) -> core::result::Result<bool, compact_std::CoreError> {
        match self {
            Self::Byte(owner) => owner.try_resize(capacity),
            Self::Word(owner) => owner.try_resize(capacity),
            Self::Aligned(owner) => owner.try_resize(capacity),
        }
    }

    fn validate(&self) -> core::result::Result<(), compact_std::CoreError> {
        match self {
            Self::Byte(owner) => CompactRuntime::validate_owned(owner),
            Self::Word(owner) => CompactRuntime::validate_owned(owner),
            Self::Aligned(owner) => CompactRuntime::validate_owned(owner),
        }
    }

    fn element_size(&self) -> usize {
        match self {
            Self::Byte(_) => core::mem::size_of::<u8>(),
            Self::Word(_) => core::mem::size_of::<u64>(),
            Self::Aligned(_) => core::mem::size_of::<Aligned64>(),
        }
    }

    fn alignment_is_valid(&self) -> bool {
        match self {
            Self::Byte(owner) => owner.as_slice().as_ptr() as usize % core::mem::align_of::<u8>() == 0,
            Self::Word(owner) => owner.as_slice().as_ptr() as usize % core::mem::align_of::<u64>() == 0,
            Self::Aligned(owner) => {
                owner.as_slice().as_ptr() as usize % core::mem::align_of::<Aligned64>() == 0
            }
        }
    }
}

fn new_owner(alignment: u8, capacity: usize) -> Option<Owner> {
    match alignment % 3 {
        0 => CompactRuntime::alloc_owned_slice::<u8>(capacity)
            .ok()
            .map(Owner::Byte),
        1 => CompactRuntime::alloc_owned_slice::<u64>(capacity)
            .ok()
            .map(Owner::Word),
        _ => CompactRuntime::alloc_owned_slice::<Aligned64>(capacity)
            .ok()
            .map(Owner::Aligned),
    }
}

fuzz_target!(|input: &[u8]| {
    common::init();
    let mut owners: Vec<Option<(Owner, Vec<u8>)>> =
        std::iter::repeat_with(|| None).take(16).collect();

    for operation in input.chunks_exact(4).take(512) {
        let index = operation[1] as usize % owners.len();
        match operation[0] % 4 {
            0 if owners[index].is_none() => {
                let capacity = 1 + operation[3] as usize % 32;
                if let Some(mut owner) = new_owner(operation[2], capacity) {
                    let initialized = (operation[0] as usize) % (capacity + 1);
                    let mut model = Vec::with_capacity(capacity);
                    for slot in 0..initialized {
                        let value = operation[2].wrapping_add(slot as u8);
                        owner.push(value).unwrap();
                        model.push(value);
                    }
                    owners[index] = Some((owner, model));
                }
            }
            1 => owners[index] = None,
            2 => {
                if let Some((owner, model)) = owners[index].as_mut() {
                    let old_capacity = owner.capacity();
                    let target = if operation[3] & 1 == 0 {
                        old_capacity + 1 + operation[2] as usize % 16
                    } else {
                        let shrinkable = old_capacity - model.len();
                        model.len() + operation[2] as usize % (shrinkable + 1)
                    };
                    let resized = owner.try_resize(target).unwrap_or(false);
                    if resized {
                        assert_eq!(owner.capacity(), target);
                    } else {
                        assert_eq!(owner.capacity(), old_capacity);
                    }
                    assert_eq!(owner.len(), model.len());
                }
            }
            _ => {
                if let Some((owner, model)) = owners[index].as_mut() {
                    if !model.is_empty() {
                        let slot = operation[2] as usize % model.len();
                        let value = operation[3];
                        owner.xor(slot, value);
                        model[slot] ^= value;
                    }
                }
            }
        }

        let mut live_ranges = Vec::new();
        let cage_capacity = CompactRuntime::capacity().unwrap() as u64;
        for (owner, model) in owners.iter().flatten() {
            owner.validate().unwrap();
            assert_eq!(owner.len(), model.len());
            assert!(owner.capacity() >= owner.len());
            assert!(owner.alignment_is_valid());
            for (index, expected) in model.iter().copied().enumerate() {
                assert_eq!(owner.get(index), Some(expected));
            }
            let start = owner.offset() as u64;
            let end = start + owner.capacity() as u64 * owner.element_size() as u64;
            assert!(start > 0 && end <= cage_capacity);
            if end > start {
                live_ranges.push((start, end));
            }
        }
        live_ranges.sort_unstable();
        for adjacent in live_ranges.windows(2) {
            assert!(adjacent[0].1 <= adjacent[1].0);
        }
        CompactRuntime::validate_allocator_state().unwrap();
    }
});
