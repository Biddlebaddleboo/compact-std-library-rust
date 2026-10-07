use crate::datasets;
use crate::measure::{self, BenchResult, Mutation, Read};
use crate::models::*;
use compact_std::{
    CompactBox, CompactBytes, CompactHashMap, CompactHashSet, CompactPathBuf, CompactRuntime,
    CompactString, CompactVec, CompactVecDeque, FrozenBuilder, FrozenGraph, FrozenGraphView,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error;
use std::hint::black_box;
use std::path::PathBuf;

pub const SCENARIOS: &[(&str, &str)] = &[
    ("A1", "vector build/traverse/churn"),
    ("A2", "box and object allocation"),
    ("A3", "string and byte lengths"),
    ("A4", "FIFO deque churn"),
    ("A5", "hash map and set churn"),
    ("A6", "path build and query"),
    ("B1", "10k record JSON API response"),
    ("B2", "large TOML service configuration"),
    ("B3", "24k request metadata batch"),
    ("B4", "bounded logging history churn"),
    ("B5", "mobility dispatch state"),
    ("B6", "market data order book"),
    ("B7", "100k filesystem catalog"),
    ("B8", "fixed population cache churn"),
    ("B9", "immutable frozen catalog"),
    ("B10", "concurrent worker state"),
];

pub fn description(scenario: &str) -> &'static str {
    SCENARIOS
        .iter()
        .find_map(|(id, description)| (*id == scenario).then_some(*description))
        .unwrap_or("unknown scenario")
}

pub fn repetitions_for(scenario: &str) -> usize {
    match scenario {
        "A1" | "A2" | "A3" | "A4" | "A5" | "A6" => 15,
        "B1" | "B2" | "B3" | "B4" | "B6" | "B8" => 9,
        "B5" | "B7" | "B9" | "B10" => 7,
        _ => 15,
    }
}

pub fn run(scenario: &str, variant: &str, repetitions: usize) -> BenchResult<()> {
    let compact = variant == "compact";
    let checksum = match scenario {
        "A1" => vector_build_churn(variant, repetitions)?,
        "A2" => box_objects(variant, repetitions)?,
        "A3" => strings_bytes(variant, repetitions)?,
        "A4" => deque_churn(variant, repetitions)?,
        "A5" => hash_churn(variant, repetitions)?,
        "A6" => paths(variant, repetitions)?,
        "B1" => json_api(variant, repetitions)?,
        "B2" => toml_config(variant, repetitions)?,
        "B3" => request_batch(variant, repetitions)?,
        "B4" => event_history(variant, repetitions)?,
        "B5" => dispatch(variant, repetitions)?,
        "B6" => order_book(variant, repetitions)?,
        "B7" => file_catalog(variant, repetitions)?,
        "B8" => cache_churn(variant, repetitions)?,
        "B9" => frozen_catalog(variant, repetitions)?,
        "B10" => concurrent_workers(variant, repetitions)?,
        _ => return Err(format!("unknown scenario {scenario:?}").into()),
    };
    if compact && CompactRuntime::used_bytes()? != 0 {
        return Err(format!("{scenario} retained cage bytes after scenario completion").into());
    }
    let _ = checksum;
    Ok(())
}

enum VectorState {
    Native(Vec<u32>),
    Compact(CompactVec<u32>),
}

fn vector_build_churn(variant: &str, repetitions: usize) -> BenchResult<u64> {
    const BASE: usize = 100_000;
    let compact = variant == "compact";
    let build = move || -> BenchResult<VectorState> {
        if compact {
            Ok(VectorState::Compact(CompactVec::try_from_iter(
                0..BASE as u32,
            )?))
        } else {
            Ok(VectorState::Native((0..BASE as u32).collect()))
        }
    };
    let mutation: Mutation<'_, VectorState> = (
        "mutation",
        Box::new(|state| {
            match state {
                VectorState::Native(values) => {
                    for cycle in 0..4_u32 {
                        values
                            .extend((0..50_000_u32).map(|value| 100_000 + cycle * 50_000 + value));
                        values.truncate(BASE);
                        values
                            .extend((0..25_000_u32).map(|value| 200_000 + cycle * 25_000 + value));
                        values.truncate(BASE);
                    }
                }
                VectorState::Compact(values) => {
                    for cycle in 0..4_u32 {
                        values.try_extend(
                            (0..50_000_u32).map(|value| 100_000 + cycle * 50_000 + value),
                        )?;
                        values.truncate(BASE);
                        values.try_extend(
                            (0..25_000_u32).map(|value| 200_000 + cycle * 25_000 + value),
                        )?;
                        values.truncate(BASE);
                    }
                }
            }
            Ok(0)
        }),
    );
    let read: Read<'_, VectorState> = (
        "traverse",
        Box::new(|state| {
            let checksum = match state {
                VectorState::Native(values) => values.iter().map(|value| u64::from(*value)).sum(),
                VectorState::Compact(values) => values.iter().map(|value| u64::from(*value)).sum(),
            };
            Ok(checksum)
        }),
    );
    measure::run_case(
        "A1",
        variant,
        repetitions,
        compact,
        build,
        vec![mutation],
        vec![read],
    )
}

enum BoxState {
    #[allow(clippy::vec_box)]
    Native(Vec<Box<(u64, u32)>>),
    Compact(CompactVec<CompactBox<(u64, u32)>>),
}

fn box_objects(variant: &str, repetitions: usize) -> BenchResult<u64> {
    const OBJECTS: usize = 25_000;
    let compact = variant == "compact";
    println!(
        "META\towner_size\tA2\t{}\t{}\t{}",
        variant,
        if compact {
            "CompactBox<(u64,u32)>"
        } else {
            "Box<(u64,u32)>"
        },
        if compact {
            std::mem::size_of::<CompactBox<(u64, u32)>>()
        } else {
            std::mem::size_of::<Box<(u64, u32)>>()
        }
    );
    let build = move || -> BenchResult<BoxState> {
        if compact {
            let mut values = CompactVec::with_capacity(OBJECTS)?;
            for value in 0..OBJECTS as u64 {
                values.push(CompactBox::new((value, (value % 97) as u32))?)?;
            }
            Ok(BoxState::Compact(values))
        } else {
            Ok(BoxState::Native(
                (0..OBJECTS as u64)
                    .map(|value| Box::new((value, (value % 97) as u32)))
                    .collect(),
            ))
        }
    };
    let read: Read<'_, BoxState> = (
        "traverse",
        Box::new(|state| {
            Ok(match state {
                BoxState::Native(values) => values
                    .iter()
                    .map(|value| value.0.wrapping_add(u64::from(value.1)))
                    .sum(),
                BoxState::Compact(values) => values
                    .iter()
                    .map(|value| value.0.wrapping_add(u64::from(value.1)))
                    .sum(),
            })
        }),
    );
    measure::run_case(
        "A2",
        variant,
        repetitions,
        compact,
        build,
        vec![],
        vec![read],
    )
}

enum StringState {
    Native(Vec<(String, Vec<u8>)>),
    Compact(CompactVec<(CompactString, CompactBytes)>),
}

fn strings_bytes(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let payloads = datasets::string_byte_payloads();
    let compact = variant == "compact";
    let build = move || -> BenchResult<StringState> {
        if compact {
            let mut values = CompactVec::with_capacity(payloads.len())?;
            for value in &payloads {
                values.push((
                    CompactString::from_str(value)?,
                    CompactBytes::from_slice(value.as_bytes())?,
                ))?;
            }
            Ok(StringState::Compact(values))
        } else {
            Ok(StringState::Native(
                payloads
                    .iter()
                    .map(|value| (value.clone(), value.as_bytes().to_vec()))
                    .collect(),
            ))
        }
    };
    let read: Read<'_, StringState> = (
        "traverse",
        Box::new(|state| {
            Ok(match state {
                StringState::Native(values) => values
                    .iter()
                    .map(|(text, bytes)| (text.len() + bytes.len()) as u64)
                    .sum(),
                StringState::Compact(values) => values
                    .iter()
                    .map(|(text, bytes)| (text.len() + bytes.len()) as u64)
                    .sum(),
            })
        }),
    );
    measure::run_case(
        "A3",
        variant,
        repetitions,
        compact,
        build,
        vec![],
        vec![read],
    )
}

