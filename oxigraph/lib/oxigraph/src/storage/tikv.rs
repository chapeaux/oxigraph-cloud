//! TiKV storage backend for Oxigraph.
//!
//! Maps RocksDB column families to 1-byte key prefixes in TiKV's flat key space.
//! All async tikv-client calls are wrapped via `runtime.block_on()` per ADR-003.

use crate::model::{GraphNameRef, NamedOrBlankNodeRef, QuadRef, TermRef};
use crate::storage::binary_encoder::{
    QuadEncoding, WRITTEN_TERM_MAX_SIZE, decode_term, encode_term, encode_term_pair,
    encode_term_quad, encode_term_triple, write_gosp_quad, write_gpos_quad, write_gspo_quad,
    write_osp_quad, write_ospg_quad, write_pos_quad, write_posg_quad, write_spo_quad,
    write_spog_quad, write_term,
};
pub use crate::storage::error::{CorruptionError, StorageError};
use crate::storage::numeric_encoder::{EncodedQuad, EncodedTerm, StrHash, StrLookup, insert_term};
use oxrdf::Quad;
use std::cell::RefCell;
use std::sync::Arc;
use tikv_client::{BoundRange, Key, KvPair, TransactionClient};
use tokio::runtime::Runtime;

const LATEST_STORAGE_VERSION: u64 = 1;

/// Default number of entries to prefetch per batch during range scans.
const DEFAULT_SCAN_BATCH_SIZE: usize = 512;

// Table prefix bytes — map column families to prefix bytes
const TABLE_DEFAULT: u8 = 0x00;
const TABLE_ID2STR: u8 = 0x01;
const TABLE_SPOG: u8 = 0x02;
const TABLE_POSG: u8 = 0x03;
const TABLE_OSPG: u8 = 0x04;
const TABLE_GSPO: u8 = 0x05;
const TABLE_GPOS: u8 = 0x06;
const TABLE_GOSP: u8 = 0x07;
const TABLE_DSPO: u8 = 0x08;
const TABLE_DPOS: u8 = 0x09;
const TABLE_DOSP: u8 = 0x0A;
const TABLE_GRAPHS: u8 = 0x0B;

const BATCH_SIZE: usize = 100_000;

/// Configuration for connecting to TiKV.
#[derive(Clone, Debug)]
pub struct TiKvConfig {
    /// PD (Placement Driver) endpoint addresses.
    pub pd_endpoints: Vec<String>,
    /// Number of entries to prefetch per batch during range scans (default: 512).
    pub scan_batch_size: usize,
}

impl TiKvConfig {
    /// Create a new config with the given PD endpoints and default scan batch size.
    pub fn new(pd_endpoints: Vec<String>) -> Self {
        Self {
            pd_endpoints,
            scan_batch_size: DEFAULT_SCAN_BATCH_SIZE,
        }
    }

    /// Set the scan batch size for prefetching.
    pub fn with_scan_batch_size(mut self, batch_size: usize) -> Self {
        self.scan_batch_size = batch_size;
        self
    }
}

fn prefixed_key(table: u8, key: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(1 + key.len());
    result.push(table);
    result.extend_from_slice(key);
    result
}

/// Compute the exclusive upper bound for a prefix scan.
/// Increments the rightmost byte that is not 0xFF.
fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut upper = prefix.to_vec();
    while let Some(last) = upper.pop() {
        if last < 0xFF {
            upper.push(last + 1);
            return Some(upper);
        }
    }
    None
}

/// Build a `BoundRange` for scanning all keys with a given prefix.
fn prefix_range(prefix: &[u8]) -> BoundRange {
    if let Some(upper) = prefix_upper_bound(prefix) {
        let start: Key = prefix.to_vec().into();
        let end: Key = upper.into();
        BoundRange::from(start..end)
    } else {
        let start: Key = prefix.to_vec().into();
        BoundRange::from(start..)
    }
}

fn map_tikv_error(e: tikv_client::Error) -> StorageError {
    StorageError::Other(Box::new(e))
}

struct TiKvStorageInner {
    client: TransactionClient,
    runtime: Runtime,
    scan_batch_size: usize,
}

/// Low-level TiKV storage for Oxigraph.
#[derive(Clone)]
pub struct TiKvStorage {
    inner: Arc<TiKvStorageInner>,
}

impl TiKvStorage {
    /// Connect to TiKV via PD endpoints with default configuration.
    pub fn connect(pd_endpoints: &[String]) -> Result<Self, StorageError> {
        Self::connect_with_config(TiKvConfig::new(pd_endpoints.to_vec()))
    }

    /// Connect to TiKV with full configuration.
    pub fn connect_with_config(config: TiKvConfig) -> Result<Self, StorageError> {
        let runtime = Runtime::new().map_err(|e| StorageError::Other(Box::new(e)))?;
        let client = runtime
            .block_on(TransactionClient::new(config.pd_endpoints))
            .map_err(map_tikv_error)?;
        let storage = Self {
            inner: Arc::new(TiKvStorageInner {
                client,
                runtime,
                scan_batch_size: config.scan_batch_size,
            }),
        };
        storage.ensure_version()?;
        Ok(storage)
    }

    fn ensure_version(&self) -> Result<(), StorageError> {
        let version_key = prefixed_key(TABLE_DEFAULT, b"oxigraph_version");
        let mut txn = self
            .inner
            .runtime
            .block_on(self.inner.client.begin_optimistic())
            .map_err(map_tikv_error)?;
        let current = self
            .inner
            .runtime
            .block_on(txn.get(version_key.clone()))
            .map_err(map_tikv_error)?;
        match current {
            Some(v) => {
                if v.len() == 8 {
                    let version = u64::from_be_bytes(v[..8].try_into().unwrap_or([0; 8]));
                    if version != LATEST_STORAGE_VERSION {
                        return Err(StorageError::Other(
                            format!(
                                "Unsupported TiKV storage version {version}, expected {LATEST_STORAGE_VERSION}"
                            )
                            .into(),
                        ));
                    }
                }
                self.inner
                    .runtime
                    .block_on(txn.rollback())
                    .map_err(map_tikv_error)?;
            }
            None => {
                self.inner
                    .runtime
                    .block_on(txn.put(version_key, LATEST_STORAGE_VERSION.to_be_bytes().to_vec()))
                    .map_err(map_tikv_error)?;
                self.inner
                    .runtime
                    .block_on(txn.commit())
                    .map_err(map_tikv_error)?;
            }
        }
        Ok(())
    }

