//! Frozen executable example of the documented V2.1.0 source contract.
//!
//! Keep this fixture on explicit APIs and syntax documented by V2.1.0. Do not
//! update it to use later shorthand when the facade grows new conveniences.

use compact_std::prelude::*;
use std::error::Error;

const MAX_RETRIES: u64 = 7;

#[compact]
struct Job {
    #[max = MAX_RETRIES]
    retries: u64,
    active: bool,
    name: String,
    state: State,
}

#[compact]
enum State {
    Idle,
    Running,
}

#[compact(soa)]
#[repr(C)]
#[derive(Clone, Copy)]
struct Position {
    x: i32,
    y: i32,
    active: bool,
}

fn main() -> std::result::Result<(), std::boxed::Box<dyn Error>> {
    assert_eq!(std::mem::size_of::<Offset32<'static, u64>>(), 4);
    assert!(Offset32::<u64>::null().is_null());
    assert_eq!(compact_std::NULL_OFFSET, 0);

    StdArena::with_capacity(16 * 1024, |arena| -> Result<()> {
        // This is the V2.1.0 README example and remains intentionally stable.
        let macro_example: Result<()> = arena!(arena, {
            let mut values = Vec::new();
            values.push(10_u32)?;
            values.push(20)?;
            assert_eq!(values.get(1)?, Some(&20));

            let mut text = String::from("hello")?;
            text.push_str(" compact")?;
            assert_eq!(text.as_str()?, "hello compact");

            let boxed = Box::new(42_u32)?;
            assert_eq!(*boxed.get(arena)?, 42);
            Ok(())
        });
        macro_example?;

        // Explicit V2.1.0 arena operations remain a supported source path.
        let offset: Offset32<'_, u64> = arena.alloc_value(21)?;
        assert_eq!(*arena.get(offset)?, 21);
        let mut explicit = Vec::new_in(arena);
        explicit.push_in(5_u16, arena)?;
        assert_eq!(explicit.as_slice(arena)?, &[5]);

        let text = CompactString::from_str_in("explicit", arena)?;
        assert_eq!(text.as_str(arena)?, "explicit");
        // The documented #[compact] generated handle remains source-compatible.
        let native = Job {
            retries: 3,
            active: true,
            name: std::string::String::from("worker"),
            state: State::Idle,
        };
        let compact = native.compact_in(arena)?;
        assert_eq!(compact.retries(arena)?, 3);
        assert!(compact.active(arena)?);
        assert_eq!(compact.name(arena)?, "worker");
        assert!(matches!(compact.state(arena)?, State::Idle));
        compact.set_state(State::Running, arena)?;
        assert!(matches!(compact.state(arena)?, State::Running));

        let mut positions = PositionSoa::new_in(arena);
        positions.push_in(
            Position {
                x: 4,
                y: 9,
                active: true,
            },
            arena,
        )?;
        assert_eq!(positions.get(0, arena)?.unwrap().x, 4);
        assert!(positions.get(0, arena)?.unwrap().active);

        assert!(Vec::<u8>::with_capacity_in(usize::MAX, arena).is_err());
        Ok(())
    })??;

    Ok(())
}