enum DequeState {
    Native(VecDeque<u64>),
    Compact(CompactVecDeque<u64>),
}

fn deque_churn(variant: &str, repetitions: usize) -> BenchResult<u64> {
    const CAPACITY: usize = 4_096;
    const CHURN: u64 = 80_000;
    let compact = variant == "compact";
    let build = move || -> BenchResult<DequeState> {
        if compact {
            let mut queue = CompactVecDeque::with_capacity(CAPACITY)?;
            for value in 0..CAPACITY as u64 {
                queue.push_back(value)?;
            }
            Ok(DequeState::Compact(queue))
        } else {
            Ok(DequeState::Native((0..CAPACITY as u64).collect()))
        }
    };
    let mutation: Mutation<'_, DequeState> = (
        "mutation",
        Box::new(|state| {
            match state {
                DequeState::Native(queue) => {
                    for value in 0..CHURN {
                        queue.push_back(CAPACITY as u64 + value);
                        black_box(queue.pop_front().expect("queue remains populated"));
                    }
                }
                DequeState::Compact(queue) => {
                    for value in 0..CHURN {
                        queue.push_back(CAPACITY as u64 + value)?;
                        black_box(queue.pop_front().expect("queue remains populated"));
                    }
                }
            }
            Ok(0)
        }),
    );
    let read: Read<'_, DequeState> = (
        "traverse",
        Box::new(|state| {
            Ok(match state {
                DequeState::Native(queue) => queue.iter().copied().sum(),
                DequeState::Compact(queue) => queue.iter().copied().sum(),
            })
        }),
    );
    measure::run_case(
        "A4",
        variant,
        repetitions,
        compact,
        build,
        vec![mutation],
        vec![read],
    )
}

enum HashState {
    Native(HashMap<u32, u64>, HashSet<u32>),
    Compact(CompactHashMap<u32, u64>, CompactHashSet<u32>),
}

fn hash_churn(variant: &str, repetitions: usize) -> BenchResult<u64> {
    const ENTRIES: usize = 16_000;
    const CHURN: u32 = 4_000;
    let compact = variant == "compact";
    let build = move || -> BenchResult<HashState> {
        if compact {
            let mut map = CompactHashMap::with_capacity(ENTRIES)?;
            let mut set = CompactHashSet::with_capacity(ENTRIES)?;
            for key in 0..ENTRIES as u32 {
                map.insert(key, u64::from(key) * 17)?;
                set.insert(key)?;
            }
            Ok(HashState::Compact(map, set))
        } else {
            let mut map = HashMap::with_capacity(ENTRIES);
            let mut set = HashSet::with_capacity(ENTRIES);
            for key in 0..ENTRIES as u32 {
                map.insert(key, u64::from(key) * 17);
                set.insert(key);
            }
            Ok(HashState::Native(map, set))
        }
    };
    let mutation: Mutation<'_, HashState> = (
        "mutation",
        Box::new(|state| {
            match state {
                HashState::Native(map, set) => {
                    for key in 0..CHURN {
                        if let Some(value) = map.get_mut(&key) {
                            *value = value.wrapping_add(1);
                        }
                        if key % 7 == 0 {
                            map.remove(&key);
                            set.remove(&key);
                        }
                        map.insert(32_000 + key, u64::from(32_000 + key) * 17);
                        set.insert(32_000 + key);
                    }
                }
                HashState::Compact(map, set) => {
                    for key in 0..CHURN {
                        if let Some(value) = map.get_mut(&key) {
                            *value = value.wrapping_add(1);
                        }
                        if key % 7 == 0 {
                            map.remove(&key);
                            set.remove(&key);
                        }
                        map.insert(32_000 + key, u64::from(32_000 + key) * 17)?;
                        set.insert(32_000 + key)?;
                    }
                }
            }
            Ok(0)
        }),
    );
    let read: Read<'_, HashState> = (
        "lookup_scan",
        Box::new(|state| {
            let mut checksum = 0_u64;
            match state {
                HashState::Native(map, set) => {
                    for key in (0..36_000_u32).step_by(13) {
                        checksum = checksum.wrapping_add(map.get(&key).copied().unwrap_or(0));
                        checksum = checksum.wrapping_add(u64::from(set.contains(&key)));
                    }
                    checksum = checksum.wrapping_add(
                        map.iter()
                            .map(|(key, value)| u64::from(*key) + *value)
                            .sum::<u64>(),
                    );
                }
                HashState::Compact(map, set) => {
                    for key in (0..36_000_u32).step_by(13) {
                        checksum = checksum.wrapping_add(map.get(&key).copied().unwrap_or(0));
                        checksum = checksum.wrapping_add(u64::from(set.contains(&key)));
                    }
                    checksum = checksum.wrapping_add(
                        map.iter()
                            .map(|(key, value)| u64::from(*key) + *value)
                            .sum::<u64>(),
                    );
                }
            }
            Ok(checksum)
        }),
    );
    measure::run_case(
        "A5",
        variant,
        repetitions,
        compact,
        build,
        vec![mutation],
        vec![read],
    )
}

enum PathState {
    Native(Vec<PathBuf>),
    Compact(CompactVec<CompactPathBuf>),
}

fn paths(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let strings = datasets::path_strings(6_000);
    let compact = variant == "compact";
    let build = move || -> BenchResult<PathState> {
        if compact {
            let mut values = CompactVec::with_capacity(strings.len())?;
            for value in &strings {
                values.push(CompactPathBuf::from(value.as_str())?)?;
            }
            Ok(PathState::Compact(values))
        } else {
            Ok(PathState::Native(
                strings.iter().map(PathBuf::from).collect(),
            ))
        }
    };
    let read: Read<'_, PathState> = (
        "lookup_scan",
        Box::new(|state| {
            let mut checksum = 0_u64;
            match state {
                PathState::Native(values) => {
                    for (index, path) in values.iter().enumerate() {
                        if path.is_absolute() && path.starts_with("/srv/dispatch") {
                            checksum = checksum.wrapping_add(index as u64 + 1);
                        }
                    }
                    for index in (0..values.len()).step_by(97) {
                        checksum = checksum.wrapping_add(values[index].as_os_str().len() as u64);
                    }
                }
                PathState::Compact(values) => {
                    for (index, path) in values.iter().enumerate() {
                        if path.is_absolute() && path.starts_with("/srv/dispatch") {
                            checksum = checksum.wrapping_add(index as u64 + 1);
                        }
                    }
                    for index in (0..values.len()).step_by(97) {
                        checksum = checksum
                            .wrapping_add(values[index].as_os_str().as_bytes().len() as u64);
                    }
                }
            }
            Ok(checksum)
        }),
    );
    measure::run_case(
        "A6",
        variant,
        repetitions,
        compact,
        build,
        vec![],
        vec![read],
    )
}

enum ApiState {
    Native(NativeApiResponse),
    Compact(CompactApiResponse),
}

