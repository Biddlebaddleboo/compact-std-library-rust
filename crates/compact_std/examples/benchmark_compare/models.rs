use compact_std::{
    CompactBytes, CompactHashMap, CompactPathBuf, CompactString, CompactValue, CompactVec,
    FrozenString, FrozenVec,
};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(serde::Deserialize)]
pub struct NativeApiResponse {
    pub records: Vec<NativeApiRecord>,
}

#[derive(serde::Deserialize)]
pub struct NativeApiRecord {
    pub id: u32,
    pub name: String,
    pub status: String,
    pub enabled: bool,
    pub note: Option<String>,
    pub tags: Vec<String>,
    pub metadata: NativeApiMetadata,
    pub timestamp: u64,
}

#[derive(serde::Deserialize)]
pub struct NativeApiMetadata {
    pub source: String,
    pub revision: u32,
    pub flags: Vec<u32>,
}

#[derive(compact_std::CompactDeserialize)]
pub struct CompactApiResponse {
    pub records: CompactVec<CompactApiRecord>,
}

#[derive(compact_std::CompactDeserialize)]
pub struct CompactApiRecord {
    pub id: u32,
    pub name: CompactString,
    pub status: CompactString,
    pub enabled: bool,
    pub note: Option<CompactString>,
    pub tags: CompactVec<CompactString>,
    pub metadata: CompactApiMetadata,
    pub timestamp: u64,
}

#[derive(compact_std::CompactDeserialize)]
pub struct CompactApiMetadata {
    pub source: CompactString,
    pub revision: u32,
    pub flags: CompactVec<u32>,
}

// SAFETY: these models retain compact owners and scalar fields only.
unsafe impl CompactValue for CompactApiRecord {}
unsafe impl CompactValue for CompactApiMetadata {}

#[derive(serde::Deserialize)]
pub struct NativeServiceConfig {
    pub service: String,
    pub endpoints: Vec<String>,
    pub paths: Vec<PathBuf>,
    pub labels: HashMap<String, String>,
    pub feature_flags: Vec<bool>,
    pub retries: u32,
    pub timeout_ms: u64,
    pub workers: Vec<NativeWorkerConfig>,
    pub routes: Vec<NativeRouteConfig>,
}

#[derive(serde::Deserialize)]
pub struct NativeWorkerConfig {
    pub name: String,
    pub threads: u32,
    pub retries: u32,
    pub timeout_ms: u64,
}

#[derive(serde::Deserialize)]
pub struct NativeRouteConfig {
    pub prefix: String,
    pub target: String,
    pub methods: Vec<String>,
}

#[derive(compact_std::CompactDeserialize)]
pub struct CompactServiceConfig {
    pub service: CompactString,
    pub endpoints: CompactVec<CompactString>,
    pub paths: CompactVec<CompactPathBuf>,
    pub labels: CompactHashMap<CompactString, CompactString>,
    pub feature_flags: CompactVec<bool>,
    pub retries: u32,
    pub timeout_ms: u64,
    pub workers: CompactVec<CompactWorkerConfig>,
    pub routes: CompactVec<CompactRouteConfig>,
}

#[derive(compact_std::CompactDeserialize)]
pub struct CompactWorkerConfig {
    pub name: CompactString,
    pub threads: u32,
    pub retries: u32,
    pub timeout_ms: u64,
}

#[derive(compact_std::CompactDeserialize)]
pub struct CompactRouteConfig {
    pub prefix: CompactString,
    pub target: CompactString,
    pub methods: CompactVec<CompactString>,
}

// SAFETY: these models retain compact owners and scalar fields only.
unsafe impl CompactValue for CompactWorkerConfig {}
unsafe impl CompactValue for CompactRouteConfig {}

#[derive(Clone)]
pub struct NativeHeader {
    pub name: String,
    pub value: String,
}

pub struct NativeRequestRecord {
    pub method: String,
    pub path: String,
    pub host: String,
    pub status: u16,
    pub content_length: u64,
    pub headers: Vec<NativeHeader>,
    pub request_id: String,
    pub trace_id: String,
    pub tags: Vec<String>,
}

pub struct CompactRequestRecord {
    pub method: CompactString,
    pub path: CompactString,
    pub host: CompactString,
    pub status: u16,
    pub content_length: u64,
    pub headers: CompactVec<(CompactString, CompactString)>,
    pub request_id: CompactString,
    pub trace_id: CompactString,
    pub tags: CompactVec<CompactString>,
}

