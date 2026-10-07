use compact_backend_std::StdArena;
use compact_core::Offset32;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let answer = StdArena::with_capacity(1024, |arena| -> compact_core::Result<u32> {
        let value: Offset32<'_, u32> = arena.alloc_value(41)?;
        *arena.get_mut(value)? += 1;
        Ok(*arena.get(value)?)
    })??;

    println!("compact arena value: {answer}");
    Ok(())
}
