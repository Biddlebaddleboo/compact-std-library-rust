use compact_std::prelude::*;
use std::error::Error;
use std::sync::{Arc, Barrier};
use std::thread;

type AppResult<T> = std::result::Result<T, std::boxed::Box<dyn Error>>;

#[derive(CompactDeserialize, CompactFreeze)]
#[serde(deny_unknown_fields)]
struct ServiceConfig<'arena> {
    system_id: CompactString<'arena>,
    config_path: CompactPathBuf<'arena>,
    names: CompactVec<'arena, CompactString<'arena>>,
    labels: CompactHashMap<'arena, CompactString<'arena>, CompactString<'arena>>,
    features: CompactHashSet<'arena, CompactString<'arena>>,
}

struct LogRecord<'arena> {
    sequence: u64,
    message: CompactString<'arena>,
    argument: CompactString<'arena>,
}

// SAFETY: each field is movable and its compact string owners carry all
// allocation and drop behavior required by the arena lifetime.
unsafe impl CompactValue for LogRecord<'_> {}

fn exercise_log_ring() -> AppResult<()> {
    StdArena::with_capacity(64 * 1024, |arena| -> Result<()> {
        let mut ring = VecDeque::with_capacity_in(8, arena)?;
        for sequence in 0..12 {
            let message = CompactString::from_str_in("request completed", arena)?;
            let argument = CompactString::from_str_in("status=200", arena)?;
            if ring.len() == ring.capacity() {
                let _ = ring.pop_front(arena)?;
            }
            ring.push_back(
                LogRecord {
                    sequence,
                    message,
                    argument,
                },
                arena,
            )?;
        }

        let mut snapshot = std::vec::Vec::new();
        for entry in ring.iter(arena)? {
            snapshot.push((
                entry.sequence,
                entry.message.as_str(arena)?.to_owned(),
                entry.argument.as_str(arena)?.to_owned(),
            ));
        }
        assert_eq!(snapshot.len(), 8);
        assert_eq!(snapshot.first().unwrap().0, 4);
        assert_eq!(snapshot.last().unwrap().0, 11);
        assert_eq!(snapshot[0].1, "request completed");
        assert_eq!(snapshot[0].2, "status=200");
        Ok(())
    })??;
    Ok(())
}

fn reassemble_chunks<'arena>(
    count: usize,
    chunks: &[(usize, &[u8])],
    arena: &mut Arena<'arena, '_>,
) -> Result<CompactBytes<'arena>> {
    let mut slots: CompactVec<'arena, Option<CompactBytes<'arena>>> =
        CompactVec::with_capacity_in(count, arena)?;
    for _ in 0..count {
        slots.push_in(None, arena)?;
    }

    for &(index, bytes) in chunks {
        let empty = {
            let slot = slots
                .get(index, arena)?
                .ok_or(CollectionError::InvalidCompactValue)?;
            if let Some(existing) = slot {
                if existing.as_slice() != bytes {
                    return Err(CollectionError::InvalidCompactValue);
                }
                false
            } else {
                true
            }
        };
        if empty {
            let chunk = CompactBytes::from_slice_in(bytes, arena)?;
            *slots
                .get_mut(index, arena)?
                .ok_or(CollectionError::InvalidCompactValue)? = Some(chunk);
        }
    }

    let mut body = CompactBytes::new_in(arena);
    for slot in slots.as_slice(arena)? {
        let bytes = slot
            .as_ref()
            .ok_or(CollectionError::InvalidCompactValue)?;
        body.extend_from_slice(bytes.as_slice(), arena)?;
    }
    Ok(body)
}

fn exercise_chunk_reassembly() -> AppResult<()> {
    StdArena::with_capacity(32 * 1024, |arena| -> Result<()> {
        let chunks: &[(usize, &[u8])] = &[
            (2, b"!"),
            (0, b"hel"),
            (1, b"lo"),
            (1, b"lo"), // duplicate delivery is idempotent
        ];
        let body = reassemble_chunks(3, chunks, arena)?;
        assert_eq!(body.as_slice(), b"hello!");
        Ok(())
    })??;
    Ok(())
}