    pub fn snapshot(&self) -> TiKvStorageReader<'static> {
        let txn = self
            .inner
            .runtime
            .block_on(self.inner.client.begin_optimistic())
            .expect("failed to begin snapshot transaction");
        TiKvStorageReader {
            storage: self.clone(),
            txn: TiKvReaderTxn::Owned(RefCell::new(Some(txn))),
        }
    }

    pub fn start_transaction(&self) -> Result<TiKvStorageTransaction<'_>, StorageError> {
        let txn = self
            .inner
            .runtime
            .block_on(self.inner.client.begin_optimistic())
            .map_err(map_tikv_error)?;
        Ok(TiKvStorageTransaction {
            buffer: Vec::new(),
            txn: Some(txn),
            storage: self,
        })
    }

    pub fn start_readable_transaction(
        &self,
    ) -> Result<TiKvStorageReadableTransaction<'_>, StorageError> {
        let txn = self
            .inner
            .runtime
            .block_on(self.inner.client.begin_optimistic())
            .map_err(map_tikv_error)?;
        Ok(TiKvStorageReadableTransaction {
            buffer: Vec::new(),
            txn: Some(RefCell::new(txn)),
            storage: self,
        })
    }

    pub fn bulk_loader(&self) -> TiKvStorageBulkLoader<'_> {
        TiKvStorageBulkLoader {
            storage: self,
            hooks: Vec::new(),
            done: 0,
        }
    }

    fn runtime(&self) -> &Runtime {
        &self.inner.runtime
    }

    fn scan_batch_size(&self) -> usize {
        self.inner.scan_batch_size
    }
}

// --- Reader ---

/// Holds either an owned or borrowed transaction so that
/// `TiKvStorageReader` can be created from `snapshot()` (owned) or from
/// `TiKvStorageReadableTransaction::reader()` (borrowed).
enum TiKvReaderTxn<'a> {
    Owned(RefCell<Option<tikv_client::Transaction>>),
    Borrowed(&'a RefCell<tikv_client::Transaction>),
}

impl<'a> TiKvReaderTxn<'a> {
    fn borrow_mut(&self) -> std::cell::RefMut<'_, tikv_client::Transaction> {
        match self {
            TiKvReaderTxn::Owned(cell) => std::cell::RefMut::map(cell.borrow_mut(), |opt| {
                opt.as_mut().expect("transaction already consumed")
            }),
            TiKvReaderTxn::Borrowed(cell) => cell.borrow_mut(),
        }
    }
}

#[must_use]
pub struct TiKvStorageReader<'a> {
    storage: TiKvStorage,
    txn: TiKvReaderTxn<'a>,
}

impl Drop for TiKvStorageReader<'_> {
    fn drop(&mut self) {
        if let TiKvReaderTxn::Owned(cell) = &self.txn {
            if let Some(mut txn) = cell.borrow_mut().take() {
                let _ = self.storage.runtime().block_on(txn.rollback());
            }
        }
    }
}

impl<'a> TiKvStorageReader<'a> {
    pub fn len(&self) -> Result<usize, StorageError> {
        // Count gspo + dspo entries
        let gspo_count = self.scan_prefix_keys(TABLE_GSPO, &[])?.len();
        let dspo_count = self.scan_prefix_keys(TABLE_DSPO, &[])?.len();
        Ok(gspo_count + dspo_count)
    }

    pub fn is_empty(&self) -> Result<bool, StorageError> {
        let gspo_keys = self.scan_prefix_keys_limit(TABLE_GSPO, &[], 1)?;
        if !gspo_keys.is_empty() {
            return Ok(false);
        }
        let dspo_keys = self.scan_prefix_keys_limit(TABLE_DSPO, &[], 1)?;
        Ok(dspo_keys.is_empty())
    }

    pub fn contains(&self, quad: &EncodedQuad) -> Result<bool, StorageError> {
        let mut buffer = Vec::with_capacity(4 * WRITTEN_TERM_MAX_SIZE);
        if quad.graph_name.is_default_graph() {
            write_spo_quad(&mut buffer, quad);
            self.contains_key(TABLE_DSPO, &buffer)
        } else {
            write_gspo_quad(&mut buffer, quad);
            self.contains_key(TABLE_GSPO, &buffer)
        }
    }

    pub fn quads_for_pattern(
        &self,
        subject: Option<&EncodedTerm>,
        predicate: Option<&EncodedTerm>,
        object: Option<&EncodedTerm>,
        graph_name: Option<&EncodedTerm>,
    ) -> TiKvDecodingQuadIterator {
        match subject {
            Some(subject) => match predicate {
                Some(predicate) => match object {
                    Some(object) => match graph_name {
                        Some(graph_name) => self.quads_for_subject_predicate_object_graph(
                            subject, predicate, object, graph_name,
                        ),
                        None => self.quads_for_subject_predicate_object(subject, predicate, object),
                    },
                    None => match graph_name {
                        Some(graph_name) => {
                            self.quads_for_subject_predicate_graph(subject, predicate, graph_name)
                        }
                        None => self.quads_for_subject_predicate(subject, predicate),
                    },
                },
                None => match object {
                    Some(object) => match graph_name {
                        Some(graph_name) => {
                            self.quads_for_subject_object_graph(subject, object, graph_name)
                        }
                        None => self.quads_for_subject_object(subject, object),
                    },
                    None => match graph_name {
                        Some(graph_name) => self.quads_for_subject_graph(subject, graph_name),
                        None => self.quads_for_subject(subject),
                    },
                },
            },
            None => match predicate {
                Some(predicate) => match object {
                    Some(object) => match graph_name {
                        Some(graph_name) => {
                            self.quads_for_predicate_object_graph(predicate, object, graph_name)
                        }
                        None => self.quads_for_predicate_object(predicate, object),
                    },
                    None => match graph_name {
                        Some(graph_name) => self.quads_for_predicate_graph(predicate, graph_name),
                        None => self.quads_for_predicate(predicate),
                    },
                },
                None => match object {
                    Some(object) => match graph_name {
                        Some(graph_name) => self.quads_for_object_graph(object, graph_name),
                        None => self.quads_for_object(object),
                    },
                    None => match graph_name {
                        Some(graph_name) => self.quads_for_graph(graph_name),
                        None => self.quads(),
                    },
                },
            },
        }
    }

