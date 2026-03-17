//! TiKV backend implementation.
//!
//! Uses `tikv-client` with a sync wrapper (`block_on`) per ADR-003.
//! Extends Oxigraph's `StorageKind` enum per ADR-002.

/// Configuration for connecting to a TiKV cluster.
#[derive(Clone, Debug)]
pub struct TiKvConfig {
    /// PD (Placement Driver) endpoints, e.g. `["127.0.0.1:2379"]`
    pub pd_endpoints: Vec<String>,
}

impl TiKvConfig {
    pub fn new(pd_endpoints: Vec<String>) -> Self {
        Self { pd_endpoints }
    }
}
