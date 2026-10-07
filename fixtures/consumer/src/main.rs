use compact_backend_std::StdArena;
use compact_core::{Offset32, Result as CoreResult};

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let doubled = StdArena::with_capacity(128, |arena| -> CoreResult<u64> {
        let value: Offset32<'_, u64> = arena.alloc_value(21)?;
        Ok(*arena.get(value)? * 2)
    })??;

    assert_eq!(doubled, 42);
    Ok(())
}