    fn quads(&self) -> TiKvDecodingQuadIterator {
        // For a full scan (no bound components), issue parallel scans across both
        // default-graph (dspo) and named-graph (gspo) index tables. Each prefetch
        // iterator fetches its first batch concurrently to reduce initial latency.
        let dspo_prefix = prefixed_key(TABLE_DSPO, &[]);
        let gspo_prefix = prefixed_key(TABLE_GSPO, &[]);
        let batch_size = self.storage.scan_batch_size();

        let dspo_range = prefix_range(&dspo_prefix);
        let gspo_range = prefix_range(&gspo_prefix);

        let dspo_upper = prefix_upper_bound(&dspo_prefix);
        let gspo_upper = prefix_upper_bound(&gspo_prefix);

        // Issue both initial batch fetches in parallel via tokio::join!
        let result = self.storage.inner.runtime.block_on(async {
            let client = &self.storage.inner.client;
            let (dspo_txn_res, gspo_txn_res) =
                tokio::join!(client.begin_optimistic(), client.begin_optimistic());
            let (dspo_txn, gspo_txn) = (dspo_txn_res?, gspo_txn_res?);
            let (mut dspo_txn, mut gspo_txn) = (dspo_txn, gspo_txn);
            let (dspo_scan, gspo_scan) = tokio::join!(
                dspo_txn.scan(dspo_range, batch_size as u32),
                gspo_txn.scan(gspo_range, batch_size as u32)
            );
            let dspo_pairs: Vec<KvPair> = dspo_scan?.collect();
            let gspo_pairs: Vec<KvPair> = gspo_scan?.collect();
            let _ = tokio::join!(dspo_txn.rollback(), gspo_txn.rollback());
            Ok::<_, tikv_client::Error>((dspo_pairs, gspo_pairs))
        });

        match result {
            Ok((dspo_pairs, gspo_pairs)) => {
                let dspo_exhausted = dspo_pairs.len() < batch_size;
                let gspo_exhausted = gspo_pairs.len() < batch_size;

                let dspo_next_start = dspo_pairs
                    .last()
                    .map(|p| {
                        let mut k: Vec<u8> = p.0.clone().into();
                        k.push(0x00);
                        k
                    })
                    .unwrap_or_else(|| dspo_prefix.clone());

                let gspo_next_start = gspo_pairs
                    .last()
                    .map(|p| {
                        let mut k: Vec<u8> = p.0.clone().into();
                        k.push(0x00);
                        k
                    })
                    .unwrap_or_else(|| gspo_prefix.clone());

                let dspo_buffer: Vec<Result<EncodedQuad, StorageError>> = dspo_pairs
                    .into_iter()
                    .map(|pair| {
                        let key: Vec<u8> = pair.0.into();
                        QuadEncoding::Dspo.decode(&key[1..])
                    })
                    .collect();

                let gspo_buffer: Vec<Result<EncodedQuad, StorageError>> = gspo_pairs
                    .into_iter()
                    .map(|pair| {
                        let key: Vec<u8> = pair.0.into();
                        QuadEncoding::Gspo.decode(&key[1..])
                    })
                    .collect();

                let first = TiKvPrefetchQuadIterator {
                    storage: self.storage.clone(),
                    encoding: QuadEncoding::Dspo,
                    scan_range_start: dspo_next_start,
                    scan_range_upper: dspo_upper,
                    buffer: dspo_buffer,
                    buffer_pos: 0,
                    batch_size,
                    exhausted: dspo_exhausted,
                };

                let second = TiKvPrefetchQuadIterator {
                    storage: self.storage.clone(),
                    encoding: QuadEncoding::Gspo,
                    scan_range_start: gspo_next_start,
                    scan_range_upper: gspo_upper,
                    buffer: gspo_buffer,
                    buffer_pos: 0,
                    batch_size,
                    exhausted: gspo_exhausted,
                };

                TiKvDecodingQuadIterator::pair(first, second)
            }
            Err(e) => {
                // On error, return iterators that will yield the error on first next()
                let mut first = TiKvPrefetchQuadIterator {
                    storage: self.storage.clone(),
                    encoding: QuadEncoding::Dspo,
                    scan_range_start: dspo_prefix,
                    scan_range_upper: dspo_upper,
                    buffer: vec![Err(map_tikv_error(e))],
                    buffer_pos: 0,
                    batch_size,
                    exhausted: true,
                };
                // Only the first iterator carries the error
                let _ = &mut first;
                TiKvDecodingQuadIterator::new(first)
            }
        }
    }

