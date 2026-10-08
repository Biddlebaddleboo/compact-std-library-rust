//! Authoritative native-vs-compact benchmark harness.
//!
//! Each scenario runs in its own child process per variant, accumulates
//! per-phase samples, cross-checks the logical checksum between variants and
//! reports timings plus allocator statistics.
//!
//! # Measurement modes
//!
//! `--mode measure` (default) keeps full allocator accounting: per-allocation
//! counting on the native side and per-phase allocator snapshots on the compact
//! side. `--mode profile` drops that accounting so a CPU profiler observes the
//! workload; its timings and memory columns must not be compared with
//! measure-mode output. Every child states its mode in
//! `META measurement_mode`, and `--self-check` asserts that both modes produce
//! the same logical checksum for every scenario.
//!
//! # Relationship to `collection_profile`
//!
//! `crates/compact_std/examples/collection_profile.rs` is a diagnostic harness,
//! not a timing authority. It shares the A4/A5 workload sizes (4,096-entry deque
//! population, 80,000 deque operations, 16,000 hash entries, 4,000-key churn)
//! and B6 shape (2,048 levels, 8 rounds x 192 updates), but differs in ways that
//! make its absolute numbers non-comparable: it black-boxes every individual
//! operation instead of per-phase results, it pre-reserves deque capacity so
//! growth is excluded, it runs extra probe-count models and FNV/collision
//! variants, it installs no counting global allocator, and it has no B8 or B10
//! coverage. Treat its numbers as attribution evidence, never as
//! compact/native ratios; only this harness produces the recorded ratios.
//!
//! # Variant order and repeatability
//!
//! `--order alternate` flips which variant runs first on every other scenario so
//! host drift cannot systematically favour one side of a pair; the per-scenario
//! order is recorded in `META scenario_variant_order`. Host, build, feature,
//! load and toolchain facts are recorded in the suite metadata of every run.

mod datasets;
mod measure;
mod models;
mod scenarios;

use measure::{BenchResult, CountingAllocator, MeasurementMode, MEASUREMENT_MODE_VERSION};
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::Command;

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

const CAGE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone)]
struct PhaseRow {
    scenario: String,
    variant: String,
    phase: String,
    fields: Vec<String>,
}

fn main() -> BenchResult<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "--child")
    {
        return run_child(&arguments[1..]);
    }

    let mut runs_override = None;
    let mut output_path = None;
    let mut scenario_filters = Vec::new();
    let mut measurement_mode = MeasurementMode::Measure;
    let mut order = VariantOrder::Fixed;
    let mut self_check = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--runs" => {
                index += 1;
                runs_override = Some(parse_positive(&arguments, index, "--runs")?);
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
            "--mode" => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or("--mode requires measure or profile")?;
                measurement_mode = MeasurementMode::from_name(value).ok_or_else(|| {
                    format!("unknown measurement mode {value:?}; expected measure or profile")
                })?;
            }
            "--order" => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or("--order requires fixed or alternate")?;
                order = VariantOrder::from_name(value).ok_or_else(|| {
                    format!("unknown variant order {value:?}; expected fixed or alternate")
                })?;
            }
            "--self-check" => self_check = true,
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
            other => return Err(format!("unknown argument {other:?}").into()),
        }
        index += 1;
    }

    let executable = std::env::current_exe()?;
    measure::set_measurement_mode(measurement_mode);
    let selected_scenarios = scenarios::SCENARIOS
        .iter()
        .filter(|(scenario, _)| {
            scenario_filters.is_empty() || scenario_filters.iter().any(|filter| filter == scenario)
        })
        .copied()
        .collect::<Vec<_>>();

    if self_check {
        return run_self_check(&executable, &selected_scenarios);
    }

    let mut rows = Vec::new();
    let mut metadata = host_metadata(measurement_mode, order);
    for (scenario_index, &(scenario, description)) in selected_scenarios.iter().enumerate() {
        eprintln!("benchmark {scenario}: {description}");
        let repetitions = runs_override.unwrap_or_else(|| scenarios::repetitions_for(scenario));
        let variants = order.variants(scenario_index);
        let mut pair_checksums = Vec::with_capacity(variants.len());
        for variant in variants {
            let (child_rows, child_metadata, checksum) = run_child_process(
                &executable,
                scenario,
                variant,
                repetitions,
                measurement_mode,
            )?;
            rows.extend(child_rows);
            metadata.extend(child_metadata);
            pair_checksums.push((variant, checksum));
        }
        let (first_variant, first_checksum) = pair_checksums[0];
        for (variant, checksum) in &pair_checksums[1..] {
            if *checksum != first_checksum {
                return Err(format!(
                    "{scenario} native/compact logical results differ: {first_variant}={first_checksum:#x}, {variant}={checksum:#x}"
                )
                .into());
            }
        }
        metadata.push(format!(
            "META\tscenario_variant_order\t{scenario}\t{}\t{}",
            pair_checksums[0].0, pair_checksums[1].0
        ));
    }

    print_report(&rows);
    println!(
        "\nLogical result checksums matched for all {} scenarios.",
        selected_scenarios.len()
    );
    println!(
        "Measurement mode: {} (contract v{MEASUREMENT_MODE_VERSION}); variant order: {}.",
        measurement_mode.name(),
        order.name()
    );
    if measurement_mode == MeasurementMode::Profile {
        println!(
            "Allocator accounting is disabled in profile mode: do not compare these timings or memory columns with measure-mode output."
        );
    }
    metadata.push(format!(
        "META\tsuite_summary\tscenarios\t{}\tmeasurement_mode\t{}\tvariant_order\t{}",
        selected_scenarios.len(),
        measurement_mode.name(),
        order.name()
    ));
    if !metadata.is_empty() {
        println!("\nSupplemental process and allocator diagnostics:");
        for line in &metadata {
            println!("{}", line.replace('\t', "  "));
        }
    }
    if let Some(path) = output_path {
        write_tsv(&path, &rows, &metadata)?;
        eprintln!("wrote machine-readable results to {path}");
    }
    Ok(())
}