fn json_api(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let payload = datasets::json_api_payload();
    let compact = variant == "compact";
    let build = move || -> BenchResult<ApiState> {
        if compact {
            Ok(ApiState::Compact(compact_std::json::from_str(&payload)?))
        } else {
            Ok(ApiState::Native(serde_json::from_str(&payload)?))
        }
    };
    let traverse: Read<'_, ApiState> = (
        "traverse",
        Box::new(|state| {
            Ok(match state {
                ApiState::Native(response) => response.records.iter().map(native_api_record).sum(),
                ApiState::Compact(response) => {
                    response.records.iter().map(compact_api_record).sum()
                }
            })
        }),
    );
    let selected: Read<'_, ApiState> = (
        "selected_lookup",
        Box::new(|state| {
            let mut checksum = 0_u64;
            match state {
                ApiState::Native(response) => {
                    for index in (0..response.records.len()).step_by(113) {
                        checksum =
                            checksum.wrapping_add(native_api_record(&response.records[index]));
                    }
                }
                ApiState::Compact(response) => {
                    for index in (0..response.records.len()).step_by(113) {
                        checksum =
                            checksum.wrapping_add(compact_api_record(&response.records[index]));
                    }
                }
            }
            Ok(checksum)
        }),
    );
    measure::run_case(
        "B1",
        variant,
        repetitions,
        compact,
        build,
        vec![],
        vec![traverse, selected],
    )
}

fn native_api_record(record: &NativeApiRecord) -> u64 {
    let tags: u64 = record.tags.iter().map(|value| value.len() as u64).sum();
    let flags: u64 = record
        .metadata
        .flags
        .iter()
        .map(|value| u64::from(*value))
        .sum();
    u64::from(record.id)
        .wrapping_add(record.name.len() as u64)
        .wrapping_add(record.status.len() as u64)
        .wrapping_add(u64::from(record.enabled))
        .wrapping_add(record.note.as_ref().map_or(0, |value| value.len() as u64))
        .wrapping_add(tags)
        .wrapping_add(record.metadata.source.len() as u64)
        .wrapping_add(u64::from(record.metadata.revision))
        .wrapping_add(flags)
        .wrapping_add(record.timestamp)
}

fn compact_api_record(record: &CompactApiRecord) -> u64 {
    let tags: u64 = record.tags.iter().map(|value| value.len() as u64).sum();
    let flags: u64 = record
        .metadata
        .flags
        .iter()
        .map(|value| u64::from(*value))
        .sum();
    u64::from(record.id)
        .wrapping_add(record.name.len() as u64)
        .wrapping_add(record.status.len() as u64)
        .wrapping_add(u64::from(record.enabled))
        .wrapping_add(record.note.as_ref().map_or(0, |value| value.len() as u64))
        .wrapping_add(tags)
        .wrapping_add(record.metadata.source.len() as u64)
        .wrapping_add(u64::from(record.metadata.revision))
        .wrapping_add(flags)
        .wrapping_add(record.timestamp)
}

enum ConfigState {
    Native(NativeServiceConfig),
    Compact(CompactServiceConfig),
}

fn toml_config(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let payload = datasets::toml_service_config();
    let compact = variant == "compact";
    let build = move || -> BenchResult<ConfigState> {
        if compact {
            Ok(ConfigState::Compact(compact_std::toml::from_str(&payload)?))
        } else {
            Ok(ConfigState::Native(toml::from_str(&payload)?))
        }
    };
    let reads: Read<'_, ConfigState> = (
        "repeated_reads",
        Box::new(|state| {
            let mut checksum = 0_u64;
            match state {
                ConfigState::Native(config) => {
                    for _ in 0..32 {
                        checksum = checksum.wrapping_add(config.service.len() as u64);
                        checksum = checksum.wrapping_add(config.retries as u64 + config.timeout_ms);
                        checksum = checksum.wrapping_add(
                            config.endpoints.iter().map(|s| s.len() as u64).sum::<u64>(),
                        );
                        checksum = checksum.wrapping_add(
                            config.paths.iter().filter(|p| p.is_absolute()).count() as u64,
                        );
                        checksum = checksum.wrapping_add(
                            config.feature_flags.iter().filter(|&&flag| flag).count() as u64,
                        );
                        checksum = checksum.wrapping_add(
                            config
                                .workers
                                .iter()
                                .map(|w| {
                                    w.name.len() as u64
                                        + w.threads as u64
                                        + w.retries as u64
                                        + w.timeout_ms
                                })
                                .sum::<u64>(),
                        );
                        checksum = checksum.wrapping_add(
                            config
                                .routes
                                .iter()
                                .map(|r| {
                                    r.prefix.len() as u64
                                        + r.target.len() as u64
                                        + r.methods.iter().map(|m| m.len() as u64).sum::<u64>()
                                })
                                .sum::<u64>(),
                        );
                        checksum = checksum.wrapping_add(
                            config
                                .labels
                                .iter()
                                .map(|(k, v)| (k.len() + v.len()) as u64)
                                .sum::<u64>(),
                        );
                    }
                }
                ConfigState::Compact(config) => {
                    for _ in 0..32 {
                        checksum = checksum.wrapping_add(config.service.len() as u64);
                        checksum = checksum.wrapping_add(config.retries as u64 + config.timeout_ms);
                        checksum = checksum.wrapping_add(
                            config.endpoints.iter().map(|s| s.len() as u64).sum::<u64>(),
                        );
                        checksum = checksum.wrapping_add(
                            config.paths.iter().filter(|p| p.is_absolute()).count() as u64,
                        );
                        checksum = checksum.wrapping_add(
                            config.feature_flags.iter().filter(|&&flag| flag).count() as u64,
                        );
                        checksum = checksum.wrapping_add(
                            config
                                .workers
                                .iter()
                                .map(|w| {
                                    w.name.len() as u64
                                        + w.threads as u64
                                        + w.retries as u64
                                        + w.timeout_ms
                                })
                                .sum::<u64>(),
                        );
                        checksum = checksum.wrapping_add(
                            config
                                .routes
                                .iter()
                                .map(|r| {
                                    r.prefix.len() as u64
                                        + r.target.len() as u64
                                        + r.methods.iter().map(|m| m.len() as u64).sum::<u64>()
                                })
                                .sum::<u64>(),
                        );
                        checksum = checksum.wrapping_add(
                            config
                                .labels
                                .iter()
                                .map(|(k, v)| (k.len() + v.len()) as u64)
                                .sum::<u64>(),
                        );
                    }
                }
            }
            Ok(checksum)
        }),
    );
    measure::run_case(
        "B2",
        variant,
        repetitions,
        compact,
        build,
        vec![],
        vec![reads],
    )
}

enum RequestState {
    Native(Vec<NativeRequestRecord>),
    Compact(CompactVec<CompactRequestRecord>),
}

