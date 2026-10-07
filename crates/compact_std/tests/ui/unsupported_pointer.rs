use compact_std::compact;

#[compact]
struct Bad {
    pointer: *const u8,
}

fn main() {}