fn run_child(arguments: &[String]) -> BenchResult<()> {
    let scenario = arguments.first().ok_or("--child requires a scenario")?;
    let variant = arguments.get(1).ok_or("--child requires a variant")?;
    let repetitions = arguments
        .get(2)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or_else(|| scenarios::repetitions_for(scenario));
    if repetitions == 0 {
        return Err("repetition count must be positive".into());
    }
    let mut mode = MeasurementMode::Measure;
    let mut index = 3;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--mode" => {
                index += 1;
                let value = arguments.get(index).ok_or("--mode requires a value")?;
                mode = MeasurementMode::from_name(value).ok_or_else(|| {
                    format!("unknown measurement mode {value:?}; expected measure or profile")
                })?;
            }
            other => return Err(format!("unknown child argument {other:?}").into()),
        }
        index += 1;
    }
    measure::set_measurement_mode(mode);
    if variant == "compact" {
        compact_std::CompactRuntime::init(compact_std::CageConfig::new(CAGE_BYTES))?;
    } else if variant != "native" {
        return Err(format!("unknown benchmark variant {variant:?}").into());
    }
    scenarios::run(scenario, variant, repetitions)?;
    #[cfg(feature = "allocator-telemetry")]
    if variant == "compact" {
        let stats = compact_std::CompactRuntime::allocator_stats()?;
        let allocator_policy = if cfg!(feature = "benchmark-allocator-a")
            && !cfg!(feature = "benchmark-allocator-b")
            && !cfg!(feature = "benchmark-allocator-c")
        {
            "A"
        } else if cfg!(feature = "benchmark-allocator-c")
            || (cfg!(feature = "benchmark-allocator-a") && cfg!(feature = "benchmark-allocator-b"))
        {
            "C"
        } else {
            "B"
        };
        println!("META\tallocator_policy\t{scenario}\t{variant}\t{allocator_policy}");
        println!(
            "META\tallocator_summary\t{scenario}\t{variant}\t{locks}\t{visits}\t{batches}\t{extents}\t{max_batch}\t{class_hits}\t{class_misses}\t{pending_hits}\t{pending_misses}\t{pending_no_collector}\t{pending_no_exact}\t{pending_alignment}\t{class_empty}\t{class_alignment}\t{no_class}\t{general_fallbacks}\t{cursor_fallbacks}\t{released_exact}\t{cached_exact}\t{coalesced_exact}",
            locks = stats.lock_acquisitions,
            visits = stats.free_list_nodes_visited,
            batches = stats.release_batches,
            extents = stats.released_extents,
            max_batch = stats.max_release_batch,
            class_hits = stats.size_class_hits,
            class_misses = stats.size_class_misses,
            pending_hits = stats.pending_reuse_hits,
            pending_misses = stats.pending_reuse_misses,
            pending_no_collector = stats.pending_reuse_no_active_collector,
            pending_no_exact = stats.pending_reuse_no_exact_block,
            pending_alignment = stats.pending_reuse_alignment_incompatible,
            class_empty = stats.global_class_empty.iter().sum::<u64>(),
            class_alignment = stats.global_class_alignment_incompatible.iter().sum::<u64>(),
            no_class = stats.requested_size_no_class,
            general_fallbacks = stats.general_list_fallbacks,
            cursor_fallbacks = stats.cursor_fallbacks,
            released_exact = stats.released_exact_size_extents,
            cached_exact = stats.exact_size_extents_cached,
            coalesced_exact = stats.exact_size_extents_coalesced_before_cache,
        );
        let pending_scan_depths = stats
            .pending_reuse_scan_depth_histogram
            .iter()
            .enumerate()
            .filter(|(_, count)| **count != 0)
            .map(|(depth, count)| format!("{depth}:{count}"))
            .collect::<Vec<_>>()
            .join(",");
        let pending_candidate_sizes = stats
            .pending_reuse_candidate_size_histogram
            .iter()
            .enumerate()
            .filter(|(_, count)| **count != 0)
            .map(|(bucket, count)| {
                let size = if bucket + 1 == stats.pending_reuse_candidate_size_histogram.len() {
                    "1024+".to_owned()
                } else {
                    (bucket * 8).to_string()
                };
                format!("{size}:{count}")
            })
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "META\tallocator_phase_profile\t{scenario}\t{variant}\t{pending_lookup_ns}\t{layout_ns}\t{lock_wait_ns}\t{free_list_search_ns}\t{bump_allocation_ns}\t{header_initialization_ns}\t{scan_candidates}\t{scan_depths}\t{candidate_sizes}",
            pending_lookup_ns = stats.pending_lookup_phase_ns,
            layout_ns = stats.layout_phase_ns,
            lock_wait_ns = stats.lock_wait_phase_ns,
            free_list_search_ns = stats.free_list_search_phase_ns,
            bump_allocation_ns = stats.bump_allocation_phase_ns,
            header_initialization_ns = stats.header_initialization_phase_ns,
            scan_candidates = stats.pending_reuse_scan_candidates,
            scan_depths = pending_scan_depths,
            candidate_sizes = pending_candidate_sizes,
        );
        for (class_index, (class_size, (blocks, bytes))) in [32_u32, 40, 112, 528]
            .into_iter()
            .zip(
                stats
                    .size_class_free_blocks
                    .into_iter()
                    .zip(stats.size_class_free_bytes),
            )
            .enumerate()
        {
            println!(
                "META\tclass_cache\t{scenario}\t{variant}\t{class_size}\t{}\t{}\t{}\t{}\t{}\t{}",
                stats.global_class_hits[class_index],
                stats.global_class_misses[class_index],
                stats.global_class_empty[class_index],
                stats.global_class_alignment_incompatible[class_index],
                blocks,
                bytes,
            );
        }
        for (bucket, count) in stats.allocation_size_histogram.iter().copied().enumerate() {
            if count != 0 {
                let size = if bucket + 1 == stats.allocation_size_histogram.len() {
                    "1024+".to_owned()
                } else {
                    (bucket * 8).to_string()
                };
                println!("META\tblock_size_bucket\t{scenario}\t{variant}\t{size}\t{count}");
            }
        }
    }
    Ok(())
}

