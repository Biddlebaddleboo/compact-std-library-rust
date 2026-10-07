use compact_std::{read_bits, write_bits, CageConfig, CompactBytes, CompactRuntime, CompactVec};
use std::hint::black_box;
use std::mem::size_of;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    CompactRuntime::init(CageConfig::new(64 * 1024 * 1024))?;
    println!(
        "owner sizes: Vec<u32>={}B Box<u64>={}B CompactBytes={}B",
        size_of::<CompactVec<u32>>(),
        size_of::<compact_std::CompactBox<u64>>(),
        size_of::<CompactBytes>()
    );
    let started = Instant::now();
    let mut values = CompactVec::new();
    for value in 0..100_000_u32 {
        values.push(value)?;
    }
    let checksum: u64 = values.iter().map(|value| u64::from(*value)).sum();
    println!(
        "vector growth + traversal: {:?}; checksum={checksum}; used={}B",
        started.elapsed(),
        CompactRuntime::used_bytes()?
    );

    let payload = [0x5a; 64];
    let started = Instant::now();
    for _ in 0..10_000 {
        black_box(CompactBytes::from_slice(black_box(&payload))?);
    }
    println!("64-byte values: {:?}", started.elapsed());

    let mut bytes = [0_u8; 8];
    write_bits(&mut bytes, 5, 17, 0x1ffff)?;
    assert_eq!(read_bits(&bytes, 5, 17)?, 0x1ffff);
    Ok(())
}
