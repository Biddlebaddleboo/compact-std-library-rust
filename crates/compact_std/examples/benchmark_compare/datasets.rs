use crate::models::{NativeCatalogRecord, NativeFileRecord, NativeOffer, PriceLevel, WorkerRecord};
use std::fmt::Write as _;
use std::path::PathBuf;

pub const API_RECORDS: usize = 10_000;
pub const REQUEST_RECORDS: usize = 24_000;
pub const EVENT_HISTORY: usize = 4_096;
pub const EVENT_CHURN: usize = 40_000;
pub const DISPATCH_OFFERS: usize = 24_000;
pub const FILE_RECORDS: usize = 100_000;
pub const CACHE_POPULATION: usize = 8_192;
pub const CACHE_CYCLES: usize = 64;
pub const FROZEN_RECORDS: usize = 40_000;
pub const WORKERS: usize = 2;
pub const WORKER_RECORDS: usize = 8_000;

pub struct RequestSeed {
    pub method: String,
    pub path: String,
    pub host: String,
    pub status: u16,
    pub content_length: u64,
    pub headers: Vec<(String, String)>,
    pub request_id: String,
    pub trace_id: String,
    pub tags: Vec<String>,
}

pub struct EventSeed {
    pub timestamp: u64,
    pub severity: u8,
    pub component: String,
    pub message: String,
    pub context: [u32; 3],
    pub request_id: Option<String>,
}

pub struct CacheSeed {
    pub payload: Vec<u8>,
    pub expiry: u64,
    pub version: u32,
}

pub fn json_api_payload() -> String {
    let mut json = String::with_capacity(API_RECORDS * 190 + 32);
    json.push_str("{\"records\":[");
    for id in 0..API_RECORDS {
        if id != 0 {
            json.push(',');
        }
        let enabled = id % 3 != 0;
        let note = if id % 7 == 0 {
            "null".to_owned()
        } else {
            format!("\"note-{id:05}\"")
        };
        let status = match id % 4 {
            0 => "ready",
            1 => "queued",
            2 => "paused",
            _ => "complete",
        };
        write!(
            json,
            "{{\"id\":{id},\"name\":\"client-{id:05}\",\"status\":\"{status}\",\"enabled\":{enabled},\"note\":{note},\"tags\":[\"api\",\"zone-{:02}\",\"tier-{}\"],\"metadata\":{{\"source\":\"edge-{}\",\"revision\":{},\"flags\":[1,{},{}]}},\"timestamp\":{}}}",
            id % 64,
            id % 5,
            id % 8,
            id % 97,
            id % 16,
            id % 32,
            1_700_000_000_u64 + id as u64 * 17,
        )
        .expect("writing into String cannot fail");
    }
    json.push_str("]}");
    json
}

pub fn toml_service_config() -> String {
    let mut toml = String::with_capacity(48_000);
    toml.push_str("service = 'dispatch-edge'\n");
    toml.push_str("endpoints = [\n");
    for index in 0..96 {
        writeln!(toml, "  'https://edge-{index:03}.example.invalid/api',").unwrap();
    }
    toml.push_str("]\npaths = [\n");
    for index in 0..80 {
        writeln!(toml, "  '/etc/dispatch/workers/worker-{index:03}.toml',").unwrap();
    }
    toml.push_str("]\nfeature_flags = [");
    for index in 0..64 {
        if index != 0 {
            toml.push_str(", ");
        }
        toml.push_str(if index % 3 == 0 { "true" } else { "false" });
    }
    toml.push_str("]\nretries = 5\ntimeout_ms = 1250\n\n[labels]\n");
    for index in 0..24 {
        writeln!(toml, "label_{index:02} = 'region-{}'", index % 8).unwrap();
    }
    for index in 0..48 {
        writeln!(
            toml,
            "\n[[workers]]\nname = 'worker-{index:03}'\nthreads = {}\nretries = {}\ntimeout_ms = {}",
            2 + index % 8,
            index % 6,
            500 + index * 25,
        )
        .unwrap();
    }
    for index in 0..160 {
        writeln!(
            toml,
            "\n[[routes]]\nprefix = '/v{}/resource/{index:03}'\ntarget = 'worker-{:03}'\nmethods = ['GET', '{}']",
            index % 4,
            index % 48,
            if index % 2 == 0 { "POST" } else { "PUT" },
        )
        .unwrap();
    }
    toml
}

pub fn string_byte_payloads() -> Vec<String> {
    const LENGTHS: [usize; 5] = [8, 24, 96, 1024, 65_536];
    let mut values = Vec::with_capacity(500);
    for index in 0..500 {
        let len = LENGTHS[index % LENGTHS.len()];
        let letter = b'a' + (index % 26) as u8;
        values.push(String::from_utf8(vec![letter; len]).expect("ASCII is UTF-8"));
    }
    values
}

pub fn path_strings(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| {
            format!(
                "/srv/dispatch/region-{}/worker-{}/config-{}.toml",
                index % 24,
                index % 512,
                index % 7
            )
        })
        .collect()
}