    fn quads_for_subject(&self, subject: &EncodedTerm) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::pair(
            self.dspo_quads(&encode_term(subject)),
            self.spog_quads(&encode_term(subject)),
        )
    }

    fn quads_for_subject_predicate(
        &self,
        subject: &EncodedTerm,
        predicate: &EncodedTerm,
    ) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::pair(
            self.dspo_quads(&encode_term_pair(subject, predicate)),
            self.spog_quads(&encode_term_pair(subject, predicate)),
        )
    }

    fn quads_for_subject_predicate_object(
        &self,
        subject: &EncodedTerm,
        predicate: &EncodedTerm,
        object: &EncodedTerm,
    ) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::pair(
            self.dspo_quads(&encode_term_triple(subject, predicate, object)),
            self.spog_quads(&encode_term_triple(subject, predicate, object)),
        )
    }

    fn quads_for_subject_object(
        &self,
        subject: &EncodedTerm,
        object: &EncodedTerm,
    ) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::pair(
            self.dosp_quads(&encode_term_pair(object, subject)),
            self.ospg_quads(&encode_term_pair(object, subject)),
        )
    }

    fn quads_for_predicate(&self, predicate: &EncodedTerm) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::pair(
            self.dpos_quads(&encode_term(predicate)),
            self.posg_quads(&encode_term(predicate)),
        )
    }

    fn quads_for_predicate_object(
        &self,
        predicate: &EncodedTerm,
        object: &EncodedTerm,
    ) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::pair(
            self.dpos_quads(&encode_term_pair(predicate, object)),
            self.posg_quads(&encode_term_pair(predicate, object)),
        )
    }

    fn quads_for_object(&self, object: &EncodedTerm) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::pair(
            self.dosp_quads(&encode_term(object)),
            self.ospg_quads(&encode_term(object)),
        )
    }

    fn quads_for_graph(&self, graph_name: &EncodedTerm) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::new(if graph_name.is_default_graph() {
            self.dspo_quads(&[])
        } else {
            self.gspo_quads(&encode_term(graph_name))
        })
    }

    fn quads_for_subject_graph(
        &self,
        subject: &EncodedTerm,
        graph_name: &EncodedTerm,
    ) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::new(if graph_name.is_default_graph() {
            self.dspo_quads(&encode_term(subject))
        } else {
            self.gspo_quads(&encode_term_pair(graph_name, subject))
        })
    }

    fn quads_for_subject_predicate_graph(
        &self,
        subject: &EncodedTerm,
        predicate: &EncodedTerm,
        graph_name: &EncodedTerm,
    ) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::new(if graph_name.is_default_graph() {
            self.dspo_quads(&encode_term_pair(subject, predicate))
        } else {
            self.gspo_quads(&encode_term_triple(graph_name, subject, predicate))
        })
    }

    fn quads_for_subject_predicate_object_graph(
        &self,
        subject: &EncodedTerm,
        predicate: &EncodedTerm,
        object: &EncodedTerm,
        graph_name: &EncodedTerm,
    ) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::new(if graph_name.is_default_graph() {
            self.dspo_quads(&encode_term_triple(subject, predicate, object))
        } else {
            self.gspo_quads(&encode_term_quad(graph_name, subject, predicate, object))
        })
    }

    fn quads_for_subject_object_graph(
        &self,
        subject: &EncodedTerm,
        object: &EncodedTerm,
        graph_name: &EncodedTerm,
    ) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::new(if graph_name.is_default_graph() {
            self.dosp_quads(&encode_term_pair(object, subject))
        } else {
            self.gosp_quads(&encode_term_triple(graph_name, object, subject))
        })
    }

    fn quads_for_predicate_graph(
        &self,
        predicate: &EncodedTerm,
        graph_name: &EncodedTerm,
    ) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::new(if graph_name.is_default_graph() {
            self.dpos_quads(&encode_term(predicate))
        } else {
            self.gpos_quads(&encode_term_pair(graph_name, predicate))
        })
    }

    fn quads_for_predicate_object_graph(
        &self,
        predicate: &EncodedTerm,
        object: &EncodedTerm,
        graph_name: &EncodedTerm,
    ) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::new(if graph_name.is_default_graph() {
            self.dpos_quads(&encode_term_pair(predicate, object))
        } else {
            self.gpos_quads(&encode_term_triple(graph_name, predicate, object))
        })
    }

    fn quads_for_object_graph(
        &self,
        object: &EncodedTerm,
        graph_name: &EncodedTerm,
    ) -> TiKvDecodingQuadIterator {
        TiKvDecodingQuadIterator::new(if graph_name.is_default_graph() {
            self.dosp_quads(&encode_term(object))
        } else {
            self.gosp_quads(&encode_term_pair(graph_name, object))
        })
    }

    pub fn named_graphs(&self) -> TiKvDecodingGraphIterator {
        let pairs = self.scan_prefix(TABLE_GRAPHS, &[]).unwrap_or_default();
        TiKvDecodingGraphIterator {
            pairs: pairs.into_iter(),
        }
    }

    pub fn contains_named_graph(&self, graph_name: &EncodedTerm) -> Result<bool, StorageError> {
        self.contains_key(TABLE_GRAPHS, &encode_term(graph_name))
    }

    fn spog_quads(&self, prefix: &[u8]) -> TiKvPrefetchQuadIterator {
        self.inner_quads(TABLE_SPOG, prefix, QuadEncoding::Spog)
    }

    fn posg_quads(&self, prefix: &[u8]) -> TiKvPrefetchQuadIterator {
        self.inner_quads(TABLE_POSG, prefix, QuadEncoding::Posg)
    }

    fn ospg_quads(&self, prefix: &[u8]) -> TiKvPrefetchQuadIterator {
        self.inner_quads(TABLE_OSPG, prefix, QuadEncoding::Ospg)
    }

    fn gspo_quads(&self, prefix: &[u8]) -> TiKvPrefetchQuadIterator {
        self.inner_quads(TABLE_GSPO, prefix, QuadEncoding::Gspo)
    }

    fn gpos_quads(&self, prefix: &[u8]) -> TiKvPrefetchQuadIterator {
        self.inner_quads(TABLE_GPOS, prefix, QuadEncoding::Gpos)
    }

    fn gosp_quads(&self, prefix: &[u8]) -> TiKvPrefetchQuadIterator {
        self.inner_quads(TABLE_GOSP, prefix, QuadEncoding::Gosp)
    }

    fn dspo_quads(&self, prefix: &[u8]) -> TiKvPrefetchQuadIterator {
        self.inner_quads(TABLE_DSPO, prefix, QuadEncoding::Dspo)
    }

    fn dpos_quads(&self, prefix: &[u8]) -> TiKvPrefetchQuadIterator {
        self.inner_quads(TABLE_DPOS, prefix, QuadEncoding::Dpos)
    }

    fn dosp_quads(&self, prefix: &[u8]) -> TiKvPrefetchQuadIterator {
        self.inner_quads(TABLE_DOSP, prefix, QuadEncoding::Dosp)
    }

    fn inner_quads(
        &self,
        table: u8,
        prefix: &[u8],
        encoding: QuadEncoding,
    ) -> TiKvPrefetchQuadIterator {
        let full_prefix = prefixed_key(table, prefix);
        TiKvPrefetchQuadIterator {
            storage: self.storage.clone(),
            encoding,
            scan_range_start: full_prefix.clone(),
            scan_range_upper: prefix_upper_bound(&full_prefix),
            buffer: Vec::new(),
            buffer_pos: 0,
            batch_size: self.storage.scan_batch_size(),
            exhausted: false,
        }
    }

    pub fn contains_str(&self, key: &StrHash) -> Result<bool, StorageError> {
        self.contains_key(TABLE_ID2STR, &key.to_be_bytes())
    }

    pub fn validate(&self) -> Result<(), StorageError> {
        // Basic validation: check that dspo/dpos/dosp counts match
        let dspo_count = self.scan_prefix_keys(TABLE_DSPO, &[])?.len();
        let dpos_count = self.scan_prefix_keys(TABLE_DPOS, &[])?.len();
        let dosp_count = self.scan_prefix_keys(TABLE_DOSP, &[])?.len();
        if dspo_count != dpos_count || dspo_count != dosp_count {
            return Err(CorruptionError::new(
                "Not the same number of triples in dspo, dpos and dosp",
            )
            .into());
        }

        let gspo_count = self.scan_prefix_keys(TABLE_GSPO, &[])?.len();
        let gpos_count = self.scan_prefix_keys(TABLE_GPOS, &[])?.len();
        let gosp_count = self.scan_prefix_keys(TABLE_GOSP, &[])?.len();
        let spog_count = self.scan_prefix_keys(TABLE_SPOG, &[])?.len();
        let posg_count = self.scan_prefix_keys(TABLE_POSG, &[])?.len();
        let ospg_count = self.scan_prefix_keys(TABLE_OSPG, &[])?.len();
        if gspo_count != gpos_count
            || gspo_count != gosp_count
            || gspo_count != spog_count
            || gspo_count != posg_count
            || gspo_count != ospg_count
        {
            return Err(CorruptionError::new(
                "Not the same number of quads across named graph indexes",
            )
            .into());
        }
        Ok(())
    }

    // --- Low-level TiKV helpers ---

    fn contains_key(&self, table: u8, key: &[u8]) -> Result<bool, StorageError> {
        let full_key = prefixed_key(table, key);
        let mut txn = self.txn.borrow_mut();
        self.storage
            .inner
            .runtime
            .block_on(txn.key_exists(full_key))
            .map_err(map_tikv_error)
    }

    fn get_value(&self, table: u8, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        let full_key = prefixed_key(table, key);
        let mut txn = self.txn.borrow_mut();
        self.storage
            .inner
            .runtime
            .block_on(txn.get(full_key))
            .map_err(map_tikv_error)
    }

    fn scan_prefix(&self, table: u8, prefix: &[u8]) -> Result<Vec<KvPair>, StorageError> {
        let full_prefix = prefixed_key(table, prefix);
        let range = prefix_range(&full_prefix);
        let mut txn = self.txn.borrow_mut();
        let pairs = self
            .storage
            .inner
            .runtime
            .block_on(txn.scan(range, u32::MAX))
            .map_err(map_tikv_error)?;
        Ok(pairs.collect())
    }

    fn scan_prefix_keys(&self, table: u8, prefix: &[u8]) -> Result<Vec<Key>, StorageError> {
        let full_prefix = prefixed_key(table, prefix);
        let range = prefix_range(&full_prefix);
        let mut txn = self.txn.borrow_mut();
        let keys = self
            .storage
            .inner
            .runtime
            .block_on(txn.scan_keys(range, u32::MAX))
            .map_err(map_tikv_error)?;
        Ok(keys.collect())
    }

    fn scan_prefix_keys_limit(
        &self,
        table: u8,
        prefix: &[u8],
        limit: u32,
    ) -> Result<Vec<Key>, StorageError> {
        let full_prefix = prefixed_key(table, prefix);
        let range = prefix_range(&full_prefix);
        let mut txn = self.txn.borrow_mut();
        let keys = self
            .storage
            .inner
            .runtime
            .block_on(txn.scan_keys(range, limit))
            .map_err(map_tikv_error)?;
        Ok(keys.collect())
    }
}