fn request_batch(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let seeds = datasets::requests();
    let compact = variant == "compact";
    let build = move || -> BenchResult<RequestState> {
        if compact {
            let mut requests = CompactVec::with_capacity(seeds.len())?;
            for seed in &seeds {
                let mut headers = CompactVec::with_capacity(seed.headers.len())?;
                for (name, value) in &seed.headers {
                    headers.push((
                        CompactString::from_str(name)?,
                        CompactString::from_str(value)?,
                    ))?;
                }
                let mut tags = CompactVec::with_capacity(seed.tags.len())?;
                for tag in &seed.tags {
                    tags.push(CompactString::from_str(tag)?)?;
                }
                requests.push(CompactRequestRecord {
                    method: CompactString::from_str(&seed.method)?,
                    path: CompactString::from_str(&seed.path)?,
                    host: CompactString::from_str(&seed.host)?,
                    status: seed.status,
                    content_length: seed.content_length,
                    headers,
                    request_id: CompactString::from_str(&seed.request_id)?,
                    trace_id: CompactString::from_str(&seed.trace_id)?,
                    tags,
                })?;
            }
            Ok(RequestState::Compact(requests))
        } else {
            Ok(RequestState::Native(
                seeds
                    .iter()
                    .map(|seed| NativeRequestRecord {
                        method: seed.method.clone(),
                        path: seed.path.clone(),
                        host: seed.host.clone(),
                        status: seed.status,
                        content_length: seed.content_length,
                        headers: seed
                            .headers
                            .iter()
                            .map(|(name, value)| NativeHeader {
                                name: name.clone(),
                                value: value.clone(),
                            })
                            .collect(),
                        request_id: seed.request_id.clone(),
                        trace_id: seed.trace_id.clone(),
                        tags: seed.tags.clone(),
                    })
                    .collect(),
            ))
        }
    };
    let status_scan: Read<'_, RequestState> = (
        "status_scan",
        Box::new(|state| {
            let mut checksum = 0_u64;
            match state {
                RequestState::Native(requests) => {
                    for request in requests {
                        if request.status >= 500 || request.status == 401 {
                            checksum = checksum
                                .wrapping_add(request.content_length)
                                .wrapping_add(u64::from(request.status));
                        }
                    }
                }
                RequestState::Compact(requests) => {
                    for request in requests {
                        if request.status >= 500 || request.status == 401 {
                            checksum = checksum
                                .wrapping_add(request.content_length)
                                .wrapping_add(u64::from(request.status));
                        }
                    }
                }
            }
            Ok(checksum)
        }),
    );
    let lookups: Read<'_, RequestState> = (
        "selected_lookups",
        Box::new(|state| {
            let mut checksum = 0_u64;
            match state {
                RequestState::Native(requests) => {
                    for index in (0..requests.len()).step_by(101) {
                        let request = &requests[index];
                        checksum = checksum
                            .wrapping_add(request.method.len() as u64)
                            .wrapping_add(request.path.len() as u64)
                            .wrapping_add(request.host.len() as u64)
                            .wrapping_add(request.request_id.len() as u64)
                            .wrapping_add(request.trace_id.len() as u64)
                            .wrapping_add(
                                request
                                    .headers
                                    .iter()
                                    .map(|h| (h.name.len() + h.value.len()) as u64)
                                    .sum::<u64>(),
                            )
                            .wrapping_add(
                                request.tags.iter().map(|tag| tag.len() as u64).sum::<u64>(),
                            );
                    }
                }
                RequestState::Compact(requests) => {
                    for index in (0..requests.len()).step_by(101) {
                        let request = &requests[index];
                        checksum = checksum
                            .wrapping_add(request.method.len() as u64)
                            .wrapping_add(request.path.len() as u64)
                            .wrapping_add(request.host.len() as u64)
                            .wrapping_add(request.request_id.len() as u64)
                            .wrapping_add(request.trace_id.len() as u64)
                            .wrapping_add(
                                request
                                    .headers
                                    .iter()
                                    .map(|(name, value)| (name.len() + value.len()) as u64)
                                    .sum::<u64>(),
                            )
                            .wrapping_add(
                                request.tags.iter().map(|tag| tag.len() as u64).sum::<u64>(),
                            );
                    }
                }
            }
            Ok(checksum)
        }),
    );
    measure::run_case(
        "B3",
        variant,
        repetitions,
        compact,
        build,
        vec![],
        vec![status_scan, lookups],
    )
}

enum EventState {
    Native(VecDeque<NativeEvent>),
    Compact(CompactVecDeque<CompactEvent>),
}

fn event_history(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let initial = datasets::events();
    let churn: Vec<_> = (0..datasets::EVENT_CHURN as u64)
        .map(datasets::event_seed)
        .collect();
    let compact = variant == "compact";
    let build = move || -> BenchResult<EventState> {
        if compact {
            let mut queue = CompactVecDeque::with_capacity(datasets::EVENT_HISTORY)?;
            for event in &initial {
                queue.push_back(compact_event(event)?)?;
            }
            Ok(EventState::Compact(queue))
        } else {
            let mut queue = VecDeque::with_capacity(datasets::EVENT_HISTORY);
            for event in &initial {
                queue.push_back(native_event(event));
            }
            Ok(EventState::Native(queue))
        }
    };
    let mutation: Mutation<'_, EventState> = (
        "steady_state_mutation",
        Box::new(|state| {
            match state {
                EventState::Native(queue) => {
                    for event in &churn {
                        queue.push_back(native_event(event));
                        black_box(queue.pop_front().expect("bounded queue stays populated"));
                    }
                }
                EventState::Compact(queue) => {
                    for event in &churn {
                        queue.push_back(compact_event(event)?)?;
                        black_box(queue.pop_front().expect("bounded queue stays populated"));
                    }
                }
            }
            Ok(0)
        }),
    );
    let read: Read<'_, EventState> = (
        "retained_scan",
        Box::new(|state| {
            let mut checksum = 0_u64;
            match state {
                EventState::Native(queue) => {
                    for event in queue.iter() {
                        checksum = checksum
                            .wrapping_add(event.timestamp)
                            .wrapping_add(u64::from(event.severity))
                            .wrapping_add(event.component.len() as u64)
                            .wrapping_add(event.message.len() as u64)
                            .wrapping_add(
                                event
                                    .context
                                    .iter()
                                    .map(|value| u64::from(*value))
                                    .sum::<u64>(),
                            )
                            .wrapping_add(
                                event.request_id.as_ref().map_or(0, |id| id.len() as u64),
                            );
                    }
                }
                EventState::Compact(queue) => {
                    for event in queue.iter() {
                        checksum = checksum
                            .wrapping_add(event.timestamp)
                            .wrapping_add(u64::from(event.severity))
                            .wrapping_add(event.component.len() as u64)
                            .wrapping_add(event.message.len() as u64)
                            .wrapping_add(
                                event
                                    .context
                                    .iter()
                                    .map(|value| u64::from(*value))
                                    .sum::<u64>(),
                            )
                            .wrapping_add(
                                event.request_id.as_ref().map_or(0, |id| id.len() as u64),
                            );
                    }
                }
            }
            Ok(checksum)
        }),
    );
    measure::run_case(
        "B4",
        variant,
        repetitions,
        compact,
        build,
        vec![mutation],
        vec![read],
    )
}

fn native_event(seed: &datasets::EventSeed) -> NativeEvent {
    NativeEvent {
        timestamp: seed.timestamp,
        severity: seed.severity,
        component: seed.component.clone(),
        message: seed.message.clone(),
        context: seed.context,
        request_id: seed.request_id.clone(),
    }
}

fn compact_event(seed: &datasets::EventSeed) -> BenchResult<CompactEvent> {
    Ok(CompactEvent {
        timestamp: seed.timestamp,
        severity: seed.severity,
        component: CompactString::from_str(&seed.component)?,
        message: CompactString::from_str(&seed.message)?,
        context: seed.context,
        request_id: seed
            .request_id
            .as_deref()
            .map(CompactString::from_str)
            .transpose()?,
    })
}

enum DispatchState {
    Native(HashMap<u64, NativeOffer>),
    Compact(CompactHashMap<u64, CompactOffer>),
}

