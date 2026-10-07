use compact_std::{
    CompactBytes, CompactHashMap, CompactHashSet, CompactPathBuf, CompactRuntime, CompactString,
    CompactVec, CompactVecDeque, FrozenBuilder, FrozenString, FrozenVec, ScratchRegion,
};
use std::hint::black_box;
use std::time::Instant;

const CAGE_BYTES: usize = 128 * 1024 * 1024;

fn report(name: &str, started: Instant, before: usize) {
    println!(
        "{name}: elapsed={:?}; live_cage_delta={}B",
        started.elapsed(),
        CompactRuntime::used_bytes().unwrap() - before
    );
}

#[derive(compact_std::CompactDeserialize)]
struct ServiceConfig {
    service: CompactString,
    paths: CompactVec<CompactString>,
    labels: CompactVec<CompactString>,
}

#[derive(Clone, Copy, compact_std::FrozenValue)]
struct CatalogRoot {
    title: FrozenString,
    ids: FrozenVec<u32>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    CompactRuntime::init(compact_std::CageConfig::new(CAGE_BYTES))?;

    let before = CompactRuntime::used_bytes()?;
    let started = Instant::now();
    let mut values = CompactVec::new();
    for value in 0..65_536_u32 {
        values.push(value)?;
    }
    let checksum: u64 = values.iter().map(|value| u64::from(*value)).sum();
    black_box(checksum);
    report("Vec growth and traversal", started, before);
    drop(values);

    let before = CompactRuntime::used_bytes()?;
    let started = Instant::now();
    let mut queue = CompactVecDeque::with_capacity(1_024)?;
    for value in 0..1_000_u64 {
        queue.push_back(value)?;
    }
    for expected in 0..250 {
        assert_eq!(queue.pop_front(), Some(expected));
        queue.push_back(expected + 1_000)?;
    }
    black_box(queue.iter().copied().sum::<u64>());
    report("VecDeque FIFO", started, before);
    drop(queue);

    let before = CompactRuntime::used_bytes()?;
    let started = Instant::now();
    let mut map = CompactHashMap::with_capacity(4_096)?;
    let mut set = CompactHashSet::with_capacity(4_096)?;
    for value in 0..4_096_u32 {
        map.insert(value, value.wrapping_mul(17))?;
        set.insert(value)?;
    }
    black_box((map.get(&777), set.contains(&777)));
    report("HashMap and HashSet", started, before);
    drop((map, set));

    let before = CompactRuntime::used_bytes()?;
    let started = Instant::now();
    let mut chunks = CompactVec::with_capacity(1_024)?;
    for index in 0..1_024_u16 {
        chunks.push(CompactBytes::from_slice(&[index as u8; 256])?)?;
    }
    black_box(chunks.iter().map(CompactBytes::len).sum::<usize>());
    report("chunk assembly", started, before);
    drop(chunks);

    let before = CompactRuntime::used_bytes()?;
    let started = Instant::now();
    let mut text = CompactString::new();
    for _ in 0..4_096 {
        text.push_str("service-label/edge-router;")?;
    }
    black_box(text.as_str());
    report("String append", started, before);
    drop(text);

    let before = CompactRuntime::used_bytes()?;
    let started = Instant::now();
    let bytes = CompactBytes::from_slice(&[0x5a; 64 * 1024])?;
    black_box(bytes.as_slice());
    report("64 KiB Bytes", started, before);
    drop(bytes);

    let before = CompactRuntime::used_bytes()?;
    let started = Instant::now();
    let mut paths = CompactVec::with_capacity(1_000)?;
    for index in 0..1_000 {
        paths.push(CompactPathBuf::from(format!(
            "/srv/worker/{index}/config.toml"
        ))?)?;
    }
    black_box(paths[999].to_path_buf());
    report("PathBuf workload", started, before);
    drop(paths);

    let source = "service = 'edge-router'\npaths = ['/etc/edge/router.toml', '/var/log/edge/router.log']\nlabels = ['api', 'ingest', 'metrics']\n";
    let before = CompactRuntime::used_bytes()?;
    let started = Instant::now();
    let config = compact_std::toml::from_str::<ServiceConfig>(source)?;
    black_box((
        config.service.as_str(),
        config.paths.len(),
        config.labels.len(),
    ));
    report("direct TOML config", started, before);
    drop(config);

    let before = CompactRuntime::used_bytes()?;
    let started = Instant::now();
    let mut scratch = ScratchRegion::new(64 * 1024)?;
    for _ in 0..1_000 {
        black_box(scratch.alloc_bytes(32)?);
    }
    report("scratch allocations", started, before);
    drop(scratch);

    let before = CompactRuntime::used_bytes()?;
    let started = Instant::now();
    let mut builder = FrozenBuilder::new()?;
    let title = builder.store_str("catalog")?;
    let ids: std::vec::Vec<u32> = (0..4_096).collect();
    let ids = builder.store_slice(&ids)?;
    let graph = builder.finish(CatalogRoot { title, ids })?;
    black_box((
        graph.str(&graph.root().title)?,
        graph.slice(&graph.root().ids)?.len(),
    ));
    report("frozen catalog", started, before);
    drop(graph);

    println!("final live cage bytes={}", CompactRuntime::used_bytes()?);
    Ok(())
}