impl StrLookup for TiKvStorageReader<'_> {
    fn get_str(&self, key: &StrHash) -> Result<Option<String>, StorageError> {
        match self.get_value(TABLE_ID2STR, &key.to_be_bytes())? {
            Some(v) => Ok(Some(String::from_utf8(v).map_err(CorruptionError::new)?)),
            None => Ok(None),
        }
    }
}

// --- Iterators ---

/// A lazy, batch-prefetching iterator over quad entries in a single TiKV index table.
///
/// Instead of loading all matching keys in one scan request, this iterator fetches
/// `batch_size` entries at a time. When the buffer is exhausted it issues the next
/// scan starting from where the previous batch left off, until no more keys remain
/// in the prefix range.
#[must_use]
pub struct TiKvPrefetchQuadIterator {
    storage: TiKvStorage,
    encoding: QuadEncoding,
    /// The current start key for the next scan batch (inclusive).
    scan_range_start: Vec<u8>,
    /// The exclusive upper bound for the prefix range, or `None` for unbounded.
    scan_range_upper: Option<Vec<u8>>,
    /// Buffered decoded results from the last batch fetch.
    buffer: Vec<Result<EncodedQuad, StorageError>>,
    /// Current position within `buffer`.
    buffer_pos: usize,
    /// Number of entries to fetch per batch.
    batch_size: usize,
    /// True when the underlying scan has been fully consumed.
    exhausted: bool,
}

impl TiKvPrefetchQuadIterator {
    /// Fetch the next batch of entries from TiKV.
    fn fetch_next_batch(&mut self) -> Result<(), StorageError> {
        let range: BoundRange = if let Some(ref upper) = self.scan_range_upper {
            let start: Key = self.scan_range_start.clone().into();
            let end: Key = upper.clone().into();
            BoundRange::from(start..end)
        } else {
            let start: Key = self.scan_range_start.clone().into();
            BoundRange::from(start..)
        };

        let mut txn = self
            .storage
            .inner
            .runtime
            .block_on(self.storage.inner.client.begin_optimistic())
            .map_err(map_tikv_error)?;
        let pairs = self
            .storage
            .inner
            .runtime
            .block_on(txn.scan(range, self.batch_size as u32))
            .map_err(map_tikv_error)?;
        let pairs: Vec<KvPair> = pairs.collect();
        drop(self.storage.inner.runtime.block_on(txn.rollback()));

        if pairs.is_empty() || pairs.len() < self.batch_size {
            self.exhausted = true;
        }

        if let Some(last_pair) = pairs.last() {
            // Set the next scan start to just after the last key returned.
            // We do this by appending a 0x00 byte so the next scan is exclusive
            // of the last key we already received.
            let last_key: Vec<u8> = last_pair.0.clone().into();
            let mut next_start = last_key;
            next_start.push(0x00);
            self.scan_range_start = next_start;
        }

        self.buffer = pairs
            .into_iter()
            .map(|pair| {
                let key: Vec<u8> = pair.0.into();
                // Strip the 1-byte table prefix
                self.encoding.decode(&key[1..])
            })
            .collect();
        self.buffer_pos = 0;

        Ok(())
    }
}