fn dispatch(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let seeds = datasets::dispatch_offers();
    let compact = variant == "compact";
    let build = || -> BenchResult<DispatchState> {
        if compact {
            let mut offers = CompactHashMap::with_capacity(seeds.len())?;
            for seed in &seeds {
                offers.insert(seed.id, compact_offer(seed, seed.id)?)?;
            }
            Ok(DispatchState::Compact(offers))
        } else {
            let mut offers = HashMap::with_capacity(seeds.len());
            for seed in &seeds {
                offers.insert(seed.id, seed.clone());
            }
            Ok(DispatchState::Native(offers))
        }
    };
    let mutation: Mutation<'_, DispatchState> = (
        "status_and_expiry_updates",
        Box::new(|state| {
            match state {
                DispatchState::Native(offers) => {
                    for index in 0..4_000_usize {
                        offers.remove(&(index as u64));
                        let source = &seeds[datasets::DISPATCH_OFFERS - 4_000 + index];
                        let id = (datasets::DISPATCH_OFFERS + index) as u64;
                        offers.insert(id, clone_offer_with_id(source, id));
                    }
                    for (_, offer) in offers.iter_mut() {
                        offer.status = offer.status.wrapping_add(1) % 5;
                    }
                }
                DispatchState::Compact(offers) => {
                    for index in 0..4_000_usize {
                        offers.remove(&(index as u64));
                        let source = &seeds[datasets::DISPATCH_OFFERS - 4_000 + index];
                        let id = (datasets::DISPATCH_OFFERS + index) as u64;
                        offers.insert(id, compact_offer(source, id)?)?;
                    }
                    for (_, offer) in offers.iter_mut() {
                        offer.status = offer.status.wrapping_add(1) % 5;
                    }
                }
            }
            Ok(0)
        }),
    );
    let reads: Read<'_, DispatchState> = (
        "zone_status_and_id_queries",
        Box::new(|state| {
            let mut checksum = 0_u64;
            match state {
                DispatchState::Native(offers) => {
                    for offer in offers.values() {
                        if offer.pickup_zone % 19 == 0 && offer.status % 2 == 0 {
                            checksum = checksum
                                .wrapping_add(offer.id)
                                .wrapping_add(u64::from(offer.distance_m))
                                .wrapping_add(u64::from(offer.eta_s))
                                .wrapping_add(u64::from(offer.dropoff_zone))
                                .wrapping_add(offer.latitude_e6 as u32 as u64)
                                .wrapping_add(offer.longitude_e6 as u32 as u64)
                                .wrapping_add(offer.address.len() as u64)
                                .wrapping_add(offer.provider.len() as u64)
                                .wrapping_add(
                                    offer.rider.as_ref().map_or(0, |rider| rider.len() as u64),
                                )
                                .wrapping_add(
                                    offer
                                        .route_tags
                                        .iter()
                                        .map(|tag| u64::from(*tag))
                                        .sum::<u64>(),
                                );
                        }
                    }
                    for id in (0..datasets::DISPATCH_OFFERS as u64).step_by(97) {
                        checksum =
                            checksum.wrapping_add(offers.get(&id).map_or(0, |offer| offer.id));
                    }
                }
                DispatchState::Compact(offers) => {
                    for offer in offers.values() {
                        if offer.pickup_zone % 19 == 0 && offer.status % 2 == 0 {
                            checksum = checksum
                                .wrapping_add(offer.id)
                                .wrapping_add(u64::from(offer.distance_m))
                                .wrapping_add(u64::from(offer.eta_s))
                                .wrapping_add(u64::from(offer.dropoff_zone))
                                .wrapping_add(offer.latitude_e6 as u32 as u64)
                                .wrapping_add(offer.longitude_e6 as u32 as u64)
                                .wrapping_add(offer.address.len() as u64)
                                .wrapping_add(offer.provider.len() as u64)
                                .wrapping_add(
                                    offer.rider.as_ref().map_or(0, |rider| rider.len() as u64),
                                )
                                .wrapping_add(
                                    offer
                                        .route_tags
                                        .iter()
                                        .map(|tag| u64::from(*tag))
                                        .sum::<u64>(),
                                );
                        }
                    }
                    for id in (0..datasets::DISPATCH_OFFERS as u64).step_by(97) {
                        checksum =
                            checksum.wrapping_add(offers.get(&id).map_or(0, |offer| offer.id));
                    }
                }
            }
            Ok(checksum)
        }),
    );
    measure::run_case(
        "B5",
        variant,
        repetitions,
        compact,
        build,
        vec![mutation],
        vec![reads],
    )
}

fn clone_offer_with_id(seed: &NativeOffer, id: u64) -> NativeOffer {
    let mut offer = seed.clone();
    offer.id = id;
    offer
}

fn compact_offer(seed: &NativeOffer, id: u64) -> BenchResult<CompactOffer> {
    Ok(CompactOffer {
        id,
        pickup_zone: seed.pickup_zone,
        dropoff_zone: seed.dropoff_zone,
        latitude_e6: seed.latitude_e6,
        longitude_e6: seed.longitude_e6,
        distance_m: seed.distance_m,
        eta_s: seed.eta_s,
        provider: CompactString::from_str(&seed.provider)?,
        status: seed.status,
        address: CompactString::from_str(&seed.address)?,
        rider: seed
            .rider
            .as_deref()
            .map(CompactString::from_str)
            .transpose()?,
        route_tags: seed.route_tags,
    })
}

enum OrderBookState {
    Native(NativeOrderBook),
    Compact(CompactOrderBook),
}

fn order_book(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let level_data = datasets::order_book_levels();
    let compact = variant == "compact";
    let build = move || -> BenchResult<OrderBookState> {
        let midpoint = level_data.len() / 2;
        if compact {
            let mut bids = CompactVec::with_capacity(midpoint)?;
            let mut asks = CompactVec::with_capacity(midpoint)?;
            for level in level_data[..midpoint].iter().rev() {
                bids.push(*level)?;
            }
            for level in &level_data[midpoint..] {
                asks.push(*level)?;
            }
            Ok(OrderBookState::Compact(CompactOrderBook {
                symbol: CompactString::from_str("ACME-USD")?,
                venue: CompactString::from_str("northstar")?,
                bids,
                asks,
            }))
        } else {
            Ok(OrderBookState::Native(NativeOrderBook {
                symbol: "ACME-USD".to_owned(),
                venue: "northstar".to_owned(),
                bids: level_data[..midpoint].iter().rev().copied().collect(),
                asks: level_data[midpoint..].to_vec(),
            }))
        }
    };
    let mutation: Mutation<'_, OrderBookState> =
        (
            "quote_updates_and_snapshot_rebuilds",
            Box::new(|state| {
                match state {
                    OrderBookState::Native(book) => {
                        for round in 0..8_usize {
                            for update in 0..192_usize {
                                let index = (round * 137 + update * 31) % book.bids.len();
                                let level = &mut book.bids[index];
                                level.quantity = if update % 11 == 0 {
                                    0
                                } else {
                                    level.quantity.saturating_sub(1 + (update % 13) as u64)
                                };
                                level.order_count = level
                                    .order_count
                                    .saturating_sub(if update % 11 == 0 { 1 } else { 0 });
                                let ask_index = (round * 73 + update * 17) % book.asks.len();
                                let ask = &mut book.asks[ask_index];
                                ask.quantity = if update % 13 == 0 {
                                    0
                                } else {
                                    ask.quantity.saturating_sub(1 + (update % 9) as u64)
                                };
                                ask.order_count = ask
                                    .order_count
                                    .saturating_sub(if update % 13 == 0 { 1 } else { 0 });
                            }
                            book.bids.retain(|level| level.quantity != 0);
                            book.asks.retain(|level| level.quantity != 0);
                            if round % 2 == 1 {
                                book.bids = book.bids.clone();
                                book.asks = book.asks.clone();
                            }
                        }
                    }
                    OrderBookState::Compact(book) => {
                        for round in 0..8_usize {
                            for update in 0..192_usize {
                                let index = (round * 137 + update * 31) % book.bids.len();
                                let level = &mut book.bids[index];
                                level.quantity = if update % 11 == 0 {
                                    0
                                } else {
                                    level.quantity.saturating_sub(1 + (update % 13) as u64)
                                };
                                level.order_count = level
                                    .order_count
                                    .saturating_sub(if update % 11 == 0 { 1 } else { 0 });
                                let ask_index = (round * 73 + update * 17) % book.asks.len();
                                let ask = &mut book.asks[ask_index];
                                ask.quantity = if update % 13 == 0 {
                                    0
                                } else {
                                    ask.quantity.saturating_sub(1 + (update % 9) as u64)
                                };
                                ask.order_count = ask
                                    .order_count
                                    .saturating_sub(if update % 13 == 0 { 1 } else { 0 });
                            }
                            compact_retain_levels(&mut book.bids);
                            compact_retain_levels(&mut book.asks);
                            if round % 2 == 1 {
                                book.bids = book.bids.try_clone()?;
                                book.asks = book.asks.try_clone()?;
                            }
                        }
                    }
                }
                Ok(0)
            }),
        );
    let read: Read<'_, OrderBookState> = (
        "best_price_and_depth",
        Box::new(|state| {
            let checksum = match state {
                OrderBookState::Native(book) => {
                    let best_bid = book.bids.first().map_or(0, |level| level.price_micros);
                    let best_ask = book.asks.first().map_or(0, |level| level.price_micros);
                    let bid_depth: u64 = book.bids.iter().map(|level| level.quantity).sum();
                    let ask_depth: u64 = book.asks.iter().map(|level| level.quantity).sum();
                    let flags: u64 = book
                        .bids
                        .iter()
                        .chain(&book.asks)
                        .map(|level| u64::from(level.flags))
                        .sum();
                    best_bid
                        + best_ask
                        + bid_depth
                        + ask_depth
                        + flags
                        + book.symbol.len() as u64
                        + book.venue.len() as u64
                }
                OrderBookState::Compact(book) => {
                    let best_bid = book.bids.first().map_or(0, |level| level.price_micros);
                    let best_ask = book.asks.first().map_or(0, |level| level.price_micros);
                    let bid_depth: u64 = book.bids.iter().map(|level| level.quantity).sum();
                    let ask_depth: u64 = book.asks.iter().map(|level| level.quantity).sum();
                    let flags: u64 = book
                        .bids
                        .iter()
                        .chain(book.asks.iter())
                        .map(|level| u64::from(level.flags))
                        .sum();
                    best_bid
                        + best_ask
                        + bid_depth
                        + ask_depth
                        + flags
                        + book.symbol.len() as u64
                        + book.venue.len() as u64
                }
            };
            Ok(checksum)
        }),
    );
    let snapshots: Read<'_, OrderBookState> = (
        "snapshot_copy",
        Box::new(|state| {
            Ok(match state {
                OrderBookState::Native(book) => book
                    .bids
                    .clone()
                    .into_iter()
                    .chain(book.asks.clone())
                    .map(|level| level.price_micros ^ level.quantity ^ u64::from(level.flags))
                    .sum(),
                OrderBookState::Compact(book) => {
                    let mut bids = book.bids.try_clone()?;
                    let mut asks = book.asks.try_clone()?;
                    let checksum: u64 = bids
                        .iter()
                        .chain(asks.iter())
                        .map(|level| level.price_micros ^ level.quantity ^ u64::from(level.flags))
                        .sum();
                    bids.clear();
                    asks.clear();
                    checksum
                }
            })
        }),
    );
    measure::run_case(
        "B6",
        variant,
        repetitions,
        compact,
        build,
        vec![mutation],
        vec![read, snapshots],
    )
}

