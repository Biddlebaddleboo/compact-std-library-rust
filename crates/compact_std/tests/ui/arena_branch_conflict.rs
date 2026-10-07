use compact_std::prelude::*;

fn main() {
    let _ = StdArena::with_capacity(128, |arena| -> compact_std::Result<()> {
        arena!(arena, {
            let mut values = Vec::new();
            if true {
                values = Vec::new();
            } else {
                values = std::vec::Vec::new();
            }
            values.push(1)?;
            Ok(())
        })
    });
}
