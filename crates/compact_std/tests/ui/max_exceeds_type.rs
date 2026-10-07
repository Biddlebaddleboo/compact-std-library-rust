use compact_std::compact;

#[compact]
struct Bad {
    #[max = 300]
    value: u8,
}

fn main() {
    let mut storage = compact_std::StdBacking::with_capacity(64).unwrap();
    storage.with_arena(|arena| {
        let _ = Bad { value: 1 }.compact_in(arena);
    }).unwrap();
}
