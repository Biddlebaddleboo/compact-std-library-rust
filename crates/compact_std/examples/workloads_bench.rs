use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use compact_std::prelude::*;
use compact_std::{format_in, Result as CompactResult};

const ARENA_CAPACITY: usize = 16 * 1024 * 1024;
const SERVICE_TOML: &str = r#"
service = "edge-router"
paths = ["/etc/edge/router.toml", "/var/log/edge/router.log", "/run/edge/router.pid"]
labels = ["api", "ingest", "control", "metrics"]
"#;

struct CountingSystem;

static ALLOCATION_EVENTS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every allocation operation is forwarded to `System` with the same
// valid layout and pointer, while atomics only record measurement counters.
unsafe impl GlobalAlloc for CountingSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_EVENTS.fetch_add(1, Ordering::Relaxed);
        ALLOCATION_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: `layout` is supplied by the global allocator contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_EVENTS.fetch_add(1, Ordering::Relaxed);
        ALLOCATION_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: `layout` is supplied by the global allocator contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller gives the original pointer and layout for a live
        // allocation created by this allocator.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATION_EVENTS.fetch_add(1, Ordering::Relaxed);
        ALLOCATION_BYTES.fetch_add(new_size, Ordering::Relaxed);
        // SAFETY: the caller gives a live allocation and valid replacement
        // size according to the global allocator contract.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingSystem = CountingSystem;

#[derive(Clone, Copy)]
struct AllocationSnapshot {
    events: usize,
    bytes: usize,
}

impl AllocationSnapshot {
    fn now() -> Self {
        Self {
            events: ALLOCATION_EVENTS.load(Ordering::Relaxed),
            bytes: ALLOCATION_BYTES.load(Ordering::Relaxed),
        }
    }

    fn delta(self, earlier: Self) -> (usize, usize) {
        (self.events - earlier.events, self.bytes - earlier.bytes)
    }
}

#[derive(Default)]
struct Observation {
    payload_bytes: usize,
    high_water: usize,
    nested_high_water: usize,
    fragmented_ranges: usize,
    fragmented_free_bytes: usize,
}

fn report(
    name: &str,
    elapsed: Duration,
    payload_bytes: usize,
    arena_current: usize,
    arena_high_water: usize,
    arena_free: usize,
    fragmented_ranges: usize,
    fragmented_free_bytes: usize,
    allocations: (usize, usize),
    nested_high_water: usize,
) {
    println!(
        "{name}: payload={payload_bytes}B arena_current={arena_current}B arena_high_water={arena_high_water}B nested_arena_high_water={nested_high_water}B arena_overhead_estimate={}B free={arena_free}B fragmented_ranges={fragmented_ranges} fragmented_free={fragmented_free_bytes}B native_alloc_events={} native_alloc_bytes={}B runtime={elapsed:?}",
        arena_high_water.saturating_sub(payload_bytes),
        allocations.0,
        allocations.1,
    );
}

fn measure_arena<F>(name: &str, operation: F)
where
    F: for<'arena, 'memory> FnOnce(&mut Arena<'arena, 'memory>) -> Observation,
{
    let allocations_before = AllocationSnapshot::now();
    let started = Instant::now();
    StdArena::with_capacity(ARENA_CAPACITY, |arena| {
        let initial_used = arena.used_bytes();
        let observation = operation(arena);
        let elapsed = started.elapsed();
        let current = arena.used_bytes().saturating_sub(initial_used);
        let high_water = observation
            .high_water
            .saturating_sub(initial_used)
            .max(current);
        let native_allocations = AllocationSnapshot::now().delta(allocations_before);
        report(
            name,
            elapsed,
            observation.payload_bytes,
            current,
            high_water,
            arena.remaining_bytes(),
            observation.fragmented_ranges,
            observation.fragmented_free_bytes,
            native_allocations,
            observation.nested_high_water,
        );
    })
    .expect("benchmark arena construction succeeds");
}

#[derive(CompactDeserialize, CompactFreeze)]
struct ServiceConfig<'arena> {
    service: CompactString<'arena>,
    paths: CompactVec<'arena, CompactPathBuf<'arena>>,
    labels: CompactVec<'arena, CompactString<'arena>>,
}

#[derive(CompactFreeze)]
struct FrozenCatalog<'arena> {
    service: CompactString<'arena>,
    base_path: CompactPathBuf<'arena>,
    names: CompactVec<'arena, CompactString<'arena>>,
}

