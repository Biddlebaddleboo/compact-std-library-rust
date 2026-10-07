use compact_backend_std::{CageConfig, CompactRuntime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    CompactRuntime::init(CageConfig::new(1024 * 1024))?;
    let mut values = CompactRuntime::alloc_owned_slice::<u64>(10)?;
    for value in 1..=10 {
        values.push(value)?;
    }
    let total: u64 = values.as_slice().iter().sum();
    println!("sum={total}, cage used={}B", CompactRuntime::used_bytes()?);
    Ok(())
}