fn compact_retain_levels(levels: &mut CompactVec<PriceLevel>) {
    let mut output = 0;
    for input in 0..levels.len() {
        if levels[input].quantity != 0 {
            if output != input {
                levels[output] = levels[input];
            }
            output += 1;
        }
    }
    levels.truncate(output);
}

enum FileCatalogState {
    Native(Vec<NativeFileRecord>),
    Compact(CompactVec<CompactFileRecord>),
}

fn file_catalog(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let seeds = datasets::file_records();
    let compact = variant == "compact";
    let build = || -> BenchResult<FileCatalogState> {
        if compact {
            let mut records = CompactVec::with_capacity(seeds.len())?;
            for seed in &seeds {
                records.push(CompactFileRecord {
                    path: CompactPathBuf::from(&seed.path)?,
                    size: seed.size,
                    modified: seed.modified,
                    file_type: seed.file_type,
                    category: CompactString::from_str(&seed.category)?,
                    hash: seed.hash,
                })?;
            }
            Ok(FileCatalogState::Compact(records))
        } else {
            Ok(FileCatalogState::Native(seeds.clone()))
        }
    };
    let mutation: Mutation<'_, FileCatalogState> = (
        "subset_updates",
        Box::new(|state| {
            match state {
                FileCatalogState::Native(records) => {
                    for index in (0..records.len()).step_by(17) {
                        records[index].size = records[index].size.wrapping_add(4_096);
                        records[index].modified = records[index].modified.wrapping_add(1);
                    }
                }
                FileCatalogState::Compact(records) => {
                    for index in (0..records.len()).step_by(17) {
                        records[index].size = records[index].size.wrapping_add(4_096);
                        records[index].modified = records[index].modified.wrapping_add(1);
                    }
                }
            }
            Ok(0)
        }),
    );
    let queries: Read<'_, FileCatalogState> = (
        "path_queries_and_category_scan",
        Box::new(|state| {
            let mut checksum = 0_u64;
            match state {
                FileCatalogState::Native(records) => {
                    for index in (0..records.len()).step_by(23) {
                        if records[index].path == seeds[index].path {
                            checksum = checksum.wrapping_add(records[index].modified);
                        }
                    }
                    for record in records {
                        if record.category == "config" || record.category == "source" {
                            checksum = checksum
                                .wrapping_add(record.size)
                                .wrapping_add(record.hash.unwrap_or(0))
                                .wrapping_add(u64::from(record.file_type));
                        }
                    }
                }
                FileCatalogState::Compact(records) => {
                    for index in (0..records.len()).step_by(23) {
                        if records[index].path.to_path_buf() == seeds[index].path {
                            checksum = checksum.wrapping_add(records[index].modified);
                        }
                    }
                    for record in records {
                        if record.category.as_str() == "config"
                            || record.category.as_str() == "source"
                        {
                            checksum = checksum
                                .wrapping_add(record.size)
                                .wrapping_add(record.hash.unwrap_or(0))
                                .wrapping_add(u64::from(record.file_type));
                        }
                    }
                }
            }
            Ok(checksum)
        }),
    );
    measure::run_case(
        "B7",
        variant,
        repetitions,
        compact,
        build,
        vec![mutation],
        vec![queries],
    )
}

enum CacheState {
    Native(HashMap<u64, NativeCacheEntry>),
    Compact(CompactHashMap<u64, CompactCacheEntry>),
}