fn benchmark_vecdeque() {
    measure_arena("VecDeque FIFO", |arena| {
        let mut queue = CompactVecDeque::with_capacity(1024, arena).unwrap();
        for value in 0..1_000_u64 {
            queue.push_back(value, arena).unwrap();
        }
        let payload_bytes = queue.len() * core::mem::size_of::<u64>();
        for expected in 0..250_u64 {
            assert_eq!(queue.pop_front(arena).unwrap(), Some(expected));
            queue.push_back(expected + 1_000, arena).unwrap();
        }
        let checksum = queue.iter(arena).unwrap().copied().sum::<u64>();
        black_box(checksum);
        Observation {
            payload_bytes,
            high_water: arena.used_bytes(),
            ..Observation::default()
        }
    });
}

fn benchmark_vec() {
    measure_arena("Vec growth and traversal", |arena| {
        let mut values = CompactVec::new_in(arena);
        for value in 0..10_000_u64 {
            values.push_in(value, arena).unwrap();
        }
        let checksum = values.as_slice(arena).unwrap().iter().copied().sum::<u64>();
        black_box(checksum);
        Observation {
            payload_bytes: values.len() * core::mem::size_of::<u64>(),
            high_water: arena.used_bytes(),
            ..Observation::default()
        }
    });
}

fn benchmark_log_ring() {
    measure_arena("bounded ring / 1,000 log entries", |arena| {
        let mut ring = CompactRing::with_capacity(1_000, arena).unwrap();
        let mut payload_bytes = 0;
        for index in 0..1_000_u32 {
            let line = format_in!(arena, "[{index:04}] service=edge-router status=ready").unwrap();
            payload_bytes += line.len();
            ring.push_back(line, arena).unwrap();
        }
        let checksum = ring
            .iter(arena)
            .unwrap()
            .map(|line| line.as_str(arena).unwrap().len())
            .sum::<usize>();
        black_box(checksum);
        Observation {
            payload_bytes,
            high_water: arena.used_bytes(),
            ..Observation::default()
        }
    });
}

fn benchmark_chunk_transfer() {
    measure_arena("chunk transfer / 1,024 fragments", |arena| {
        const CHUNK_BYTES: usize = 256;
        const CHUNK_COUNT: usize = 1_024;
        let mut chunks = CompactVec::with_capacity_in(CHUNK_COUNT, arena).unwrap();
        for sequence in 0..CHUNK_COUNT {
            let fragment = [sequence as u8; CHUNK_BYTES];
            chunks
                .push_in(
                    Some(CompactBytes::from_slice_in(&fragment, arena).unwrap()),
                    arena,
                )
                .unwrap();
        }
        let mut body = CompactBytes::with_capacity_in(CHUNK_COUNT * CHUNK_BYTES, arena).unwrap();
        for fragment in chunks.as_slice(arena).unwrap().iter().flatten() {
            body.extend_from_slice_in(fragment.as_slice(), arena)
                .unwrap();
        }
        black_box(body.as_slice());
        Observation {
            payload_bytes: CHUNK_COUNT * CHUNK_BYTES + body.len(),
            high_water: arena.used_bytes(),
            ..Observation::default()
        }
    });
}

fn benchmark_hash_collections() {
    measure_arena("HashMap + HashSet", |arena| {
        let mut map = CompactHashMap::with_capacity(1_000, arena).unwrap();
        let mut set = CompactHashSet::with_capacity(1_000, arena).unwrap();
        for value in 0..1_000_u32 {
            map.insert(value, u64::from(value) * 17, arena).unwrap();
            set.insert(value, arena).unwrap();
        }
        let checksum = map
            .iter(arena)
            .unwrap()
            .map(|(key, value)| u64::from(*key) + value)
            .sum::<u64>();
        black_box((checksum, set.len()));
        Observation {
            payload_bytes: 1_000 * (core::mem::size_of::<u32>() * 2 + core::mem::size_of::<u64>()),
            high_water: arena.used_bytes(),
            ..Observation::default()
        }
    });
}