/// Order in which the native and compact children of a scenario are executed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VariantOrder {
    Fixed,
    Alternate,
}

impl VariantOrder {
    fn name(self) -> &'static str {
        match self {
            VariantOrder::Fixed => "fixed",
            VariantOrder::Alternate => "alternate",
        }
    }

    fn from_name(value: &str) -> Option<Self> {
        match value {
            "fixed" => Some(VariantOrder::Fixed),
            "alternate" => Some(VariantOrder::Alternate),
            _ => None,
        }
    }

    /// Variant execution order for the scenario at `scenario_index`.
    ///
    /// `alternate` flips the order for every other scenario so host drift
    /// between the two children of a pair cannot systematically favour one
    /// variant across a whole suite.
    fn variants(self, scenario_index: usize) -> [&'static str; 2] {
        match (self, scenario_index % 2) {
            (VariantOrder::Alternate, 1) => ["compact", "native"],
            _ => ["native", "compact"],
        }
    }
}

fn run_child_process(
    executable: &Path,
    scenario: &str,
    variant: &str,
    repetitions: usize,
    mode: MeasurementMode,
) -> BenchResult<(Vec<PhaseRow>, Vec<String>, u64)> {
    let output = Command::new(executable)
        .arg("--child")
        .arg(scenario)
        .arg(variant)
        .arg(repetitions.to_string())
        .arg("--mode")
        .arg(mode.name())
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "child {scenario}/{variant} failed with {}:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let stdout = String::from_utf8(output.stdout)?;
    let mut rows = Vec::new();
    let mut metadata = Vec::new();
    let mut checksum = None;
    for line in stdout.lines() {
        if let Some(row) = parse_phase(line) {
            rows.push(row);
        } else if line.starts_with("META\tchecksum\t") {
            let fields: Vec<&str> = line.split('\t').collect();
            checksum = Some(
                fields
                    .get(4)
                    .ok_or_else(|| format!("invalid checksum row: {line}"))?
                    .parse::<u64>()?,
            );
            metadata.push(line.to_owned());
        } else if line.starts_with("META\t") {
            metadata.push(line.to_owned());
        }
    }
    let checksum =
        checksum.ok_or_else(|| format!("child {scenario}/{variant} did not report a checksum"))?;
    Ok((rows, metadata, checksum))
}