fn load_and_share_config() -> AppResult<()> {
    let input = include_str!("../service.toml");
    let (frozen, root) = StdArena::with_capacity(64 * 1024, |arena| {
        let config = compact_std::toml::from_str_in::<ServiceConfig<'_>>(input, arena)
            .map_err(|_| CollectionError::InvalidCompactValue)?;
        assert_eq!(config.system_id.as_str(arena)?, "rideshare-edge");
        let graph = freeze_in(&config, arena)
            .map_err(|_| CollectionError::InvalidCompactValue)?;
        Ok::<_, CollectionError>(graph)
    })??;

    let shared = Arc::new(frozen);
    let barrier = Arc::new(Barrier::new(4));
    let readers: std::vec::Vec<_> = (0..4)
        .map(|_| {
            let arena = Arc::clone(&shared);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let config = root.get(&arena).unwrap();
                assert_eq!(config.system_id().as_str(&arena).unwrap(), "rideshare-edge");
                assert_eq!(config.names().len(), 3);
                assert_eq!(
                    config.names().get(1, &arena).unwrap().unwrap().as_str(&arena).unwrap(),
                    "dispatcher"
                );
                assert!(config.features().contains_by(&arena, |feature| {
                    feature
                        .as_str(&arena)
                        .map(|feature| feature == "chunk-reassembly")
                        .unwrap_or(false)
                }).unwrap());
                assert_eq!(
                    config.labels().find_by(&arena, |key| {
                        key.as_str(&arena)
                            .map(|key| key == "region")
                            .unwrap_or(false)
                    }).unwrap().map(|(_, value)| value.as_str(&arena).unwrap()),
                    Some("us-west")
                );
                assert_eq!(
                    config.config_path().to_path_buf(&arena).unwrap(),
                    std::path::Path::new("/etc/rideshare/service.toml")
                );
            })
        })
        .collect();
    for reader in readers {
        reader.join().unwrap();
    }
    Ok(())
}

fn exercise_scratch_packet() -> AppResult<()> {
    let packet = StdArena::with_capacity(16 * 1024, |arena| -> Result<std::vec::Vec<u8>> {
        let packet = arena.scratch(8 * 1024, |scratch| -> Result<std::vec::Vec<u8>> {
            let mut bytes = CompactBytes::new_in(scratch);
            bytes.extend_from_slice_in(&[0xCA, 0xFE], scratch)?;
            let line = format_in!(scratch, "payload={} bytes", 12_u32)?;
            let rendered = line.as_bytes(scratch)?.to_vec();
            bytes.extend_from_slice_in(&rendered, scratch)?;
            Ok(bytes.as_slice().to_vec())
        })??;
        Ok(packet)
    })??;
    assert_eq!(&packet[..2], &[0xCA, 0xFE]);
    assert!(packet.ends_with(b"payload=12 bytes"));
    Ok(())
}

fn exercise_ordinary_arena_syntax() -> AppResult<()> {
    StdArena::with_capacity(32 * 1024, |arena| -> Result<()> {
        arena!(arena, {
            let mut numbers = vec![1_u32, 2, 3];
            let copied = numbers.clone();
            numbers.push(4)?;
            let collected = [5_u32, 6].into_iter().collect::<Vec<_>>();
            assert_eq!(copied.as_slice()?, &[1, 2, 3]);
            assert_eq!(numbers.as_slice()?, &[1, 2, 3, 4]);
            assert_eq!(collected.as_slice()?, &[5, 6]);

            let name = String::from("worker")?;
            let cloned_name = name.clone();
            let rendered_name = name.to_string();
            let formatted = format!("{}:{}", cloned_name, numbers.len())?;
            assert_eq!(rendered_name.as_str()?, "worker");
            assert_eq!(formatted.as_str()?, "worker:4");

            let mut counts = HashMap::new();
            counts.insert(String::from("worker")?, 4_u32)?;
            assert_eq!(counts.get("worker")?, Some(&4));

            let mut health = HashSet::new();
            health.insert(String::from("ready")?)?;
            assert!(health.contains("ready")?);

            let mut events = VecDeque::new();
            events.push_back(String::from("connected")?)?;
            events.push_front(String::from("started")?)?;
            let started: String<'_> = events.pop_front()?.unwrap();
            assert_eq!(started.as_str()?, "started");
            assert_eq!(events.front(arena)?.unwrap().as_str(arena)?, "connected");

            let mut path = PathBuf::from("var/log")?;
            path.push("rideshare")?;
            assert!(path.ends_with("rideshare"));
            Ok(())
        })
    })??;
    Ok(())
}

fn main() -> AppResult<()> {
    exercise_log_ring()?;
    exercise_chunk_reassembly()?;
    load_and_share_config()?;
    exercise_scratch_packet()?;
    exercise_ordinary_arena_syntax()?;
    Ok(())
}
