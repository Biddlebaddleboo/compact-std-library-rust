use compact_std::prelude::*;

fn make_values<'arena>(
    arena: &mut Arena<'arena, '_>,
) -> compact_std::Result<compact_std::Vec<'arena, u32>> {
    Ok(compact_std::Vec::new_in(arena))
}

fn main() -> std::result::Result<(), std::boxed::Box<dyn std::error::Error>> {
    StdArena::with_capacity(4096, |arena| -> compact_std::Result<()> {
        arena!(arena, {
            let original = Vec::new();
            let mut moved = original;
            moved.push(7_u32)?;
            assert_eq!(moved[0], 7);

            let mut nested_item = Vec::new();
            nested_item.push(29_u32)?;
            let mut nested_values = Vec::new();
            nested_values.push(nested_item)?;
            assert_eq!(
                nested_values
                    .get(0, arena)?
                    .unwrap()
                    .as_slice(arena)?[0],
                29
            );

            let mut boxed_item = Vec::new();
            boxed_item.push(33_u32)?;
            let boxed = Box::new(boxed_item)?;
            assert_eq!(boxed.get(arena)?.len(), 1);

            let mut shadowing = Vec::new();
            shadowing.push(3_u8)?;
            {
                let shadowing = std::vec::Vec::<u8>::new();
                assert!(shadowing.is_empty());
            }
            shadowing.push(4_u8)?;

            let mut reassigned: Vec<'_, u32>;
            if true {
                reassigned = Vec::with_capacity(2)?;
            } else {
                reassigned = Vec::new();
            }
            reassigned.push(9_u32)?;
            assert_eq!(reassigned.get(0, arena)?, Some(&9));

            let (mut pair, mut text) = (Vec::new(), String::new());
            pair.push(11_i32)?;
            text.push_str("tuple")?;
            assert_eq!(pair[0], 11);
            assert_eq!(text.as_str()?, "tuple");

            let match_pair = (Vec::new(), String::new());
            match match_pair {
                (mut match_values, mut match_text) => {
                    match_values.push(12_i32)?;
                    match_text.push_str("match")?;
                    assert_eq!(match_values[0], 12);
                    assert_eq!(match_text.as_str()?, "match");
                }
            }

            let rest_pair = (Vec::new(), 5_u8, Vec::new());
            let (mut first_values, .., mut last_values) = rest_pair;
            first_values.push(14_i32)?;
            last_values.push(15_i32)?;

            let helper: Vec<'_, u32> = make_values(arena)?;
            let mut helper = helper;
            helper.push(13)?;
            assert_eq!(helper[0], 13);

            let reader = || -> compact_std::Result<usize> {
                Ok(shadowing.as_slice()?.len())
            };
            assert_eq!(reader()?, 2);

            let index_error = || -> compact_std::Result<u32> { Ok(moved[usize::MAX]) };
            assert!(index_error().is_err());

            for _ in 0..2 {
                let native = std::vec::Vec::<u16>::new();
                assert!(native.is_empty());
            }
            let mut iterations = 0;
            while iterations < 2 {
                moved.push(iterations)?;
                iterations += 1;
            }
            assert_eq!(moved[2], 1);

            let mut explicit = Vec::new_in(arena);
            explicit.push_in(21_u8, arena)?;
            assert_eq!(explicit.get(0, arena)?, Some(&21));

            let message = String::from("move me")?;
            let moved_message = message;
            assert_eq!(moved_message.as_str()?, "move me");

            let mut string_shadow = String::new();
            {
                let string_shadow = std::string::String::new();
                assert!(string_shadow.is_empty());
            }
            string_shadow.push_char('!')?;

            let mut native = std::vec::Vec::<u8>::new();
            assert!(native.is_empty());
            native = std::vec::Vec::new();
            native.push(1);
            assert_eq!(native, [1]);

            let boxed = Box::new(31_u32)?;
            assert_eq!(*boxed.get(arena)?, 31);
            Ok(())
        })
    })??;
    Ok(())
}
