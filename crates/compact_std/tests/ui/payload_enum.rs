use compact_std::compact;

#[compact]
enum Bad {
    Empty,
    Full(u32),
}

fn main() {}
