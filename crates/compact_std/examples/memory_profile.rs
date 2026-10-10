//! Non-timing process and cage memory snapshots for the V2.5 allocator gate.

use compact_std::{CageAllocation, CageConfig, CompactRuntime};
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const CAGE_BYTES: usize = 128 * 1024 * 1024;
const OWNER_COUNT: usize = 128;
const ELEMENTS_PER_OWNER: usize = 512;
const QUIET_PERIOD: Duration = Duration::from_millis(200);

fn allocate_owners() -> Result<Vec<CageAllocation<u64>>, Box<dyn Error>> {
    let mut owners = Vec::with_capacity(OWNER_COUNT);
    for _ in 0..OWNER_COUNT {
        let mut owner = CompactRuntime::alloc_owned_slice::<u64>(ELEMENTS_PER_OWNER)?;
        owner.extend_from_fn(ELEMENTS_PER_OWNER, || 0x5a5a_a5a5_d3d3_c3c3)?;
        owners.push(owner);
    }
    black_box(&owners);
    Ok(owners)
}

fn kib_value(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find(|line| line.starts_with(key))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
}

fn snapshot(label: &str) -> Result<(), Box<dyn Error>> {
    let status = fs::read_to_string("/proc/self/status").ok();
    let rollup = fs::read_to_string("/proc/self/smaps_rollup").ok();
    let field = |contents: &Option<String>, name: &str| {
        contents
            .as_deref()
            .and_then(|text| kib_value(text, name))
            .map_or_else(|| "unavailable".to_owned(), |value| value.to_string())
    };
    let stats = CompactRuntime::allocator_stats()?;
    println!(
        "process\t{label}\tVmSize_kB={}\tVmRSS_kB={}\tVmHWM_kB={}\tVmData_kB={}\tVmSwap_kB={}\tRssAnon_kB={}\tSmapsRss_kB={}\tSmapsAnonymous_kB={}\tPrivateDirty_kB={}",
        field(&status, "VmSize:"),
        field(&status, "VmRSS:"),
        field(&status, "VmHWM:"),
        field(&status, "VmData:"),
        field(&status, "VmSwap:"),
        field(&status, "RssAnon:"),
        field(&rollup, "Rss:"),
        field(&rollup, "Anonymous:"),
        field(&rollup, "Private_Dirty:"),
    );
    println!(
        "cage\t{label}\tlive_bytes={}\thigh_water_cursor={}\tfree_bytes={}\tfree_blocks={}\tlargest_free_block={}",
        stats.live_bytes,
        stats.high_water_cursor,
        stats.free_bytes,
        stats.free_blocks,
        stats.largest_free_block,
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    CompactRuntime::init(CageConfig::new(CAGE_BYTES))?;
    println!(
        "meta\tcage_capacity_bytes={CAGE_BYTES}\towner_count={OWNER_COUNT}\telements_per_owner={ELEMENTS_PER_OWNER}\tquiescence_ms={}",
        QUIET_PERIOD.as_millis()
    );
    snapshot("initialized")?;

    let owners = allocate_owners()?;
    snapshot("main_thread_live")?;
    drop(owners);
    thread::sleep(QUIET_PERIOD);
    snapshot("main_thread_dropped_quiescent")?;

    let (ready_tx, ready_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let worker = thread::spawn(move || {
        let owners = allocate_owners().expect("worker cage allocations succeed");
        ready_tx
            .send(())
            .expect("main thread receives ready signal");
        release_rx
            .recv()
            .expect("main thread releases the worker owners");
        drop(owners);
    });
    ready_rx.recv()?;
    thread::sleep(QUIET_PERIOD);
    snapshot("worker_thread_live")?;
    release_tx.send(())?;
    worker
        .join()
        .map_err(|_| std::io::Error::other("memory probe worker panicked"))?;
    thread::sleep(QUIET_PERIOD);
    snapshot("after_worker_exit_quiescent")?;
    Ok(())
}