fn cache_churn(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let seeds = datasets::cache_seeds();
    let compact = variant == "compact";
    let build = || -> BenchResult<CacheState> {
        if compact {
            let mut entries = CompactHashMap::with_capacity(datasets::CACHE_POPULATION)?;
            for (index, seed) in seeds.iter().take(datasets::CACHE_POPULATION).enumerate() {
                entries.insert(
                    index as u64,
                    CompactCacheEntry {
                        payload: CompactBytes::from_slice(&seed.payload)?,
                        expiry: seed.expiry,
                        version: seed.version,
                    },
                )?;
            }
            Ok(CacheState::Compact(entries))
        } else {
            let mut entries = HashMap::with_capacity(datasets::CACHE_POPULATION);
            for (index, seed) in seeds.iter().take(datasets::CACHE_POPULATION).enumerate() {
                entries.insert(
                    index as u64,
                    NativeCacheEntry {
                        payload: seed.payload.clone(),
                        expiry: seed.expiry,
                        version: seed.version,
                    },
                );
            }
            Ok(CacheState::Native(entries))
        }
    };
    let mutation: Mutation<'_, CacheState> = (
        "fixed_population_churn",
        Box::new(|state| {
            match state {
                CacheState::Native(entries) => mutate_native_cache(entries, &seeds),
                CacheState::Compact(entries) => mutate_compact_cache(entries, &seeds)?,
            }
            Ok(0)
        }),
    );
    let reads: Read<'_, CacheState> = (
        "cache_lookup_and_scan",
        Box::new(|state| {
            let mut checksum = 0_u64;
            match state {
                CacheState::Native(entries) => {
                    for key in (0..datasets::CACHE_POPULATION as u64 * 5).step_by(31) {
                        if let Some(entry) = entries.get(&key) {
                            checksum = checksum
                                .wrapping_add(key)
                                .wrapping_add(entry.expiry)
                                .wrapping_add(u64::from(entry.version))
                                .wrapping_add(entry.payload.len() as u64);
                        }
                    }
                    checksum = checksum.wrapping_add(
                        entries
                            .values()
                            .map(|entry| entry.payload.len() as u64)
                            .sum::<u64>(),
                    );
                }
                CacheState::Compact(entries) => {
                    for key in (0..datasets::CACHE_POPULATION as u64 * 5).step_by(31) {
                        if let Some(entry) = entries.get(&key) {
                            checksum = checksum
                                .wrapping_add(key)
                                .wrapping_add(entry.expiry)
                                .wrapping_add(u64::from(entry.version))
                                .wrapping_add(entry.payload.len() as u64);
                        }
                    }
                    checksum = checksum.wrapping_add(
                        entries
                            .values()
                            .map(|entry| entry.payload.len() as u64)
                            .sum::<u64>(),
                    );
                }
            }
            Ok(checksum)
        }),
    );
    let checksum = measure::run_case(
        "B8",
        variant,
        repetitions,
        compact,
        build,
        vec![mutation],
        vec![reads],
    )?;
    if compact {
        diagnose_cache_fragmentation(&seeds)?;
    }
    Ok(checksum)
}

fn mutate_native_cache(
    entries: &mut HashMap<u64, NativeCacheEntry>,
    seeds: &[datasets::CacheSeed],
) {
    let batch = datasets::CACHE_POPULATION / 8;
    for cycle in 0..datasets::CACHE_CYCLES {
        for index in 0..batch {
            let old_id = (cycle * batch + index) as u64;
            entries.remove(&old_id);
            let seed_index = datasets::CACHE_POPULATION + cycle * batch + index;
            let seed = &seeds[seed_index];
            let new_id = seed_index as u64;
            entries.insert(
                new_id,
                NativeCacheEntry {
                    payload: seed.payload.clone(),
                    expiry: seed.expiry,
                    version: seed.version,
                },
            );
        }
        debug_assert_eq!(entries.len(), datasets::CACHE_POPULATION);
    }
}

fn mutate_compact_cache(
    entries: &mut CompactHashMap<u64, CompactCacheEntry>,
    seeds: &[datasets::CacheSeed],
) -> BenchResult<()> {
    CompactRuntime::with_batched_releases(|| mutate_compact_cache_batch(entries, seeds))
}

fn mutate_compact_cache_batch(
    entries: &mut CompactHashMap<u64, CompactCacheEntry>,
    seeds: &[datasets::CacheSeed],
) -> BenchResult<()> {
    let batch = datasets::CACHE_POPULATION / 8;
    for cycle in 0..datasets::CACHE_CYCLES {
        for index in 0..batch {
            let old_id = (cycle * batch + index) as u64;
            entries.remove(&old_id);
            let seed_index = datasets::CACHE_POPULATION + cycle * batch + index;
            let seed = &seeds[seed_index];
            entries.insert(
                seed_index as u64,
                CompactCacheEntry {
                    payload: CompactBytes::from_slice(&seed.payload)?,
                    expiry: seed.expiry,
                    version: seed.version,
                },
            )?;
        }
        debug_assert_eq!(entries.len(), datasets::CACHE_POPULATION);
    }
    Ok(())
}

fn diagnose_cache_fragmentation(seeds: &[datasets::CacheSeed]) -> BenchResult<()> {
    let mut entries = CompactHashMap::with_capacity(datasets::CACHE_POPULATION)?;
    for (index, seed) in seeds.iter().take(datasets::CACHE_POPULATION).enumerate() {
        entries.insert(
            index as u64,
            CompactCacheEntry {
                payload: CompactBytes::from_slice(&seed.payload)?,
                expiry: seed.expiry,
                version: seed.version,
            },
        )?;
    }
    let batch = datasets::CACHE_POPULATION / 8;
    for cycle in 0..datasets::CACHE_CYCLES {
        for index in 0..batch {
            let old_id = (cycle * batch + index) as u64;
            entries.remove(&old_id);
            let seed_index = datasets::CACHE_POPULATION + cycle * batch + index;
            let seed = &seeds[seed_index];
            entries.insert(
                seed_index as u64,
                CompactCacheEntry {
                    payload: CompactBytes::from_slice(&seed.payload)?,
                    expiry: seed.expiry,
                    version: seed.version,
                },
            )?;
        }
        CompactRuntime::validate_allocator_state()?;
        let stats = CompactRuntime::allocator_stats()?;
        println!(
            "META\tcache_cycle\tB8\tcompact\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            cycle + 1,
            entries.len(),
            entries.capacity(),
            stats.high_water_cursor,
            stats.free_bytes,
            stats.free_blocks,
            stats.largest_free_block
        );
    }
    drop(entries);
    CompactRuntime::validate_allocator_state()?;
    Ok(())
}

enum FrozenState {
    Native {
        title: String,
        records: Vec<NativeCatalogRecord>,
    },
    Compact(FrozenGraph<FrozenCatalogRoot>),
}

fn frozen_catalog(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let seeds = datasets::frozen_catalog();
    let compact = variant == "compact";
    let build = || -> BenchResult<FrozenState> {
        if compact {
            let mut builder = FrozenBuilder::new()?;
            let title = builder.store_str("dispatch immutable catalog")?;
            let mut descriptors = Vec::with_capacity(seeds.len());
            for record in &seeds {
                descriptors.push(FrozenCatalogRecord {
                    id: record.id,
                    name: builder.store_str(&record.name)?,
                    category: builder.store_str(&record.category)?,
                    related: builder.store_slice(&record.related)?,
                    flags: record.flags,
                });
            }
            let records = builder.store_slice(&descriptors)?;
            Ok(FrozenState::Compact(
                builder.finish(FrozenCatalogRoot { title, records })?,
            ))
        } else {
            Ok(FrozenState::Native {
                title: "dispatch immutable catalog".to_owned(),
                records: seeds.clone(),
            })
        }
    };
    let sequential: Read<'_, FrozenState> = (
        "sequential_traversal",
        Box::new(|state| match state {
            FrozenState::Native { title, records } => Ok(native_frozen_checksum(title, records)),
            FrozenState::Compact(graph) => Ok(compact_frozen_checksum(graph)),
        }),
    );
    let random: Read<'_, FrozenState> = (
        "deterministic_random_lookup",
        Box::new(|state| {
            let mut checksum = 0_u64;
            match state {
                FrozenState::Native { records, .. } => {
                    for offset in 0..8_000 {
                        let index = (offset * 7_919) % records.len();
                        checksum = checksum.wrapping_add(native_catalog_record(&records[index]));
                    }
                }
                FrozenState::Compact(graph) => {
                    let view = graph.view();
                    let records = view.slice(&graph.root().records)?;
                    for offset in 0..8_000 {
                        let index = (offset * 7_919) % records.len();
                        checksum =
                            checksum.wrapping_add(compact_catalog_record(&view, &records[index]));
                    }
                }
            }
            Ok(checksum)
        }),
    );
    let parallel: Read<'_, FrozenState> = (
        "parallel_read_traversal",
        Box::new(|state| match state {
            FrozenState::Native { records, .. } => Ok(native_parallel_checksum(records)),
            FrozenState::Compact(graph) => Ok(compact_parallel_checksum(graph)),
        }),
    );
    measure::run_case(
        "B9",
        variant,
        repetitions,
        compact,
        build,
        vec![],
        vec![sequential, random, parallel],
    )
}