impl Iterator for TiKvPrefetchQuadIterator {
    type Item = Result<EncodedQuad, StorageError>;

    fn next(&mut self) -> Option<Self::Item> {
        // Return from buffer if available
        if self.buffer_pos < self.buffer.len() {
            let pos = self.buffer_pos;
            self.buffer_pos += 1;
            // Use swap_remove-like approach: replace with a dummy value
            // Actually, since we track position, just index into it.
            return Some(std::mem::replace(
                &mut self.buffer[pos],
                Err(StorageError::Other("consumed".into())),
            ));
        }

        // Buffer exhausted — fetch next batch if not done
        if self.exhausted {
            return None;
        }

        match self.fetch_next_batch() {
            Ok(()) => {
                if self.buffer.is_empty() {
                    None
                } else {
                    self.buffer_pos = 1;
                    Some(std::mem::replace(
                        &mut self.buffer[0],
                        Err(StorageError::Other("consumed".into())),
                    ))
                }
            }
            Err(e) => {
                self.exhausted = true;
                Some(Err(e))
            }
        }
    }
}

/// Iterator over quads that chains one or two [`TiKvPrefetchQuadIterator`]s.
///
/// For queries spanning both default-graph and named-graph indexes, this chains
/// two prefetch iterators. For queries targeting a single graph type, only the
/// first iterator is used.
#[must_use]
pub struct TiKvDecodingQuadIterator {
    first: TiKvPrefetchQuadIterator,
    second: Option<TiKvPrefetchQuadIterator>,
}

impl TiKvDecodingQuadIterator {
    fn new(first: TiKvPrefetchQuadIterator) -> Self {
        Self {
            first,
            second: None,
        }
    }

    fn pair(first: TiKvPrefetchQuadIterator, second: TiKvPrefetchQuadIterator) -> Self {
        Self {
            first,
            second: Some(second),
        }
    }
}

impl Iterator for TiKvDecodingQuadIterator {
    type Item = Result<EncodedQuad, StorageError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(result) = self.first.next() {
            Some(result)
        } else if let Some(second) = &mut self.second {
            second.next()
        } else {
            None
        }
    }
}

#[must_use]
pub struct TiKvDecodingGraphIterator {
    pairs: std::vec::IntoIter<KvPair>,
}

impl Iterator for TiKvDecodingGraphIterator {
    type Item = Result<EncodedTerm, StorageError>;

    fn next(&mut self) -> Option<Self::Item> {
        let pair = self.pairs.next()?;
        let key: Vec<u8> = pair.0.into();
        // Strip the 1-byte table prefix
        Some(decode_term(&key[1..]))
    }
}

// --- Transaction (write-only) ---

#[must_use]
pub struct TiKvStorageTransaction<'a> {
    buffer: Vec<u8>,
    txn: Option<tikv_client::Transaction>,
    storage: &'a TiKvStorage,
}

impl TiKvStorageTransaction<'_> {
    pub fn insert(&mut self, quad: QuadRef<'_>) {
        let encoded: EncodedQuad = quad.into();
        self.buffer.clear();
        if quad.graph_name.is_default_graph() {
            write_spo_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_DSPO, &self.buffer.clone());

            self.buffer.clear();
            write_pos_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_DPOS, &self.buffer.clone());

            self.buffer.clear();
            write_osp_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_DOSP, &self.buffer.clone());

            self.insert_term(quad.subject.into(), &encoded.subject);
            self.insert_term(quad.predicate.into(), &encoded.predicate);
            self.insert_term(quad.object, &encoded.object);
        } else {
            write_spog_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_SPOG, &self.buffer.clone());

            self.buffer.clear();
            write_posg_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_POSG, &self.buffer.clone());

            self.buffer.clear();
            write_ospg_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_OSPG, &self.buffer.clone());

            self.buffer.clear();
            write_gspo_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_GSPO, &self.buffer.clone());

            self.buffer.clear();
            write_gpos_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_GPOS, &self.buffer.clone());

            self.buffer.clear();
            write_gosp_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_GOSP, &self.buffer.clone());

            self.insert_term(quad.subject.into(), &encoded.subject);
            self.insert_term(quad.predicate.into(), &encoded.predicate);
            self.insert_term(quad.object, &encoded.object);

            self.buffer.clear();
            write_term(&mut self.buffer, &encoded.graph_name);
            self.put_empty(TABLE_GRAPHS, &self.buffer.clone());
            self.insert_graph_name(quad.graph_name, &encoded.graph_name);
        }
    }

    pub fn insert_named_graph(&mut self, graph_name: NamedOrBlankNodeRef<'_>) {
        let encoded_graph_name: EncodedTerm = graph_name.into();
        self.buffer.clear();
        write_term(&mut self.buffer, &encoded_graph_name);
        self.put_empty(TABLE_GRAPHS, &self.buffer.clone());
        self.insert_term(graph_name.into(), &encoded_graph_name);
    }

    fn insert_term(&mut self, term: TermRef<'_>, encoded: &EncodedTerm) {
        insert_term(term, encoded, &mut |key, value| self.insert_str(key, value));
    }

    fn insert_graph_name(&mut self, graph_name: GraphNameRef<'_>, encoded: &EncodedTerm) {
        match graph_name {
            GraphNameRef::NamedNode(graph_name) => self.insert_term(graph_name.into(), encoded),
            GraphNameRef::BlankNode(graph_name) => self.insert_term(graph_name.into(), encoded),
            GraphNameRef::DefaultGraph => (),
        }
    }

    fn insert_str(&mut self, key: &StrHash, value: &str) {
        let full_key = prefixed_key(TABLE_ID2STR, &key.to_be_bytes());
        // Best effort — errors on individual puts are deferred to commit
        let _ = self.storage.runtime().block_on(
            self.txn
                .as_mut()
                .unwrap()
                .put(full_key, value.as_bytes().to_vec()),
        );
    }

    pub fn remove(&mut self, quad: QuadRef<'_>) {
        self.remove_encoded(&quad.into());
    }

    fn remove_encoded(&mut self, quad: &EncodedQuad) {
        self.buffer.clear();
        if quad.graph_name.is_default_graph() {
            write_spo_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_DSPO, &self.buffer.clone());

            self.buffer.clear();
            write_pos_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_DPOS, &self.buffer.clone());

            self.buffer.clear();
            write_osp_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_DOSP, &self.buffer.clone());
        } else {
            write_spog_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_SPOG, &self.buffer.clone());

            self.buffer.clear();
            write_posg_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_POSG, &self.buffer.clone());

            self.buffer.clear();
            write_ospg_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_OSPG, &self.buffer.clone());

            self.buffer.clear();
            write_gspo_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_GSPO, &self.buffer.clone());

            self.buffer.clear();
            write_gpos_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_GPOS, &self.buffer.clone());

            self.buffer.clear();
            write_gosp_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_GOSP, &self.buffer.clone());
        }
    }

    pub fn clear_default_graph(&mut self) {
        self.delete_prefix(TABLE_DSPO);
        self.delete_prefix(TABLE_DPOS);
        self.delete_prefix(TABLE_DOSP);
    }

    pub fn clear_all_named_graphs(&mut self) {
        self.delete_prefix(TABLE_GSPO);
        self.delete_prefix(TABLE_GPOS);
        self.delete_prefix(TABLE_GOSP);
        self.delete_prefix(TABLE_SPOG);
        self.delete_prefix(TABLE_POSG);
        self.delete_prefix(TABLE_OSPG);
    }

    pub fn clear_all_graphs(&mut self) {
        self.clear_default_graph();
        self.remove_all_named_graphs();
    }

    pub fn remove_all_named_graphs(&mut self) {
        self.clear_all_named_graphs();
        self.delete_prefix(TABLE_GRAPHS);
    }

    pub fn clear(&mut self) {
        self.clear_default_graph();
        self.remove_all_named_graphs();
    }

    pub fn commit(mut self) -> Result<(), StorageError> {
        let mut txn = self.txn.take().unwrap();
        self.storage
            .runtime()
            .block_on(txn.commit())
            .map_err(map_tikv_error)?;
        Ok(())
    }

    fn put_empty(&mut self, table: u8, key: &[u8]) {
        let full_key = prefixed_key(table, key);
        let _ = self
            .storage
            .runtime()
            .block_on(self.txn.as_mut().unwrap().put(full_key, vec![]));
    }

    fn delete_key(&mut self, table: u8, key: &[u8]) {
        let full_key = prefixed_key(table, key);
        let _ = self
            .storage
            .runtime()
            .block_on(self.txn.as_mut().unwrap().delete(full_key));
    }

    fn delete_prefix(&mut self, table: u8) {
        // Scan all keys with this table prefix and delete them
        let full_prefix = prefixed_key(table, &[]);
        let range = prefix_range(&full_prefix);
        let keys = self
            .storage
            .runtime()
            .block_on(self.txn.as_mut().unwrap().scan_keys(range, u32::MAX));
        if let Ok(keys) = keys {
            let keys: Vec<Key> = keys.collect();
            for key in keys {
                let _ = self
                    .storage
                    .runtime()
                    .block_on(self.txn.as_mut().unwrap().delete(key));
            }
        }
    }
}

