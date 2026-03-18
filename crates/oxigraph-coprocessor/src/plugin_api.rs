// Vendored types from TiKV's coprocessor_plugin_api crate.
// Original source: https://github.com/tikv/tikv/tree/master/components/coprocessor_plugin_api
// Licensed under Apache-2.0 by TiKV Authors.
//
// These types are vendored here because the `coprocessor_plugin_api` crate is
// not published on crates.io (publish = false). We reproduce the minimal
// surface needed for our plugin.
//
// NOTE: The upstream `RawStorage` trait uses `async fn` (via `async_trait`),
// but since `CoprocessorPlugin::on_raw_coprocessor_request` is synchronous and
// Region-local data access does not actually cross the network, we provide a
// synchronous version here. When integrating with the real TiKV runtime, a
// thin adapter can bridge the async interface to these synchronous methods.

use std::any::Any;
use std::ops::Range;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Core type aliases
// ---------------------------------------------------------------------------

pub type Key = Vec<u8>;
pub type Value = Vec<u8>;
pub type KvPair = (Key, Value);
pub type RawRequest = Vec<u8>;
pub type RawResponse = Vec<u8>;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

pub type PluginResult<T> = Result<T, PluginError>;

#[derive(Debug)]
pub enum PluginError {
    KeyNotInRegion {
        key: Key,
        region_id: u64,
        start_key: Key,
        end_key: Key,
    },
    Timeout(Duration),
    Canceled,
    Other(String, Box<dyn Any>),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KeyNotInRegion {
                key,
                region_id,
                start_key,
                end_key,
            } => write!(
                f,
                "key {key:?} not in region {region_id} [{start_key:?}, {end_key:?})"
            ),
            Self::Timeout(d) => write!(f, "timeout after {d:?}"),
            Self::Canceled => write!(f, "canceled"),
            Self::Other(msg, _) => write!(f, "{msg}"),
        }
    }
}

// ---------------------------------------------------------------------------
// RawStorage trait
// ---------------------------------------------------------------------------

/// Storage interface provided by TiKV to coprocessor plugins.
///
/// In the real TiKV runtime this is backed by the Raft engine; for testing
/// we provide in-memory implementations. The upstream API is async, but since
/// `CoprocessorPlugin::on_raw_coprocessor_request` is synchronous and all data
/// access is Region-local, we expose synchronous methods here.
pub trait RawStorage {
    fn get(&self, key: Key) -> PluginResult<Option<Value>>;
    fn batch_get(&self, keys: Vec<Key>) -> PluginResult<Vec<KvPair>>;
    fn scan(&self, key_range: Range<Key>) -> PluginResult<Vec<KvPair>>;
    fn put(&self, key: Key, value: Value) -> PluginResult<()>;
    fn batch_put(&self, kv_pairs: Vec<KvPair>) -> PluginResult<()>;
    fn delete(&self, key: Key) -> PluginResult<()>;
    fn batch_delete(&self, keys: Vec<Key>) -> PluginResult<()>;
    fn delete_range(&self, key_range: Range<Key>) -> PluginResult<()>;
}

// ---------------------------------------------------------------------------
// CoprocessorPlugin trait
// ---------------------------------------------------------------------------

/// Trait that all TiKV coprocessor plugins must implement.
pub trait CoprocessorPlugin: Send + Sync {
    fn on_raw_coprocessor_request(
        &self,
        ranges: Vec<Range<Key>>,
        request: RawRequest,
        storage: &dyn RawStorage,
    ) -> PluginResult<RawResponse>;
}

// ---------------------------------------------------------------------------
// Plugin registration macro
// ---------------------------------------------------------------------------

/// Declares a plugin constructor that TiKV calls via FFI when loading the
/// shared library.
#[macro_export]
macro_rules! declare_plugin {
    ($plugin_constructor:expr) => {
        /// # Safety
        ///
        /// Called by TiKV's plugin loader via FFI. The returned pointer must be
        /// freed by the caller using `Box::from_raw`.
        #[unsafe(no_mangle)]
        #[allow(unsafe_code, improper_ctypes_definitions)]
        pub extern "C" fn _tikv_coprocessor_plugin_create()
            -> *mut dyn $crate::plugin_api::CoprocessorPlugin
        {
            let plugin = $plugin_constructor;
            let boxed: Box<dyn $crate::plugin_api::CoprocessorPlugin> = Box::new(plugin);
            Box::into_raw(boxed)
        }
    };
}
