use compact_std::prelude::*;

fn make_values<'arena>(
    arena: &mut Arena<'arena, '_>,
) -> compact_std::Result<compact_std::Vec<'arena, u32>> {
    Ok(compact_std::Vec::new_in(arena))
}

fn main() {
    let _ = StdArena::with_capacity(128, |arena| -> compact_std::Result<()> {
        arena!(arena, {
            let mut values = make_values(arena)?;
            values.push(1)?;
            Ok(())
        })
    });
}