impl Drop for TiKvStorageTransaction<'_> {
    fn drop(&mut self) {
        if let Some(mut txn) = self.txn.take() {
            let _ = self.storage.runtime().block_on(txn.rollback());
        }
    }
}

// --- Readable Transaction (read + write) ---

#[must_use]
pub struct TiKvStorageReadableTransaction<'a> {
    buffer: Vec<u8>,
    txn: Option<RefCell<tikv_client::Transaction>>,
    storage: &'a TiKvStorage,
}

impl TiKvStorageReadableTransaction<'_> {
    pub fn reader(&self) -> TiKvStorageReader<'_> {
        // The reader borrows the same transaction, so it sees uncommitted writes.
        TiKvStorageReader {
            storage: self.storage.clone(),
            txn: TiKvReaderTxn::Borrowed(self.txn.as_ref().unwrap()),
        }
    }

    pub fn insert(&mut self, quad: QuadRef<'_>) {
        let encoded: EncodedQuad = quad.into();
        self.buffer.clear();
        if quad.graph_name.is_default_graph() {
            write_spo_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_DSPO, &self.buffer.clone());

            self.buffer.clear();
            write_pos_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_DPOS, &self.buffer.clone());

            self.buffer.clear();
            write_osp_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_DOSP, &self.buffer.clone());

            self.insert_term(quad.subject.into(), &encoded.subject);
            self.insert_term(quad.predicate.into(), &encoded.predicate);
            self.insert_term(quad.object, &encoded.object);
        } else {
            write_spog_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_SPOG, &self.buffer.clone());

            self.buffer.clear();
            write_posg_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_POSG, &self.buffer.clone());

            self.buffer.clear();
            write_ospg_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_OSPG, &self.buffer.clone());

            self.buffer.clear();
            write_gspo_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_GSPO, &self.buffer.clone());

            self.buffer.clear();
            write_gpos_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_GPOS, &self.buffer.clone());

            self.buffer.clear();
            write_gosp_quad(&mut self.buffer, &encoded);
            self.put_empty(TABLE_GOSP, &self.buffer.clone());

            self.insert_term(quad.subject.into(), &encoded.subject);
            self.insert_term(quad.predicate.into(), &encoded.predicate);
            self.insert_term(quad.object, &encoded.object);

            self.buffer.clear();
            write_term(&mut self.buffer, &encoded.graph_name);
            self.put_empty(TABLE_GRAPHS, &self.buffer.clone());
            self.insert_graph_name(quad.graph_name, &encoded.graph_name);
        }
    }

    pub fn insert_named_graph(&mut self, graph_name: NamedOrBlankNodeRef<'_>) {
        let encoded_graph_name: EncodedTerm = graph_name.into();
        self.buffer.clear();
        write_term(&mut self.buffer, &encoded_graph_name);
        self.put_empty(TABLE_GRAPHS, &self.buffer.clone());
        self.insert_term(graph_name.into(), &encoded_graph_name);
    }

    fn insert_term(&mut self, term: TermRef<'_>, encoded: &EncodedTerm) {
        insert_term(term, encoded, &mut |key, value| self.insert_str(key, value));
    }

    fn insert_graph_name(&mut self, graph_name: GraphNameRef<'_>, encoded: &EncodedTerm) {
        match graph_name {
            GraphNameRef::NamedNode(graph_name) => self.insert_term(graph_name.into(), encoded),
            GraphNameRef::BlankNode(graph_name) => self.insert_term(graph_name.into(), encoded),
            GraphNameRef::DefaultGraph => (),
        }
    }

    fn insert_str(&mut self, key: &StrHash, value: &str) {
        let full_key = prefixed_key(TABLE_ID2STR, &key.to_be_bytes());
        let _ = self.storage.runtime().block_on(
            self.txn
                .as_ref()
                .unwrap()
                .borrow_mut()
                .put(full_key, value.as_bytes().to_vec()),
        );
    }

    pub fn remove(&mut self, quad: QuadRef<'_>) {
        self.remove_encoded(&quad.into());
    }

    fn remove_encoded(&mut self, quad: &EncodedQuad) {
        self.buffer.clear();
        if quad.graph_name.is_default_graph() {
            write_spo_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_DSPO, &self.buffer.clone());

            self.buffer.clear();
            write_pos_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_DPOS, &self.buffer.clone());

            self.buffer.clear();
            write_osp_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_DOSP, &self.buffer.clone());
        } else {
            write_spog_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_SPOG, &self.buffer.clone());

            self.buffer.clear();
            write_posg_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_POSG, &self.buffer.clone());

            self.buffer.clear();
            write_ospg_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_OSPG, &self.buffer.clone());

            self.buffer.clear();
            write_gspo_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_GSPO, &self.buffer.clone());

            self.buffer.clear();
            write_gpos_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_GPOS, &self.buffer.clone());

            self.buffer.clear();
            write_gosp_quad(&mut self.buffer, quad);
            self.delete_key(TABLE_GOSP, &self.buffer.clone());
        }
    }

    pub fn clear_graph(&mut self, graph_name: GraphNameRef<'_>) -> Result<(), StorageError> {
        self.clear_encoded_graph(&graph_name.into())
    }

    fn clear_encoded_graph(&mut self, graph_name: &EncodedTerm) -> Result<(), StorageError> {
        // Read quads in batches and remove them
        loop {
            let quads: Vec<EncodedQuad> = self
                .reader()
                .quads_for_graph(graph_name)
                .take(BATCH_SIZE)
                .collect::<Result<Vec<_>, _>>()?;
            for quad in &quads {
                self.remove_encoded(quad);
            }
            if quads.len() < BATCH_SIZE {
                return Ok(());
            }
        }
    }

    pub fn clear_all_named_graphs(&mut self) -> Result<(), StorageError> {
        loop {
            let graph_names = self
                .reader()
                .named_graphs()
                .take(BATCH_SIZE)
                .collect::<Result<Vec<_>, _>>()?;
            for graph_name in &graph_names {
                self.clear_encoded_graph(graph_name)?;
            }
            if graph_names.len() < BATCH_SIZE {
                return Ok(());
            }
        }
    }

    pub fn clear_all_graphs(&mut self) -> Result<(), StorageError> {
        self.clear_all_named_graphs()?;
        self.clear_graph(GraphNameRef::DefaultGraph)
    }

    pub fn remove_named_graph(
        &mut self,
        graph_name: NamedOrBlankNodeRef<'_>,
    ) -> Result<(), StorageError> {
        self.remove_encoded_named_graph(&graph_name.into())
    }

    fn remove_encoded_named_graph(&mut self, graph_name: &EncodedTerm) -> Result<(), StorageError> {
        self.clear_encoded_graph(graph_name)?;
        self.buffer.clear();
        write_term(&mut self.buffer, graph_name);
        self.delete_key(TABLE_GRAPHS, &self.buffer.clone());
        Ok(())
    }

    pub fn remove_all_named_graphs(&mut self) -> Result<(), StorageError> {
        loop {
            let graph_names = self
                .reader()
                .named_graphs()
                .take(BATCH_SIZE)
                .collect::<Result<Vec<_>, _>>()?;
            for graph_name in &graph_names {
                self.remove_encoded_named_graph(graph_name)?;
            }
            if graph_names.len() < BATCH_SIZE {
                return Ok(());
            }
        }
    }

    pub fn clear(&mut self) -> Result<(), StorageError> {
        self.remove_all_named_graphs()?;
        self.clear_graph(GraphNameRef::DefaultGraph)
    }

    pub fn commit(mut self) -> Result<(), StorageError> {
        let mut txn = self.txn.take().unwrap().into_inner();
        self.storage
            .runtime()
            .block_on(txn.commit())
            .map_err(map_tikv_error)?;
        Ok(())
    }

    fn put_empty(&mut self, table: u8, key: &[u8]) {
        let full_key = prefixed_key(table, key);
        let _ = self.storage.runtime().block_on(
            self.txn
                .as_ref()
                .unwrap()
                .borrow_mut()
                .put(full_key, vec![]),
        );
    }

    fn delete_key(&mut self, table: u8, key: &[u8]) {
        let full_key = prefixed_key(table, key);
        let _ = self
            .storage
            .runtime()
            .block_on(self.txn.as_ref().unwrap().borrow_mut().delete(full_key));
    }
}

