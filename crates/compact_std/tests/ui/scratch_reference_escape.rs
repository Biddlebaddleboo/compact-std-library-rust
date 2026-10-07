use compact_std::StdBacking;

fn main() {
    let mut backing = StdBacking::with_capacity(1024).unwrap();
    let escaped = backing
        .with_arena(|arena| {
            arena
                .scratch(256, |scratch| {
                    let value = scratch.alloc_value(7_u32).unwrap();
                    scratch.get(value).unwrap()
                })
                .unwrap()
        })
        .unwrap();
    println!("{escaped}");
}
