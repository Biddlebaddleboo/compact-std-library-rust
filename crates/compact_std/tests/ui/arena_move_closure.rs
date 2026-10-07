use compact_std::prelude::*;

fn main() {
    let _ = StdArena::with_capacity(128, |arena| -> compact_std::Result<()> {
        arena!(arena, {
            let values = Vec::new();
            let read = move || values.len();
            let _ = read();
            Ok(())
        })
    });
}
