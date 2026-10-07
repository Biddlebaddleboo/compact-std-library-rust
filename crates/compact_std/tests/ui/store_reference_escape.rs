use compact_std::CompactStore;

fn main() {
    let mut store = CompactStore::<u32>::build(256, |arena| arena.alloc_value(7_u32)).unwrap();
    let escaped = store
        .with(|arena, root| root.get(arena).unwrap())
        .unwrap();
    println!("{escaped}");
}