/// Verifies that dropping allocator accounting does not change the logical
/// workload: every selected scenario must produce the same checksum in
/// measure and profile mode, and the native and compact variants must agree
/// inside each mode.
fn run_self_check(executable: &Path, selected_scenarios: &[(&str, &str)]) -> BenchResult<()> {
    println!(
        "Self-check: measure/profile checksum parity, one repetition per child, {} scenario(s).",
        selected_scenarios.len()
    );
    let mut failures = Vec::new();
    for &(scenario, _) in selected_scenarios {
        let mut mode_checksums = Vec::new();
        for mode in [MeasurementMode::Measure, MeasurementMode::Profile] {
            let mut variant_checksums = Vec::new();
            for variant in ["native", "compact"] {
                let (_, _, checksum) = run_child_process(executable, scenario, variant, 1, mode)?;
                variant_checksums.push((variant, checksum));
            }
            if variant_checksums[0].1 != variant_checksums[1].1 {
                return Err(format!(
                    "self-check {scenario}: {}/{} checksums differ in {} mode ({:#x} vs {:#x})",
                    variant_checksums[0].0,
                    variant_checksums[1].0,
                    mode.name(),
                    variant_checksums[0].1,
                    variant_checksums[1].1
                )
                .into());
            }
            mode_checksums.push((mode, variant_checksums[0].1));
        }
        let (_, measure_checksum) = mode_checksums[0];
        let (_, profile_checksum) = mode_checksums[1];
        let parity = measure_checksum == profile_checksum;
        println!(
            "SELF-CHECK\t{scenario}\tmeasure={measure_checksum:#018x}\tprofile={profile_checksum:#018x}\t{}",
            if parity { "ok" } else { "MISMATCH" }
        );
        if !parity {
            failures.push(scenario);
        }
    }
    if failures.is_empty() {
        println!(
            "Self-check passed: all {} scenario(s) produced identical checksums with and without allocator accounting.",
            selected_scenarios.len()
        );
        Ok(())
    } else {
        Err(format!("self-check mode parity failed for: {}", failures.join(", ")).into())
    }
}

/// Recording that makes every saved artifact self-describing: harness contract
/// version, measurement mode, build profile, features, host and toolchain. The
/// parent process is not timed, so gathering this costs the workload nothing.
fn host_metadata(mode: MeasurementMode, order: VariantOrder) -> Vec<String> {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let load = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .map(|value| {
            value
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join("\t")
        })
        .unwrap_or_else(|| "unavailable\tunavailable\tunavailable".to_owned());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|| "unavailable".to_owned());
    let workers = std::thread::available_parallelism()
        .map(|value| value.get().to_string())
        .unwrap_or_else(|_| "unavailable".to_owned());
    let mut rows = vec![
        format!(
            "META\tharness\tbenchmark_compare\t{}\tcontract_v{MEASUREMENT_MODE_VERSION}",
            env!("CARGO_PKG_VERSION")
        ),
        format!(
            "META\tsuite\tmeasurement_mode\t{}\tcontract_v{MEASUREMENT_MODE_VERSION}\tvariant_order\t{}",
            mode.name(),
            order.name()
        ),
        format!(
            "META\tsuite\tbuild\t{profile}\t{}\t{}\t{}-bit\t{}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            usize::BITS,
            std::env::consts::FAMILY
        ),
        format!("META\tsuite\tfeatures\t{}", enabled_features()),
        format!("META\tsuite\tavailable_parallelism\t{workers}"),
        format!("META\tsuite\tloadavg_1_5_15\t{load}"),
        format!("META\tsuite\tkernel\t{kernel}"),
        format!("META\tsuite\thost_cpu\t{}", cpu_model()),
    ];
    rows.extend(toolchain_metadata());
    rows
}