pub fn requests() -> Vec<RequestSeed> {
    (0..REQUEST_RECORDS)
        .map(|index| RequestSeed {
            method: match index % 4 {
                0 => "GET",
                1 => "POST",
                2 => "PATCH",
                _ => "DELETE",
            }
            .to_owned(),
            path: format!("/api/v2/users/{}/items/{index}", index % 3_000),
            host: format!("api-{}.example.invalid", index % 16),
            status: [200, 201, 401, 404, 503][index % 5],
            content_length: (index as u64 * 7919) % 65_536,
            headers: vec![
                ("accept".to_owned(), "application/json".to_owned()),
                ("x-region".to_owned(), format!("r{}", index % 24)),
                ("cache-control".to_owned(), "private, max-age=0".to_owned()),
            ],
            request_id: format!("req-{index:08x}"),
            trace_id: format!("trace-{:016x}", index as u64 * 0x9e37_79b9),
            tags: vec![
                format!("service-{}", index % 12),
                format!("zone-{}", index % 64),
            ],
        })
        .collect()
}

pub fn events() -> Vec<EventSeed> {
    (0..EVENT_HISTORY)
        .map(|index| event_seed(index as u64))
        .collect()
}

pub fn event_seed(sequence: u64) -> EventSeed {
    EventSeed {
        timestamp: 1_800_000_000 + sequence * 3,
        severity: (sequence % 5) as u8,
        component: format!("worker-{:02}", sequence % 32),
        message: format!("job {} moved to queue {}", sequence % 1_000, sequence % 128),
        context: [
            sequence as u32,
            (sequence % 127) as u32,
            (sequence % 31) as u32,
        ],
        request_id: (sequence % 3 != 0).then(|| format!("req-{sequence:08x}")),
    }
}

pub fn dispatch_offers() -> Vec<NativeOffer> {
    (0..DISPATCH_OFFERS)
        .map(|id| NativeOffer {
            id: id as u64,
            pickup_zone: (id % 384) as u32,
            dropoff_zone: ((id * 7 + 19) % 384) as u32,
            latitude_e6: 37_000_000 + (id % 90_000) as i32,
            longitude_e6: -122_000_000 + (id % 80_000) as i32,
            distance_m: 700 + (id * 37 % 48_000) as u32,
            eta_s: 90 + (id * 11 % 3_600) as u32,
            provider: format!("p{:04}", id % 32),
            status: (id % 4) as u8,
            address: format!("Mkt-{id:08}"),
            rider: (id % 5 != 0).then(|| format!("r{:010}", id % 90_000)),
            route_tags: [(id % 97) as u32, ((id * 3) % 211) as u32],
        })
        .collect()
}

pub fn order_book_levels() -> Vec<PriceLevel> {
    (0..2_048)
        .map(|index| PriceLevel {
            price_micros: 50_000_000 + index as u64 * 10_000,
            quantity: 100 + (index as u64 * 53 % 50_000),
            order_count: 1 + (index as u32 % 32),
            flags: (index as u32) & 3,
        })
        .collect()
}

pub fn file_records() -> Vec<NativeFileRecord> {
    (0..FILE_RECORDS)
        .map(|index| {
            let category = match index % 6 {
                0 => "source",
                1 => "config",
                2 => "log",
                3 => "image",
                4 => "database",
                _ => "archive",
            };
            NativeFileRecord {
                path: PathBuf::from(format!(
                    "/srv/catalog/tenant-{}/bucket-{}/object-{index:06}.{}",
                    index % 128,
                    index % 512,
                    match index % 6 {
                        0 => "rs",
                        1 => "toml",
                        2 => "log",
                        3 => "png",
                        4 => "db",
                        _ => "tar",
                    }
                )),
                size: 512 + (index as u64 * 7_919 % 16_777_216),
                modified: 1_700_000_000 + index as u64 * 13,
                file_type: (index % 6) as u8,
                category: category.to_owned(),
                hash: (index % 4 != 0).then_some(index as u64 * 0x9e37_79b9),
            }
        })
        .collect()
}

pub fn cache_seeds() -> Vec<CacheSeed> {
    (0..CACHE_POPULATION + CACHE_CYCLES * (CACHE_POPULATION / 8))
        .map(|index| {
            let sizes = [8, 24, 96, 512, 1_024];
            let size = sizes[index % sizes.len()];
            CacheSeed {
                payload: vec![(index % 251) as u8; size],
                expiry: 1_900_000_000 + index as u64 * 60,
                version: (index % 17) as u32,
            }
        })
        .collect()
}

pub fn frozen_catalog() -> Vec<NativeCatalogRecord> {
    (0..FROZEN_RECORDS)
        .map(|id| NativeCatalogRecord {
            id: id as u32,
            name: format!("catalog-item-{id:06}"),
            category: format!("category-{}", id % 64),
            related: (0..5)
                .map(|offset| ((id * 17 + offset * 1_009) % FROZEN_RECORDS) as u32)
                .collect(),
            flags: (id as u32).rotate_left(5) & 0x3f,
        })
        .collect()
}

pub fn worker_records() -> Vec<Vec<WorkerRecord>> {
    (0..WORKERS)
        .map(|worker| {
            (0..WORKER_RECORDS)
                .map(|index| WorkerRecord {
                    id: (worker * WORKER_RECORDS + index) as u32,
                    value: (index as u64 * 7_919) ^ (worker as u64 * 0x9e37),
                    group: (index % 256) as u16,
                    active: index % 7 != 0,
                })
                .collect()
        })
        .collect()
}