impl Drop for TiKvStorageReadableTransaction<'_> {
    fn drop(&mut self) {
        if let Some(cell) = self.txn.take() {
            let _ = self
                .storage
                .runtime()
                .block_on(cell.into_inner().rollback());
        }
    }
}

// --- Bulk Loader ---

#[must_use]
pub struct TiKvStorageBulkLoader<'a> {
    storage: &'a TiKvStorage,
    hooks: Vec<Box<dyn Fn(u64) + Send + Sync>>,
    done: u64,
}

impl TiKvStorageBulkLoader<'_> {
    pub fn on_progress(mut self, callback: impl Fn(u64) + Send + Sync + 'static) -> Self {
        self.hooks.push(Box::new(callback));
        self
    }

    pub fn without_atomicity(self) -> Self {
        // TiKV transactions are always atomic; this is a no-op for now
        self
    }

    pub fn load_batch(
        &mut self,
        quads: Vec<Quad>,
        _max_num_threads: usize,
    ) -> Result<(), StorageError> {
        let mut txn = self.storage.start_transaction()?;
        for quad in quads {
            txn.insert(quad.as_ref());
            self.done += 1;
            if self.done.is_multiple_of(1_000_000) {
                for hook in &self.hooks {
                    hook(self.done);
                }
            }
        }
        txn.commit()
    }

    pub fn commit(self) -> Result<(), StorageError> {
        // All data was committed in load_batch calls
        Ok(())
    }
}