fn native_catalog_record(record: &NativeCatalogRecord) -> u64 {
    record.id as u64
        + record.name.len() as u64
        + record.category.len() as u64
        + record
            .related
            .iter()
            .map(|value| u64::from(*value))
            .sum::<u64>()
        + u64::from(record.flags)
}

fn native_frozen_checksum(title: &str, records: &[NativeCatalogRecord]) -> u64 {
    records
        .iter()
        .map(native_catalog_record)
        .sum::<u64>()
        .wrapping_add(title.len() as u64)
}

fn compact_catalog_record(view: &FrozenGraphView<'_>, record: &FrozenCatalogRecord) -> u64 {
    let name = view
        .str(&record.name)
        .expect("catalog name descriptor is valid");
    let category = view
        .str(&record.category)
        .expect("catalog category descriptor is valid");
    let related = view
        .slice(&record.related)
        .expect("catalog related descriptor is valid");
    record.id as u64
        + name.len() as u64
        + category.len() as u64
        + related.iter().map(|value| u64::from(*value)).sum::<u64>()
        + u64::from(record.flags)
}

fn compact_frozen_checksum(graph: &FrozenGraph<FrozenCatalogRoot>) -> u64 {
    let view = graph.view();
    let root = graph.root();
    let title = view
        .str(&root.title)
        .expect("catalog title descriptor is valid");
    let records = view
        .slice(&root.records)
        .expect("catalog records descriptor is valid");
    records
        .iter()
        .map(|record| compact_catalog_record(&view, record))
        .sum::<u64>()
        .wrapping_add(title.len() as u64)
}

fn native_parallel_checksum(records: &[NativeCatalogRecord]) -> u64 {
    let mid = records.len() / 2;
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            records[..mid]
                .iter()
                .map(native_catalog_record)
                .sum::<u64>()
        });
        let second = scope.spawn(|| {
            records[mid..]
                .iter()
                .map(native_catalog_record)
                .sum::<u64>()
        });
        first.join().expect("catalog read worker") + second.join().expect("catalog read worker")
    })
}

fn compact_parallel_checksum(graph: &FrozenGraph<FrozenCatalogRoot>) -> u64 {
    let view = graph.view();
    let records = view
        .slice(&graph.root().records)
        .expect("catalog records descriptor is valid");
    let mid = records.len() / 2;
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            records[..mid]
                .iter()
                .map(|record| compact_catalog_record(&view, record))
                .sum::<u64>()
        });
        let second = scope.spawn(|| {
            records[mid..]
                .iter()
                .map(|record| compact_catalog_record(&view, record))
                .sum::<u64>()
        });
        first.join().expect("catalog read worker") + second.join().expect("catalog read worker")
    })
}

enum WorkerState {
    Native(Vec<Vec<WorkerRecord>>),
    Compact(Vec<CompactVec<WorkerRecord>>),
}

fn concurrent_workers(variant: &str, repetitions: usize) -> BenchResult<u64> {
    let seeds = datasets::worker_records();
    let compact = variant == "compact";
    let build = || -> BenchResult<WorkerState> {
        if compact {
            std::thread::scope(|scope| -> BenchResult<Vec<CompactVec<WorkerRecord>>> {
                let mut handles = Vec::with_capacity(datasets::WORKERS);
                for records in &seeds {
                    handles.push(scope.spawn(
                        move || -> compact_std::Result<CompactVec<WorkerRecord>> {
                            let mut values = CompactVec::with_capacity(records.len())?;
                            values.try_extend(records.iter().copied())?;
                            Ok(values)
                        },
                    ));
                }
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .map_err(|_| "compact build worker panicked")?
                            .map_err(|error| -> Box<dyn Error> { Box::new(error) })
                    })
                    .collect()
            })
            .map(WorkerState::Compact)
        } else {
            let workers = std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(datasets::WORKERS);
                for records in &seeds {
                    handles.push(scope.spawn(move || records.clone()));
                }
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("native build worker"))
                    .collect()
            });
            Ok(WorkerState::Native(workers))
        }
    };
    let mutation: Mutation<'_, WorkerState> = (
        "parallel_allocate_drop_churn",
        Box::new(|state| match state {
            WorkerState::Native(workers) => Ok(native_worker_churn(workers)),
            WorkerState::Compact(workers) => compact_worker_churn(workers),
        }),
    );
    let reads: Read<'_, WorkerState> = (
        "parallel_traversal",
        Box::new(|state| match state {
            WorkerState::Native(workers) => Ok(native_worker_parallel_read(workers)),
            WorkerState::Compact(workers) => Ok(compact_worker_parallel_read(workers)),
        }),
    );
    measure::run_case(
        "B10",
        variant,
        repetitions,
        compact,
        build,
        vec![mutation],
        vec![reads],
    )
}

fn native_worker_churn(workers: &mut [Vec<WorkerRecord>]) -> u64 {
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers.len());
        for (worker, records) in workers.iter_mut().enumerate() {
            handles.push(scope.spawn(move || {
                let mut checksum = 0_u64;
                for index in 0..4_000_u32 {
                    let record = WorkerRecord {
                        id: worker as u32 * 100_000 + index,
                        value: u64::from(index) * 31,
                        group: (index % 256) as u16,
                        active: index % 5 != 0,
                    };
                    let boxed = Box::new(record);
                    checksum = checksum.wrapping_add(u64::from(boxed.id) + boxed.value);
                    black_box(boxed);
                }
                records
                    .iter()
                    .map(|record| u64::from(record.id) + record.value)
                    .sum::<u64>()
                    ^ checksum
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("native churn worker"))
            .sum()
    })
}

fn compact_worker_churn(workers: &mut [CompactVec<WorkerRecord>]) -> BenchResult<u64> {
    std::thread::scope(|scope| -> compact_std::Result<u64> {
        let mut handles = Vec::with_capacity(workers.len());
        for (worker, records) in workers.iter_mut().enumerate() {
            handles.push(scope.spawn(move || -> compact_std::Result<u64> {
                let mut checksum = 0_u64;
                for index in 0..4_000_u32 {
                    let record = WorkerRecord {
                        id: worker as u32 * 100_000 + index,
                        value: u64::from(index) * 31,
                        group: (index % 256) as u16,
                        active: index % 5 != 0,
                    };
                    let boxed = CompactBox::new(record)?;
                    checksum = checksum.wrapping_add(u64::from(boxed.id) + boxed.value);
                    black_box(boxed);
                }
                Ok(records
                    .iter()
                    .map(|record| u64::from(record.id) + record.value)
                    .sum::<u64>()
                    ^ checksum)
            }));
        }
        handles.into_iter().try_fold(0_u64, |sum, handle| {
            Ok(sum.wrapping_add(handle.join().expect("compact churn worker")?))
        })
    })
    .map_err(|error| Box::new(error) as Box<dyn Error>)
}

fn native_worker_parallel_read(workers: &[Vec<WorkerRecord>]) -> u64 {
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers.len());
        for records in workers {
            handles.push(scope.spawn(move || {
                records
                    .iter()
                    .filter(|record| record.active)
                    .map(|record| u64::from(record.id) + record.value + u64::from(record.group))
                    .sum::<u64>()
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("native read worker"))
            .sum()
    })
}

fn compact_worker_parallel_read(workers: &[CompactVec<WorkerRecord>]) -> u64 {
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers.len());
        for records in workers {
            handles.push(scope.spawn(move || {
                records
                    .iter()
                    .filter(|record| record.active)
                    .map(|record| u64::from(record.id) + record.value + u64::from(record.group))
                    .sum::<u64>()
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("compact read worker"))
            .sum()
    })
}
