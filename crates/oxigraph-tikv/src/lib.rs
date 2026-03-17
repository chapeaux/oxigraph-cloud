//! TiKV storage backend for Oxigraph.
//!
//! This crate implements the storage backend trait for TiKV,
//! enabling distributed, cloud-native operation of Oxigraph
//! with Raft consensus and MVCC transactions.

pub mod backend;

pub use backend::TiKvConfig;
