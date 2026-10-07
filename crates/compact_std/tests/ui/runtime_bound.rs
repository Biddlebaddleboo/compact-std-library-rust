use compact_std::compact;

fn build(maximum: u64) {
    #[compact]
    struct Bad {
        #[max = maximum]
        value: u64,
    }

    let mut storage = compact_std::StdBacking::with_capacity(64).unwrap();
    storage.with_arena(|arena| {
        let _ = Bad { value: 1 }.compact_in(arena);
    }).unwrap();
}

fn main() {}
