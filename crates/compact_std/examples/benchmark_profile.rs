//! Low-overhead profiling driver for the `benchmark_compare` scenarios.
//!
//! Measured suites use [`crate::benchmark_compare`], which installs a counting
//! global allocator and captures per-phase allocator snapshots. Both cost real
//! time, so CPU profiles taken from them misattribute the accounting work to
//! the workload. This example runs the exact same scenario code in
//! [`measure::MeasurementMode::Profile`] mode:
//!
//! * no counting global allocator is installed (the process uses `System`),
//! * no per-phase allocator snapshot is taken, so the cage lock is untouched
//!   while the workload runs,
//! * compact allocator state is checked after each window, outside the
//!   recorded workload time, and
//! * a whole-process profiler can still observe that single post-window check;
//!   it is excluded from the elapsed_ns value, and
//! * each `--window` invocation repeats one scenario/variant for a wall-clock
//!   target so a sampler gets enough samples, including the named B10 worker
//!   threads, and
//! * every window reports the same logical checksum the measured harness
//!   reports, which is how mode equivalence is proven.
//!
//! Usage (driver): profile every scenario for three seconds per variant.
//! Variant order can be fixed, alternate by scenario, or alternate-reversed.
//! ```text
//! cargo run --release -p compact_std --example benchmark_profile \
//!     --features json,toml -- --seconds 3 --output /tmp/v24-profile.tsv
//! ```
//! Usage (sampler target): attach a profiler to a single long window.
//! ```text
//! perf record --call-graph dwarf -- target/release/examples/benchmark_profile-<hash> \
//!     --window B10 compact 5
//! ```
//! `--mode profile` on `benchmark_compare` is equivalent but keeps the counting
//! allocator installed as a pass-through; prefer this binary when purity of the
//! allocation path matters.
#![allow(dead_code)]

#[path = "benchmark_compare/datasets.rs"]
mod datasets;
#[path = "benchmark_compare/measure.rs"]
mod measure;
#[path = "benchmark_compare/models.rs"]
mod models;
#[path = "benchmark_compare/scenarios.rs"]
mod scenarios;

use measure::{BenchResult, MeasurementMode};
use std::process::Command;
use std::time::{Duration, Instant};

const CAGE_BYTES: usize = 128 * 1024 * 1024;

fn main() -> BenchResult<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "--window")
    {
        return run_window(&arguments[1..]);
    }
    run_driver(&arguments)
}

/// Runs the scenarios and verifies that native and compact windows agree.
fn run_driver(arguments: &[String]) -> BenchResult<()> {
    let mut seconds = 5.0_f64;
    let mut runs_override = None;
    let mut output_path = None;
    let mut scenario_filters = Vec::new();
    let mut variant_filters = Vec::new();
    let mut variant_order = VariantOrder::Fixed;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--seconds" => {
                index += 1;
                seconds = arguments
                    .get(index)
                    .ok_or("--seconds requires a value")?
                    .parse::<f64>()?;
                if !seconds.is_finite() || seconds < 0.0 {
                    return Err("--seconds must be a finite, non-negative number".into());
                }
            }
            "--runs" => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or("--runs requires a value")?
                    .parse::<usize>()?;
                if value == 0 {
                    return Err("--runs must be positive".into());
                }
                runs_override = Some(value);
            }
            "--output" => {
                index += 1;
                output_path = Some(
                    arguments
                        .get(index)
                        .ok_or("--output requires a path")?
                        .clone(),
                );
            }
            "--scenario" => {
                index += 1;
                let scenario = arguments
                    .get(index)
                    .ok_or("--scenario requires an ID")?
                    .clone();
                if !scenarios::SCENARIOS
                    .iter()
                    .any(|(known, _)| *known == scenario)
                {
                    return Err(format!("unknown scenario {scenario:?}").into());
                }
                scenario_filters.push(scenario);
            }
            "--variant" => {
                index += 1;
                let variant = arguments
                    .get(index)
                    .ok_or("--variant requires native or compact")?
                    .clone();
                if variant != "native" && variant != "compact" {
                    return Err(format!("unknown variant {variant:?}").into());
                }
                variant_filters.push(variant);
            }
            "--order" => {
                index += 1;
                variant_order = VariantOrder::from_name(
                    arguments
                        .get(index)
                        .ok_or("--order requires fixed, alternate, or alternate-reversed")?,
                )
                .ok_or("--order requires fixed, alternate, or alternate-reversed")?;
            }
            other => return Err(format!("unknown argument {other:?}").into()),
        }
        index += 1;
    }

    let executable = std::env::current_exe()?;
    let selected_scenarios = scenarios::SCENARIOS
        .iter()
        .filter(|(scenario, _)| {
            scenario_filters.is_empty() || scenario_filters.iter().any(|filter| filter == scenario)
        })
        .copied()
        .collect::<Vec<_>>();
    let variants = ["native", "compact"]
        .into_iter()
        .filter(|variant| {
            variant_filters.is_empty() || variant_filters.iter().any(|filter| filter == variant)
        })
        .collect::<Vec<_>>();

    let mut metadata = Vec::new();
    println!(
        "Profiling windows: {} scenario(s) x {} variant(s), target {seconds:.1}s each, order {}, no allocator accounting.",
        selected_scenarios.len(),
        variants.len(),
        variant_order.name()
    );
    metadata.push(format!(
        "META\tsuite\tvariant_order\t{}",
        variant_order.name()
    ));
    for (scenario_index, &(scenario, description)) in selected_scenarios.iter().enumerate() {
        eprintln!("profile {scenario}: {description}");
        let runs = runs_override.unwrap_or_else(|| scenarios::repetitions_for(scenario));
        let mut pair_checksums = Vec::new();
        for &variant in &variant_order.variants(scenario_index, &variants) {
            let (window_metadata, checksum) =
                run_window_process(&executable, scenario, variant, runs, seconds)?;
            metadata.extend(window_metadata);
            pair_checksums.push((variant, checksum));
            println!(
                "PROFILE\t{scenario}\t{variant}\tchecksum={checksum:#018x}\truns_per_window={runs}\ttarget_seconds={seconds:.1}"
            );
        }
        let (first_variant, first_checksum) = pair_checksums[0];
        for (variant, checksum) in &pair_checksums[1..] {
            if *checksum != first_checksum {
                return Err(format!(
                    "{scenario} native/compact logical results differ in profile mode: {first_variant}={first_checksum:#x}, {variant}={checksum:#x}"
                )
                .into());
            }
        }
    }
    println!(
        "Profile windows finished with matching native/compact checksums; compare these checksums against measure-mode output before drawing conclusions."
    );
    if let Some(path) = output_path {
        let mut text = String::new();
        for line in &metadata {
            text.push_str(line);
            text.push('\n');
        }
        std::fs::write(&path, text)?;
        eprintln!("wrote profile metadata to {path}");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VariantOrder {
    Fixed,
    Alternate,
    AlternateReversed,
}

impl VariantOrder {
    fn name(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Alternate => "alternate",
            Self::AlternateReversed => "alternate-reversed",
        }
    }

    fn from_name(value: &str) -> Option<Self> {
        match value {
            "fixed" => Some(Self::Fixed),
            "alternate" => Some(Self::Alternate),
            "alternate-reversed" => Some(Self::AlternateReversed),
            _ => None,
        }
    }

    fn variants<'a>(self, scenario_index: usize, variants: &[&'a str]) -> Vec<&'a str> {
        let reverse = match self {
            Self::Fixed => false,
            Self::Alternate => scenario_index % 2 == 1,
            Self::AlternateReversed => scenario_index % 2 == 0,
        };
        let mut ordered = variants.to_vec();
        if reverse && ordered.len() == 2 {
            ordered.reverse();
        }
        ordered
    }
}