fn benchmark_string() {
    measure_arena("String append", |arena| {
        let mut text = CompactString::empty();
        for _ in 0..1_000 {
            text.push_str_in("service-label/edge-router;", arena)
                .unwrap();
        }
        black_box(text.as_str(arena).unwrap());
        Observation {
            payload_bytes: text.len(),
            high_water: arena.used_bytes(),
            ..Observation::default()
        }
    });
}

fn benchmark_bytes() {
    measure_arena("Bytes / 64 KiB", |arena| {
        let source = [0xA5_u8; 4_096];
        let mut bytes = CompactBytes::with_capacity_in(64 * 1024, arena).unwrap();
        for _ in 0..16 {
            bytes.extend_from_slice_in(&source, arena).unwrap();
        }
        black_box(bytes.as_slice());
        Observation {
            payload_bytes: bytes.len(),
            high_water: arena.used_bytes(),
            ..Observation::default()
        }
    });
}

fn benchmark_paths() {
    measure_arena("PathBuf / 1,000 service paths", |arena| {
        let path = Path::new("/var/lib/edge-router/shards/current.toml");
        let mut paths = CompactVec::with_capacity_in(1_000, arena).unwrap();
        let mut payload_bytes = 0;
        for _ in 0..1_000 {
            payload_bytes += path.as_os_str().len();
            paths
                .push_in(CompactPathBuf::from_path(path, arena).unwrap(), arena)
                .unwrap();
        }
        black_box(paths.get(999, arena).unwrap().unwrap().as_path());
        Observation {
            payload_bytes,
            high_water: arena.used_bytes(),
            ..Observation::default()
        }
    });
}

fn benchmark_serde_config() {
    measure_arena("Serde TOML service config", |arena| {
        let config = compact_std::toml::from_str_in::<ServiceConfig<'_>>(SERVICE_TOML, arena)
            .expect("service TOML parses");
        let mut payload_bytes = config.service.len();
        for path in config.paths.as_slice(arena).unwrap() {
            payload_bytes += path.as_os_str().len();
        }
        for label in config.labels.as_slice(arena).unwrap() {
            payload_bytes += label.len();
        }
        black_box((config.service.as_str(arena).unwrap(), config.paths.len()));
        Observation {
            payload_bytes,
            high_water: arena.used_bytes(),
            ..Observation::default()
        }
    });
}

fn benchmark_scratch() {
    measure_arena("scratch temporary vectors and formatting", |arena| {
        const SCRATCH_BYTES: usize = 64 * 1024;
        let before = arena.used_bytes();
        let (payload_bytes, scratch_high_water) = arena
            .scratch(SCRATCH_BYTES, |scratch| -> CompactResult<(usize, usize)> {
                let mut values = CompactVec::with_capacity_in(512, scratch)?;
                for value in 0..512_u32 {
                    values.push_in(value, scratch)?;
                }
                let message = format_in!(scratch, "temporary values={}", values.len())?;
                let payload = values.len() * core::mem::size_of::<u32>() + message.len();
                Ok((payload, scratch.used_bytes()))
            })
            .unwrap()
            .unwrap();
        black_box(scratch_high_water);
        Observation {
            payload_bytes,
            // The outer arena reclaims this backing at scope exit; report the
            // nested arena high-water in its own metric instead.
            high_water: before,
            nested_high_water: scratch_high_water,
            ..Observation::default()
        }
    });
}

fn benchmark_format_collect_rewrites() {
    measure_arena("format! + collect syntax rewrites", |arena| {
        let result: CompactResult<Observation> = (|| {
            let result: CompactResult<(
                CompactVec<'_, CompactString<'_>>,
                CompactVec<'_, u32>,
                CompactString<'_>,
            )> = arena!(arena, {
                let mut messages = Vec::new();
                for index in 0..1_000_u32 {
                    messages.push(format!("event-{index:04}-ready")?)?;
                }
                let range = 0_u32..1_000;
                let collected = range.collect::<Vec<_>>();
                let rendered = format!("messages={} values={}", messages.len(), collected.len())?;
                Ok::<_, CollectionError>((messages, collected, rendered))
            });
            let (messages, collected, rendered) = result?;
            let payload_bytes = messages
                .as_slice(arena)?
                .iter()
                .map(CompactString::len)
                .sum::<usize>()
                + collected.len() * core::mem::size_of::<u32>()
                + rendered.len();
            black_box(rendered.as_str(arena)?);
            let high_water = arena.used_bytes();
            Ok(Observation {
                payload_bytes,
                high_water,
                ..Observation::default()
            })
        })();
        result.unwrap()
    });
}

