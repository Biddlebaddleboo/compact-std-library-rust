mod datasets;
mod measure;
mod models;
mod scenarios;

use measure::{BenchResult, CountingAllocator};
use std::fs;
use std::io::Write as _;
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
            other => return Err(format!("unknown argument {other:?}").into()),
        }
        index += 1;
    }

    let executable = std::env::current_exe()?;
    let mut rows = Vec::new();
    let mut metadata = Vec::new();
    let mut checksums = std::collections::BTreeMap::<String, u64>::new();
    for &(scenario, description) in scenarios::SCENARIOS {
        eprintln!("benchmark {scenario}: {description}");
        let repetitions = runs_override.unwrap_or_else(|| scenarios::repetitions_for(scenario));
        for variant in ["native", "compact"] {
            let output = Command::new(&executable)
                .arg("--child")
                .arg(scenario)
                .arg(variant)
                .arg(repetitions.to_string())
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
            let mut child_checksum = None;
            for line in stdout.lines() {
                if let Some(row) = parse_phase(line) {
                    rows.push(row);
                } else if line.starts_with("META\tchecksum\t") {
                    let fields: Vec<&str> = line.split('\t').collect();
                    let checksum = fields
                        .get(4)
                        .ok_or_else(|| format!("invalid checksum row: {line}"))?
                        .parse::<u64>()?;
                    child_checksum = Some(checksum);
                    metadata.push(line.to_owned());
                } else if line.starts_with("META\t") {
                    metadata.push(line.to_owned());
                }
            }
            let checksum = child_checksum
                .ok_or_else(|| format!("child {scenario}/{variant} did not report a checksum"))?;
            if variant == "native" {
                checksums.insert(scenario.to_owned(), checksum);
            } else if checksums.get(scenario) != Some(&checksum) {
                return Err(format!(
                    "{scenario} native/compact logical results differ: native={:?}, compact={checksum:#x}",
                    checksums.get(scenario)
                )
                .into());
            }
        }
    }

    print_report(&rows);
    println!(
        "\nLogical result checksums matched for all {} scenarios.",
        scenarios::SCENARIOS.len()
    );
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
    if variant == "compact" {
        compact_std::CompactRuntime::init(compact_std::CageConfig::new(CAGE_BYTES))?;
    } else if variant != "native" {
        return Err(format!("unknown benchmark variant {variant:?}").into());
    }
    scenarios::run(scenario, variant, repetitions)
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
        "scenario\tvariant\tphase\truns\tmedian_ns\tp95_ns\tmin_ns\tmax_ns\tallocation_calls\tdeallocation_calls\trequested_bytes\tlive_delta_bytes\tpeak_extra_bytes\tcage_live_delta_bytes\tcage_high_water_cursor\tfree_bytes\tfree_blocks\tlargest_free_block"
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
