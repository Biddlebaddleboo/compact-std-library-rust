use compact_std::compact;

#[compact]
struct Bad<T> {
    value: u32,
    marker: core::marker::PhantomData<T>,
}

fn main() {}
