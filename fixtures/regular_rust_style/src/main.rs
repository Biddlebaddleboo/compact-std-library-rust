use compact_std::prelude::*;

const MAX_RETRIES: u64 = 7;

#[compact]
struct User {
    name: String,
    #[hot]
    active: bool,
    #[max = MAX_RETRIES]
    retries: u64,
    signed_balance: i32,
    state: State,
    #[cold]
    debug_label: String,
}

#[compact]
enum State {
    Idle,
    Working,
    Done,
}

#[compact(soa)]
#[repr(C)]
#[derive(Clone, Copy)]
struct Position {
    x: i32,
    y: i32,
    active: bool,
}

fn make_numbers<'arena>(
    arena: &mut Arena<'arena, '_>,
) -> Result<Vec<'arena, i32>> {
    Ok(Vec::new_in(arena))
}

fn main() -> std::result::Result<(), std::boxed::Box<dyn std::error::Error>> {
    StdArena::with_capacity(16 * 1024, |arena| -> Result<()> {
        arena!(arena, {
            let user = User {
                name: std::string::String::from("Ada"),
                active: true,
                retries: 3,
                signed_balance: -17,
                state: State::Idle,
                debug_label: std::string::String::from("imported"),
            };
            let compact_user = user.compact_in(arena)?;
            assert_eq!(compact_user.name(arena)?, "Ada");
            assert!(compact_user.active(arena)?);
            assert_eq!(compact_user.retries(arena)?, 3);
            assert_eq!(compact_user.signed_balance(arena)?, -17);
            assert!(matches!(compact_user.state(arena)?, State::Idle));
            assert_eq!(compact_user.debug_label(arena)?, "imported");
            compact_user.set_active(false, arena)?;
            assert!(!compact_user.active(arena)?);
            assert!(compact_user.set_retries(8, arena).is_err());
            compact_user.set_signed_balance(-32, arena)?;
            assert_eq!(compact_user.signed_balance(arena)?, -32);
            compact_user.set_state(State::Working, arena)?;
            assert!(matches!(compact_user.state(arena)?, State::Working));

            let mut values = Vec::new();
            values.push(10_i32)?;
            values.push(20_i32)?;
            assert_eq!(values.get(1)?, Some(&20));
            assert_eq!(values.iter()?.copied().sum::<i32>(), 30);
            values[0] = 11;
            assert_eq!(values[0], 11);

            let original = Vec::new();
            let mut moved_values = original;
            moved_values.push(17_i32)?;
            assert_eq!(moved_values[0], 17);

            let mut shadowed = Vec::new();
            {
                let shadowed = std::vec::Vec::<u8>::new();
                assert!(shadowed.is_empty());
            }
            shadowed.push(19_i32)?;

            let mut helper_values: Vec<'_, i32> = make_numbers(arena)?;
            helper_values.push(23_i32)?;
            assert_eq!(helper_values[0], 23);

            let mut reserved = Vec::with_capacity(2)?;
            reserved.push(1_u8)?;
            assert_eq!(reserved.get(0)?, Some(&1));

            let first_group = arena.alloc_slice(&[1_u16, 2, 3])?;
            let second_group = arena.alloc_slice(&[5_u16, 8])?;
            let mut nested = Vec::new();
            nested.push(first_group)?;
            nested.push(second_group)?;
            let nested_first = nested[0];
            assert_eq!(arena.get_slice(nested_first)?, &[1, 2, 3]);

            let mut message = String::new();
            message.push_str("compact")?;
            message.push_char('!')?;
            assert_eq!(message.as_str()?, "compact!");
            let greeting = String::from("hello")?;
            assert_eq!(greeting.as_str()?, "hello");
            let moving_message = String::from("moved string")?;
            let message_after_move = moving_message;
            assert_eq!(message_after_move.as_str()?, "moved string");

            let boxed = Box::new(55_u32)?;
            assert_eq!(*boxed.get(arena)?, 55);

            let compact_state = State::Working.compact_in(arena)?;
            assert!(matches!(compact_state.get(arena)?, State::Working));
            compact_state.set(State::Done, arena)?;
            assert!(matches!(compact_state.get(arena)?, State::Done));

            let mut positions = PositionSoa::new_in(arena);
            positions.push_in(Position { x: 4, y: 9, active: true }, arena)?;
            positions.push_in(Position { x: 2, y: 6, active: false }, arena)?;
            assert_eq!(positions.get(0, arena)?.unwrap().x, 4);
            assert_eq!(positions.get(1, arena)?.unwrap().active, false);
            Ok(())
        })
    })??;

    let array_of_structs_bytes = StdArena::with_capacity(4096, |arena| -> Result<usize> {
        let mut records = Vec::new_in(arena);
        for index in 0..32 {
            records.push_in(
                Position { x: index, y: index * 2, active: index % 2 == 0 },
                arena,
            )?;
        }
        Ok(arena.used_bytes())
    })??;
    let struct_of_arrays_bytes = StdArena::with_capacity(4096, |arena| -> Result<usize> {
        let mut records = PositionSoa::with_capacity_in(32, arena)?;
        for index in 0..32 {
            records.push_in(Position { x: index, y: index * 2, active: index % 2 == 0 }, arena)?;
        }
        Ok(arena.used_bytes())
    })??;
    assert!(
        struct_of_arrays_bytes < array_of_structs_bytes,
        "SoA used {struct_of_arrays_bytes} bytes; array-of-structs used {array_of_structs_bytes}"
    );
    Ok(())
}
