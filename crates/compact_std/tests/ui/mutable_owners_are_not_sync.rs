use compact_std::{Arena, ArenaAllocation, CompactStore, Vec};

fn assert_sync<T: Sync>() {}

fn main() {
    assert_sync::<Arena<'static, 'static>>();
    assert_sync::<ArenaAllocation<'static, u32>>();
    assert_sync::<Vec<'static, u32>>();
    assert_sync::<CompactStore<u32>>();
}
