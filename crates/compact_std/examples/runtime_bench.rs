use std::hint::black_box;
use std::mem::size_of;
use std::time::Instant;

use compact_std::prelude::*;
use compact_std::{read_bits, write_bits, Result as CompactResult};

#[compact]
struct BenchFields {
    flag: bool,
    #[max = 7]
    small: u8,
    aligned16: u16,
    value: u32,
}

fn reference_read(bytes: &[u8], offset: usize, width: u8) -> u64 {
    validate_reference_range(bytes.len(), offset, width);
    let mut value = 0_u64;
    for bit in 0..width as usize {
        let position = offset + bit;
        if bytes[position / 8] & (1 << (position % 8)) != 0 {
            value |= 1 << bit;
        }
    }
    value
}

fn reference_write(bytes: &mut [u8], offset: usize, width: u8, value: u64) {
    validate_reference_range(bytes.len(), offset, width);
    let value_mask = if width == 64 {
        u64::MAX
    } else if width == 0 {
        0
    } else {
        (1_u64 << width) - 1
    };
    assert_eq!(value & !value_mask, 0);
    for bit in 0..width as usize {
        let position = offset + bit;
        let mask = 1 << (position % 8);
        if value & (1 << bit) == 0 {
            bytes[position / 8] &= !mask;
        } else {
            bytes[position / 8] |= mask;
        }
    }
}

fn validate_reference_range(bytes_len: usize, offset: usize, width: u8) {
    assert!(width <= 64);
    let total_bits = bytes_len.checked_mul(8).expect("benchmark length fits");
    let end = offset
        .checked_add(width as usize)
        .expect("benchmark bit range fits");
    assert!(end <= total_bits);
}

fn benchmark_field(name: &str, offset: usize, width: u8, iterations: usize) {
    let seed = [0xA5_u8, 0x39, 0xD2, 0x68, 0xF0, 0x1B, 0x87, 0x42, 0xE1];
    let started = Instant::now();
    let mut read_sink = 0_u64;
    for _ in 0..iterations {
        read_sink ^= black_box(read_bits(black_box(&seed), offset, width).unwrap());
    }
    let fast_read = started.elapsed();

    let started = Instant::now();
    for _ in 0..iterations {
        read_sink ^= black_box(reference_read(black_box(&seed), offset, width));
    }
    let reference_read_time = started.elapsed();

    let value_mask = if width == 64 {
        u64::MAX
    } else {
        (1_u64 << width) - 1
    };
    let value = 0xA57B_91C3_5D2E_684F & value_mask;
    let started = Instant::now();
    let mut fast_bytes = seed;
    for _ in 0..iterations {
        write_bits(black_box(&mut fast_bytes), offset, width, black_box(value)).unwrap();
    }
    let fast_write = started.elapsed();

    let started = Instant::now();
    let mut reference_bytes = seed;
    for _ in 0..iterations {
        reference_write(
            black_box(&mut reference_bytes),
            offset,
            width,
            black_box(value),
        );
    }
    let reference_write_time = started.elapsed();
    assert_eq!(fast_bytes, reference_bytes);
    black_box((read_sink, fast_bytes, reference_bytes));
    println!(
        "packed {name:>12}: read fast={fast_read:?} loop={reference_read_time:?}; write fast={fast_write:?} loop={reference_write_time:?}"
    );
}

fn main() -> std::result::Result<(), std::boxed::Box<dyn std::error::Error>> {
    const ITERATIONS: usize = 2_000_000;
    for (name, offset, width) in [
        ("boolean", 3, 1),
        ("3-bit", 5, 3),
        ("cross-byte", 7, 9),
        ("aligned-16", 16, 16),
        ("aligned-32", 32, 32),
    ] {
        benchmark_field(name, offset, width, ITERATIONS);
    }

    StdArena::with_capacity(4096, |arena| -> CompactResult<()> {
        let logical = BenchFields {
            flag: true,
            small: 5,
            aligned16: 0x1234,
            value: 0xDEAD_BEEF,
        };
        let compact = logical.compact_in(arena)?;
        let started = Instant::now();
        for index in 0..ITERATIONS {
            black_box(compact.flag(arena)?);
            black_box(compact.small(arena)?);
            black_box(compact.aligned16(arena)?);
            black_box(compact.value(arena)?);
            compact.set_flag(index & 1 == 0, arena)?;
            compact.set_small((index & 7) as u8, arena)?;
            compact.set_aligned16(index as u16, arena)?;
            compact.set_value(index as u32, arena)?;
        }
        println!("generated getter/setter loop: {:?}", started.elapsed());
        Ok(())
    })??;

    StdArena::with_capacity(1024 * 1024, |arena| -> CompactResult<()> {
        let before = arena.used_bytes();
        let started = Instant::now();
        let mut values = Vec::new_in(arena);
        for index in 0..65_536_u32 {
            values.push_in(index, arena)?;
        }
        let elapsed = started.elapsed();
        let after = arena.used_bytes();
        let mut old_capacity = 4_usize;
        let mut theoretical_old_payload = 0_usize;
        while old_capacity <= 65_536 {
            theoretical_old_payload += old_capacity * size_of::<u32>();
            old_capacity *= 2;
        }
        println!(
            "vector growth: {elapsed:?}; len={}; cap={}; used_delta={}; historical_payload_if_every_growth_leaked={theoretical_old_payload}",
            values.len(),
            values.capacity(),
            after - before,
        );
        let started = Instant::now();
        let traversal_sum = values
            .iter(arena)?
            .fold(0_u64, |sum, value| sum.wrapping_add(u64::from(*value)));
        println!(
            "vector traversal: {:?}; checksum={traversal_sum}",
            started.elapsed()
        );

        let rounds = 10_000;
        let started = Instant::now();
        for _ in 0..rounds {
            black_box(arena.alloc_uninit::<u64>()?);
        }
        println!("tail allocation ({rounds} rounds): {:?}", started.elapsed());

        let started = Instant::now();
        for _ in 0..rounds {
            let allocation = arena.alloc_owned_slice::<u64>(1)?;
            drop(allocation);
        }
        println!(
            "allocate/release/reuse ({rounds} rounds): {:?}",
            started.elapsed()
        );
        Ok(())
    })??;
    Ok(())
}
