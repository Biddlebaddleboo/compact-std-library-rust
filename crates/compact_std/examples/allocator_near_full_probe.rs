use compact_std::{CageConfig, CompactRuntime, CoreError};
use std::error::Error;

const CAGE_BYTES: usize = 16 * 1024 * 1024;
const NEAR_FULL_BYTES: usize = 15 * 1024 * 1024;
const FAILURE_BYTES: usize = 2 * 1024 * 1024;
const RECOVERY_BYTES: usize = 1024 * 1024;
const HEADER_BYTES: usize = 16;
const BLOCK_ALIGNMENT: usize = 8;

fn expected_block_bytes(payload_bytes: usize) -> usize {
    (payload_bytes + HEADER_BYTES + BLOCK_ALIGNMENT - 1) & !(BLOCK_ALIGNMENT - 1)
}

fn check_live(label: &str, expected: usize) -> Result<(), Box<dyn Error>> {
    let used = CompactRuntime::used_bytes()?;
    let stats_live = CompactRuntime::allocator_stats()?.live_bytes as usize;
    if used != expected || stats_live != expected {
        return Err(format!(
            "{label}: expected used/live bytes {expected}, got used={used}, stats={stats_live}"
        )
        .into());
    }
    println!("CHECK\t{label}\texpected={expected}\tused={used}\tstats_live={stats_live}");
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    CompactRuntime::init(CageConfig::new(CAGE_BYTES))?;
    check_live("empty", 0)?;

    let near_full = CompactRuntime::alloc_owned_slice::<u8>(NEAR_FULL_BYTES)?;
    let near_full_live = expected_block_bytes(NEAR_FULL_BYTES);
    check_live("near_full", near_full_live)?;
    let remaining = CompactRuntime::remaining_bytes()?;
    let expected_remaining = CAGE_BYTES - near_full_live;
    if remaining != expected_remaining || near_full_live * 100 < CAGE_BYTES * 90 {
        return Err(format!(
            "near-full accounting: expected remaining={expected_remaining}, got {remaining}"
        )
        .into());
    }
    println!(
        "NEAR_FULL\tcage_bytes={CAGE_BYTES}\tpayload_bytes={NEAR_FULL_BYTES}\tlive_bytes={near_full_live}\tremaining_bytes={remaining}"
    );

    match CompactRuntime::alloc_owned_slice::<u8>(FAILURE_BYTES) {
        Err(CoreError::AllocationExhausted) => {
            println!("EXPECTED_FAILURE\tpayload_bytes={FAILURE_BYTES}\terror=AllocationExhausted");
        }
        Err(error) => return Err(format!("unexpected allocation error: {error}").into()),
        Ok(allocation) => {
            drop(allocation);
            return Err("2 MiB allocation unexpectedly succeeded".into());
        }
    }
    check_live("after_expected_failure", near_full_live)?;

    drop(near_full);
    check_live("after_near_full_drop", 0)?;
    CompactRuntime::validate_allocator_state()?;

    let recovered = CompactRuntime::alloc_owned_slice::<u8>(RECOVERY_BYTES)?;
    let recovery_live = expected_block_bytes(RECOVERY_BYTES);
    check_live("recovered_allocation", recovery_live)?;
    println!(
        "RECOVERY\tpayload_bytes={RECOVERY_BYTES}\tlive_bytes={recovery_live}"
    );

    drop(recovered);
    check_live("final", 0)?;
    CompactRuntime::validate_allocator_state()?;
    println!("RESULT\tnear_full_failure_recovery=passed");
    Ok(())
}
