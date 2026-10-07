use compact_std::{CompactStore, CompactValue};

struct DropRoot;

impl Drop for DropRoot {
    fn drop(&mut self) {}
}

// SAFETY: DropRoot has no address-sensitive state and can move before drop.
unsafe impl CompactValue for DropRoot {}

fn main() {
    let _ = CompactStore::<DropRoot>::build(256, |arena| arena.alloc_value(DropRoot));
}