fn enabled_features() -> String {
    let mut features = Vec::new();
    if cfg!(feature = "json") {
        features.push("json");
    }
    if cfg!(feature = "toml") {
        features.push("toml");
    }
    if cfg!(feature = "allocator-telemetry") {
        features.push("allocator-telemetry");
    }
    if cfg!(feature = "benchmark-allocator-a") {
        features.push("benchmark-allocator-a");
    }
    if cfg!(feature = "benchmark-allocator-b") {
        features.push("benchmark-allocator-b");
    }
    if cfg!(feature = "benchmark-allocator-c") {
        features.push("benchmark-allocator-c");
    }
    if features.is_empty() {
        "none".to_owned()
    } else {
        features.join(",")
    }
}

fn cpu_model() -> String {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    for line in cpuinfo.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if matches!(
            key.trim(),
            "model name" | "Model" | "Hardware" | "cpu model" | "Processor"
        ) {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_owned();
            }
        }
    }
    std::env::consts::ARCH.to_owned()
}

fn toolchain_metadata() -> Vec<String> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let Ok(output) = Command::new(rustc).arg("-vV").output() else {
        return vec!["META\tsuite\ttoolchain\tunavailable\tunavailable\tunavailable".to_owned()];
    };
    if !output.status.success() {
        return vec!["META\tsuite\ttoolchain\tunavailable\tunavailable\tunavailable".to_owned()];
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut version = "unknown".to_owned();
    let mut host = "unknown".to_owned();
    let mut llvm = "unknown".to_owned();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("rustc ") {
            version = rest.to_owned();
        } else if let Some(rest) = line.strip_prefix("host: ") {
            host = rest.to_owned();
        } else if let Some(rest) = line.strip_prefix("LLVM version: ") {
            llvm = rest.to_owned();
        }
    }
    vec![format!(
        "META\tsuite\ttoolchain\t{version}\t{host}\tLLVM {llvm}"
    )]
}

fn parse_positive(arguments: &[String], index: usize, option: &str) -> BenchResult<usize> {
    let value = arguments
        .get(index)
        .ok_or_else(|| format!("{option} requires a value"))?
        .parse::<usize>()?;
    if value == 0 {
        return Err(format!("{option} must be positive").into());
    }
    Ok(value)
}

fn parse_phase(line: &str) -> Option<PhaseRow> {
    let mut fields = line.split('\t');
    if fields.next()? != "PHASE" {
        return None;
    }
    Some(PhaseRow {
        scenario: fields.next()?.to_owned(),
        variant: fields.next()?.to_owned(),
        phase: fields.next()?.to_owned(),
        fields: fields.map(str::to_owned).collect(),
    })
}

