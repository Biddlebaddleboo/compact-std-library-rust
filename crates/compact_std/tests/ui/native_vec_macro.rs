use compact_std::{arena, StdBacking};

fn main() {
    let mut storage = StdBacking::with_capacity(64).unwrap();
    storage.with_arena(|arena| {
        arena!(arena, {
            let _values = vec![1, 2, 3];
        })
    }).unwrap();
}
