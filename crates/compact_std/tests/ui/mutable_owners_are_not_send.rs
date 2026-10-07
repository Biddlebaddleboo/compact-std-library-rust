use compact_std::{Arena, ArenaAllocation, CompactStore, Vec};

fn assert_send<T: Send>() {}

fn main() {
    assert_send::<Arena<'static, 'static>>();
    assert_send::<ArenaAllocation<'static, u32>>();
    assert_send::<Vec<'static, u32>>();
    assert_send::<CompactStore<u32>>();
}
