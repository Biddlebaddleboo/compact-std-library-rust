use compact_std::prelude::*;

fn main() {
    let _ = StdArena::with_capacity(128, |arena| -> compact_std::Result<()> {
        arena!(arena, {
            let mut values = Vec::new();
            let mut append = || {
                values.push(1)?;
                Ok::<(), compact_std::CollectionError>(())
            };
            append()?;
            Ok(())
        })
    });
}