fn benchmark_fragmentation() {
    measure_arena("fragmentation / alternating live blocks", |arena| {
        let mut blocks = CompactVec::with_capacity_in(256, arena).unwrap();
        for _ in 0..256 {
            let mut block = arena.alloc_owned_slice::<u8>(256).unwrap();
            for value in 0..256_u16 {
                block.push(value as u8).unwrap();
            }
            blocks.push_in(Some(block), arena).unwrap();
        }
        let released_bytes_before = arena.remaining_bytes();
        let mut released_ranges = 0;
        for index in (0..256).step_by(2) {
            let owner = blocks.get_mut(index, arena).unwrap().unwrap().take();
            drop(owner);
            released_ranges += 1;
        }
        let fragmented_free_bytes = arena.remaining_bytes() - released_bytes_before;
        let mut replacement = CompactBytes::with_capacity_in(128 * 256, arena).unwrap();
        for _ in 0..128 {
            replacement
                .extend_from_slice_in(&[0xAA; 256], arena)
                .unwrap();
        }
        black_box(replacement.len());
        let high_water = arena.used_bytes();
        Observation {
            payload_bytes: 256 * 128 + replacement.len(),
            high_water,
            fragmented_ranges: released_ranges,
            fragmented_free_bytes,
            ..Observation::default()
        }
    });
}

fn benchmark_freeze_and_frozen_read() {
    StdArena::with_capacity(8 * 1024 * 1024, |arena| {
        let mut names = CompactVec::with_capacity_in(4_096, arena).unwrap();
        let mut payload_bytes = 0;
        for index in 0..4_096_u32 {
            let name = format_in!(arena, "route-{index:04}").unwrap();
            payload_bytes += name.len();
            names.push_in(name, arena).unwrap();
        }
        let catalog = FrozenCatalog {
            service: CompactString::from_str_in("edge-router", arena).unwrap(),
            base_path: CompactPathBuf::from_path(Path::new("/etc/edge-router"), arena).unwrap(),
            names,
        };
        payload_bytes += catalog.service.len() + catalog.base_path.to_path_buf().as_os_str().len();
        let source_bytes = arena.used_bytes();

        let freeze_allocations = AllocationSnapshot::now();
        let freeze_started = Instant::now();
        let (frozen, root) = freeze_in(&catalog, arena).expect("catalog freezes");
        let freeze_elapsed = freeze_started.elapsed();
        report(
            "freeze / 4,096-name catalog",
            freeze_elapsed,
            payload_bytes,
            source_bytes,
            source_bytes,
            arena.remaining_bytes(),
            0,
            0,
            AllocationSnapshot::now().delta(freeze_allocations),
            0,
        );
        println!(
            "  frozen_backing={}B source_arena={}B",
            frozen.used_bytes(),
            source_bytes
        );

        let read_allocations = AllocationSnapshot::now();
        let read_started = Instant::now();
        let view = root.get(&frozen).expect("frozen root resolves");
        let names = view
            .names()
            .as_slice(&frozen)
            .expect("frozen names resolve");
        let checksum = names
            .iter()
            .map(|name| name.as_str(&frozen).expect("frozen string resolves").len())
            .sum::<usize>();
        let service = view.service().as_str(&frozen).expect("service resolves");
        let path = view
            .base_path()
            .to_path_buf(&frozen)
            .expect("frozen path resolves");
        black_box((checksum, service, path));
        let read_elapsed = read_started.elapsed();
        report(
            "frozen read / full catalog traversal",
            read_elapsed,
            payload_bytes,
            source_bytes,
            source_bytes,
            arena.remaining_bytes(),
            0,
            0,
            AllocationSnapshot::now().delta(read_allocations),
            0,
        );
        println!("  frozen_backing={}B", frozen.used_bytes());
    })
    .expect("benchmark arena construction succeeds");
}

fn main() {
    benchmark_vec();
    benchmark_vecdeque();
    benchmark_log_ring();
    benchmark_chunk_transfer();
    benchmark_hash_collections();
    benchmark_string();
    benchmark_bytes();
    benchmark_paths();
    benchmark_serde_config();
    benchmark_scratch();
    benchmark_format_collect_rewrites();
    benchmark_fragmentation();
    benchmark_freeze_and_frozen_read();
}