fn run_window_process(
    executable: &std::path::Path,
    scenario: &str,
    variant: &str,
    runs: usize,
    seconds: f64,
) -> BenchResult<(Vec<String>, u64)> {
    let output = Command::new(executable)
        .arg("--window")
        .arg(scenario)
        .arg(variant)
        .arg(runs.to_string())
        .arg("--seconds")
        .arg(format!("{seconds:.3}"))
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "profile window {scenario}/{variant} failed with {}:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let stdout = String::from_utf8(output.stdout)?;
    let mut metadata = Vec::new();
    let mut checksum = None;
    for line in stdout.lines() {
        if line.starts_with("META\tchecksum\t") {
            let fields: Vec<&str> = line.split('\t').collect();
            let reported = fields
                .get(4)
                .ok_or_else(|| format!("invalid checksum row: {line}"))?
                .parse::<u64>()?;
            if let Some(previous) = checksum {
                if previous != reported {
                    return Err(format!(
                        "profile window {scenario}/{variant} reported inconsistent checksums {previous:#x} and {reported:#x}"
                    )
                    .into());
                }
            }
            checksum = Some(reported);
            metadata.push(line.to_owned());
        } else if line.starts_with("META\t") {
            metadata.push(line.to_owned());
        }
    }
    let checksum = checksum
        .ok_or_else(|| format!("profile window {scenario}/{variant} did not report a checksum"))?;
    Ok((metadata, checksum))
}

/// Runs one scenario/variant for a wall-clock target so samplers get enough
/// samples and every named worker thread is exercised.
fn run_window(arguments: &[String]) -> BenchResult<()> {
    let scenario = arguments.first().ok_or("--window requires a scenario")?;
    let variant = arguments.get(1).ok_or("--window requires a variant")?;
    let runs = arguments
        .get(2)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or_else(|| scenarios::repetitions_for(scenario));
    if runs == 0 {
        return Err("repetition count must be positive".into());
    }
    let mut seconds = 5.0_f64;
    let mut index = 3;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--seconds" => {
                index += 1;
                seconds = arguments
                    .get(index)
                    .ok_or("--seconds requires a value")?
                    .parse::<f64>()?;
                if !seconds.is_finite() || seconds < 0.0 {
                    return Err("--seconds must be a finite, non-negative number".into());
                }
            }
            other => return Err(format!("unknown window argument {other:?}").into()),
        }
        index += 1;
    }

    measure::set_measurement_mode(MeasurementMode::Profile);
    measure::set_phase_output(false);
    if variant == "compact" {
        compact_std::CompactRuntime::init(compact_std::CageConfig::new(CAGE_BYTES))?;
    } else if variant != "native" {
        return Err(format!("unknown benchmark variant {variant:?}").into());
    }

    let target = Duration::from_secs_f64(seconds);
    let mut workload_elapsed = Duration::ZERO;
    let mut windows = 0_u64;
    loop {
        let window_started = Instant::now();
        scenarios::run(scenario, variant, runs)?;
        workload_elapsed += window_started.elapsed();
        windows += 1;
        if variant == "compact" {
            measure::validate_compact_state(scenario, variant)?;
        }
        if workload_elapsed >= target {
            break;
        }
    }
    println!(
        "META\tprofile_window\t{scenario}\t{variant}\twindows\t{windows}\truns_per_window\t{runs}\ttarget_seconds\t{seconds:.3}\telapsed_ns\t{}",
        workload_elapsed.as_nanos()
    );
    Ok(())
}