// SAFETY: all retained strings and collections use compact cage owners.
unsafe impl CompactValue for CompactRequestRecord {}

pub struct NativeEvent {
    pub timestamp: u64,
    pub severity: u8,
    pub component: String,
    pub message: String,
    pub context: [u32; 3],
    pub request_id: Option<String>,
}

pub struct CompactEvent {
    pub timestamp: u64,
    pub severity: u8,
    pub component: CompactString,
    pub message: CompactString,
    pub context: [u32; 3],
    pub request_id: Option<CompactString>,
}

// SAFETY: all retained strings are compact owners; remaining fields are scalars.
unsafe impl CompactValue for CompactEvent {}

#[derive(Clone)]
pub struct NativeOffer {
    pub id: u64,
    pub pickup_zone: u32,
    pub dropoff_zone: u32,
    pub latitude_e6: i32,
    pub longitude_e6: i32,
    pub distance_m: u32,
    pub eta_s: u32,
    pub provider: String,
    pub status: u8,
    pub address: String,
    pub rider: Option<String>,
    pub route_tags: [u32; 2],
}

pub struct CompactOffer {
    pub id: u64,
    pub pickup_zone: u32,
    pub dropoff_zone: u32,
    pub latitude_e6: i32,
    pub longitude_e6: i32,
    pub distance_m: u32,
    pub eta_s: u32,
    pub provider: CompactString,
    pub status: u8,
    pub address: CompactString,
    pub rider: Option<CompactString>,
    pub route_tags: [u32; 2],
}

// SAFETY: retained strings and route tags are compact owners; other fields are scalars.
unsafe impl CompactValue for CompactOffer {}

#[derive(Clone, Copy)]
pub struct PriceLevel {
    pub price_micros: u64,
    pub quantity: u64,
    pub order_count: u32,
    pub flags: u32,
}

// SAFETY: this order-book level contains only copyable scalar fields.
unsafe impl CompactValue for PriceLevel {}

pub struct NativeOrderBook {
    pub symbol: String,
    pub venue: String,
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
}

pub struct CompactOrderBook {
    pub symbol: CompactString,
    pub venue: CompactString,
    pub bids: CompactVec<PriceLevel>,
    pub asks: CompactVec<PriceLevel>,
}

#[derive(Clone)]
pub struct NativeFileRecord {
    pub path: PathBuf,
    pub size: u64,
    pub modified: u64,
    pub file_type: u8,
    pub category: String,
    pub hash: Option<u64>,
}

pub struct CompactFileRecord {
    pub path: CompactPathBuf,
    pub size: u64,
    pub modified: u64,
    pub file_type: u8,
    pub category: CompactString,
    pub hash: Option<u64>,
}

// SAFETY: paths and category strings are compact owners; remaining fields are scalars.
unsafe impl CompactValue for CompactFileRecord {}

pub struct NativeCacheEntry {
    pub payload: Vec<u8>,
    pub expiry: u64,
    pub version: u32,
}

pub struct CompactCacheEntry {
    pub payload: CompactBytes,
    pub expiry: u64,
    pub version: u32,
}

// SAFETY: payload is an owning compact byte value and the remaining fields are scalars.
unsafe impl CompactValue for CompactCacheEntry {}

#[derive(Clone)]
pub struct NativeCatalogRecord {
    pub id: u32,
    pub name: String,
    pub category: String,
    pub related: Vec<u32>,
    pub flags: u32,
}

#[derive(Clone, Copy, compact_std::FrozenValue)]
pub struct FrozenCatalogRecord {
    pub id: u32,
    pub name: FrozenString,
    pub category: FrozenString,
    pub related: FrozenVec<u32>,
    pub flags: u32,
}

#[derive(Clone, Copy, compact_std::FrozenValue)]
pub struct FrozenCatalogRoot {
    pub title: FrozenString,
    pub records: FrozenVec<FrozenCatalogRecord>,
}

#[derive(Clone, Copy)]
pub struct WorkerRecord {
    pub id: u32,
    pub value: u64,
    pub group: u16,
    pub active: bool,
}

// SAFETY: this worker record contains only scalar values.
unsafe impl CompactValue for WorkerRecord {}
