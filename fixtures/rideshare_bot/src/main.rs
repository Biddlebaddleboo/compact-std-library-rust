use std::collections::VecDeque;

use compact_std::{CompactRing, CompactString, CompactValue, StdArena};

struct LogRecord<'arena> {
    sequence: u64,
    message: CompactString<'arena>,
}

// SAFETY: both fields are movable compact values with ordinary drop behavior.
unsafe impl CompactValue for LogRecord<'_> {}

fn append_batch<'arena>(
    ring: &mut CompactRing<'arena, LogRecord<'arena>>,
    standard: &mut VecDeque<(u64, String)>,
    start: u64,
    count: u64,
    arena: &mut compact_std::Arena<'arena, '_>,
) -> compact_std::Result<()> {
    for sequence in start..start + count {
        let message = CompactString::from_str_in("rideshare log event", arena)?;
        ring.push_back(
            LogRecord { sequence, message },
            arena,
        )?;
        if standard.len() == 1_000 {
            standard.pop_front();
        }
        standard.push_back((sequence, String::from("rideshare log event")));
    }
    Ok(())
}

fn compare_snapshot<'arena>(
    ring: &CompactRing<'arena, LogRecord<'arena>>,
    standard: &VecDeque<(u64, String)>,
    arena: &compact_std::Arena<'arena, '_>,
) -> compact_std::Result<()> {
    let compact_snapshot = ring
        .iter(arena)?
        .map(|record| {
            Ok((
                record.sequence,
                record.message.as_str(arena)?.to_owned(),
            ))
        })
        .collect::<compact_std::Result<Vec<_>>>()?;
    let standard_snapshot = standard.iter().cloned().collect::<Vec<_>>();
    assert_eq!(compact_snapshot, standard_snapshot);
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    StdArena::with_capacity(128 * 1024, |arena| -> compact_std::Result<()> {
        let mut ring = CompactRing::with_capacity(1_000, arena)?;
        let mut standard = VecDeque::with_capacity(1_000);

        append_batch(&mut ring, &mut standard, 0, 1_250, arena)?;
        assert_eq!(ring.len(), 1_000);
        assert_eq!(ring.front(arena)?.unwrap().sequence, 250);
        compare_snapshot(&ring, &standard, arena)?;

        let retained_capacity = ring.capacity();
        ring.clear();
        standard.clear();
        assert_eq!(ring.capacity(), retained_capacity);
        append_batch(&mut ring, &mut standard, 5_000, 1_150, arena)?;
        compare_snapshot(&ring, &standard, arena)?;

        ring.clear();
        standard.clear();
        append_batch(&mut ring, &mut standard, 9_000, 25, arena)?;
        compare_snapshot(&ring, &standard, arena)?;
        ring.clear();
        assert_eq!(ring.len(), 0);
        Ok(())
    })??;

    Ok(())
}