fn print_report(rows: &[PhaseRow]) {
    println!(
        "{:<5} {:<20} {:<18} {:>19} {:>19} {:>9} {:>22} {:>14} {:>29} {:>12}",
        "ID",
        "Scenario",
        "Phase",
        "Native med/p95 ms",
        "V2.4 med/p95 ms",
        "Time x",
        "Native live/requested",
        "Cage live",
        "Aux live/req/peak",
        "Memory x"
    );
    let mut keys = std::collections::BTreeMap::<(String, String), ()>::new();
    for row in rows {
        keys.insert((row.scenario.clone(), row.phase.clone()), ());
    }
    for (scenario, phase) in keys.keys() {
        let native = rows.iter().find(|row| {
            row.scenario == *scenario && row.phase == *phase && row.variant == "native"
        });
        let compact = rows.iter().find(|row| {
            row.scenario == *scenario && row.phase == *phase && row.variant == "compact"
        });
        let (Some(native), Some(compact)) = (native, compact) else {
            continue;
        };
        let nat_median = field(native, 1).parse::<u128>().unwrap_or(0);
        let cmp_median = field(compact, 1).parse::<u128>().unwrap_or(0);
        let nat_p95 = field(native, 2).parse::<u128>().unwrap_or(0);
        let cmp_p95 = field(compact, 2).parse::<u128>().unwrap_or(0);
        let time_ratio = if nat_median == 0 {
            0.0
        } else {
            cmp_median as f64 / nat_median as f64
        };
        let native_build = rows.iter().find(|row| {
            row.scenario == *scenario && row.phase == "build" && row.variant == "native"
        });
        let Some(native_build) = native_build else {
            continue;
        };
        let native_live = field(native_build, 8).parse::<i64>().unwrap_or(0).max(0) as u64;
        let native_requested = field(native_build, 7).parse::<u64>().unwrap_or(0);
        let cage_live = build_value(rows, scenario, "compact", "build", 10);
        let aux_live = build_value(rows, scenario, "compact", "build", 8);
        let aux_requested = build_value(rows, scenario, "compact", "build", 7);
        let aux_peak = build_value(rows, scenario, "compact", "build", 9);
        let memory_ratio = if native_live == 0 {
            0.0
        } else {
            (cage_live + aux_live) as f64 / native_live as f64
        };
        println!(
            "{:<5} {:<20} {:<18} {:>19} {:>19} {:>8.2}x {:>10}/{:<10} {:>10}B {:>9}/{:<9}/{:<9} {:>10.3}x",
            scenario,
            scenario_description(scenario),
            phase,
            format!("{:.3}/{:.3}", nat_median as f64 / 1_000_000.0, nat_p95 as f64 / 1_000_000.0),
            format!("{:.3}/{:.3}", cmp_median as f64 / 1_000_000.0, cmp_p95 as f64 / 1_000_000.0),
            time_ratio,
            native_live,
            native_requested,
            cage_live,
            aux_live,
            aux_requested,
            aux_peak,
            memory_ratio,
        );
    }
}

fn build_value(rows: &[PhaseRow], scenario: &str, variant: &str, phase: &str, field: usize) -> u64 {
    rows.iter()
        .find(|row| row.scenario == scenario && row.variant == variant && row.phase == phase)
        .and_then(|row| row.fields.get(field))
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| value.max(0) as u64)
        .unwrap_or(0)
}

fn field(row: &PhaseRow, index: usize) -> &str {
    row.fields.get(index).map(String::as_str).unwrap_or("0")
}

fn scenario_description(scenario: &str) -> &'static str {
    scenarios::description(scenario)
}

fn write_tsv(path: &str, rows: &[PhaseRow], metadata: &[String]) -> BenchResult<()> {
    let mut file = fs::File::create(path)?;
    writeln!(
        file,
        "scenario\tvariant\tphase\truns\tmedian_ns\tp95_ns\tmin_ns\tmax_ns\tallocation_calls\tdeallocation_calls\trequested_bytes\tlive_delta_bytes\tpeak_extra_bytes\tcage_live_delta_bytes\tcage_high_water_cursor\tfree_bytes\tfree_blocks\tlargest_free_block\tallocator_lock_acquisitions\tfree_list_nodes_visited\trelease_batches\treleased_extents\tmax_release_batch\tsize_class_hits\tsize_class_misses\tpending_reuse_hits\tpending_reuse_misses\tpending_reuse_no_active_collector\tpending_reuse_no_exact_block\tpending_reuse_alignment_incompatible\tglobal_class_hits_32,40,112,528\tglobal_class_misses_32,40,112,528\tglobal_class_empty_32,40,112,528\tglobal_class_alignment_incompatible_32,40,112,528\trequested_size_no_class\tgeneral_list_fallbacks\tcursor_fallbacks\treleased_exact_size_extents\texact_size_extents_cached\texact_size_extents_coalesced_before_cache"
    )?;
    for row in rows {
        writeln!(
            file,
            "{}\t{}\t{}\t{}",
            row.scenario,
            row.variant,
            row.phase,
            row.fields.join("\t")
        )?;
    }
    writeln!(file, "\n# metadata")?;
    for line in metadata {
        writeln!(file, "{}", line)?;
    }
    Ok(())
}
