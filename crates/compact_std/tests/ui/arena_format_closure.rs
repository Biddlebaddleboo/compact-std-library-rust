use compact_std::prelude::*;

fn main() {
    StdArena::with_capacity(128, |arena| -> Result<()> {
        arena!(arena, {
            let make_message = || format!("inside closure");
            let _message = make_message();
            Ok(())
        })
    })
    .unwrap()
    .unwrap();
}
