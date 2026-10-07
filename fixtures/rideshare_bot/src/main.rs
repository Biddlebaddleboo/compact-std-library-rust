use compact_std::{CageConfig, CompactRing, CompactRuntime, CompactString, CompactValue};
use std::collections::VecDeque;

struct LogRecord {
    sequence: u64,
    message: CompactString,
}
// SAFETY: the record contains only a scalar and the compact string owner.
unsafe impl CompactValue for LogRecord {}

fn append_batch(
    ring: &mut CompactRing<LogRecord>,
    standard: &mut VecDeque<(u64, String)>,
    start: u64,
    count: u64,
) -> compact_std::Result<()> {
    for sequence in start..start + count {
        ring.push_back(LogRecord {
            sequence,
            message: CompactString::from_str("rideshare log event")?,
        })?;
        if standard.len() == 1_000 {
            standard.pop_front();
        }
        standard.push_back((sequence, String::from("rideshare log event")));
    }
    Ok(())
}

fn compare_snapshot(ring: &CompactRing<LogRecord>, standard: &VecDeque<(u64, String)>) {
    let compact_snapshot = ring
        .iter()
        .map(|record| (record.sequence, record.message.as_str().to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(
        compact_snapshot,
        standard.iter().cloned().collect::<Vec<_>>()
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    CompactRuntime::init(CageConfig::new(16 * 1024 * 1024))?;
    let mut ring = CompactRing::with_capacity(1_000)?;
    let mut standard = VecDeque::with_capacity(1_000);

    append_batch(&mut ring, &mut standard, 0, 1_250)?;
    assert_eq!(ring.len(), 1_000);
    assert_eq!(ring.front().unwrap().sequence, 250);
    compare_snapshot(&ring, &standard);

    let retained_capacity = ring.capacity();
    ring.clear();
    standard.clear();
    assert_eq!(ring.capacity(), retained_capacity);
    append_batch(&mut ring, &mut standard, 5_000, 1_150)?;
    compare_snapshot(&ring, &standard);

    ring.clear();
    standard.clear();
    append_batch(&mut ring, &mut standard, 9_000, 25)?;
    compare_snapshot(&ring, &standard);
    ring.clear();
    assert_eq!(ring.len(), 0);
    Ok(())
}
