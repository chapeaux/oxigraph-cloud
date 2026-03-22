use dashmap::DashMap;
use oxigraph::io::{RdfFormat, RdfParser};
use oxigraph::model::Quad;
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A buffered operation within a transaction.
#[derive(Clone)]
pub enum BufferedOp {
    InsertQuads(Vec<Quad>),
    RemoveQuads(Vec<Quad>),
    SparqlUpdate(String),
}

/// Server-side transaction state (buffered, not a live DB transaction).
struct ActiveTransaction {
    pub ops: Vec<BufferedOp>,
    pub last_active: Instant,
}

/// Manages server-side HTTP transactions.
///
/// Because `Transaction<'a>` borrows the `Store` and can't be held across
/// HTTP requests, we buffer operations in memory and replay them into a
/// real transaction on commit.
pub struct TransactionRegistry {
    txns: DashMap<String, ActiveTransaction>,
    timeout: Duration,
}

/// Result of committing a transaction.
pub struct CommitResult {
    pub txn_id: String,
    pub ops: Vec<BufferedOp>,
}

impl TransactionRegistry {
    pub fn new(timeout: Duration) -> Self {
        Self {
            txns: DashMap::new(),
            timeout,
        }
    }

    /// Begin a new transaction. Returns the transaction UUID.
    pub fn begin(&self) -> String {
        let id = Uuid::new_v4().to_string();
        self.txns.insert(
            id.clone(),
            ActiveTransaction {
                ops: Vec::new(),
                last_active: Instant::now(),
            },
        );
        tracing::info!(txn_id = %id, "Transaction started");
        id
    }

    /// Add RDF statements to a transaction by parsing a body.
    pub fn add(
        &self,
        txn_id: &str,
        format: RdfFormat,
        body: &[u8],
    ) -> Result<usize, TransactionError> {
        let parser = RdfParser::from_format(format);
        let quads: Vec<Quad> = parser
            .for_slice(body)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| TransactionError::Parse(e.to_string()))?;
        let count = quads.len();
        let mut txn = self
            .txns
            .get_mut(txn_id)
            .ok_or(TransactionError::NotFound)?;
        txn.last_active = Instant::now();
        txn.ops.push(BufferedOp::InsertQuads(quads));
        Ok(count)
    }

    /// Remove RDF statements from a transaction by parsing a body.
    pub fn remove(
        &self,
        txn_id: &str,
        format: RdfFormat,
        body: &[u8],
    ) -> Result<usize, TransactionError> {
        let parser = RdfParser::from_format(format);
        let quads: Vec<Quad> = parser
            .for_slice(body)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| TransactionError::Parse(e.to_string()))?;
        let count = quads.len();
        let mut txn = self
            .txns
            .get_mut(txn_id)
            .ok_or(TransactionError::NotFound)?;
        txn.last_active = Instant::now();
        txn.ops.push(BufferedOp::RemoveQuads(quads));
        Ok(count)
    }

    /// Buffer a SPARQL UPDATE within the transaction.
    pub fn update(&self, txn_id: &str, sparql: String) -> Result<(), TransactionError> {
        let mut txn = self
            .txns
            .get_mut(txn_id)
            .ok_or(TransactionError::NotFound)?;
        txn.last_active = Instant::now();
        txn.ops.push(BufferedOp::SparqlUpdate(sparql));
        Ok(())
    }

    /// Execute a SPARQL query within the transaction context.
    ///
    /// Opens a temporary transaction, replays all buffered ops, runs the query,
    /// then drops the transaction (rollback — no side effects).
    pub fn query(&self, txn_id: &str, store: &Store, sparql: &str) -> Result<String, TransactionError> {
        let txn = self
            .txns
            .get(txn_id)
            .ok_or(TransactionError::NotFound)?;
        let ops = txn.ops.clone();
        drop(txn); // release DashMap lock before taking store transaction

        let mut db_txn = store
            .start_transaction()
            .map_err(|e| TransactionError::Storage(e.to_string()))?;
        replay_ops(&mut db_txn, &ops)?;

        let evaluator = SparqlEvaluator::new();
        let prepared = evaluator
            .parse_query(sparql)
            .map_err(|e| TransactionError::Query(e.to_string()))?;
        let results = prepared
            .on_transaction(&db_txn)
            .execute()
            .map_err(|e| TransactionError::Query(e.to_string()))?;

        // Serialize results to JSON string
        let output = serialize_query_results(results)?;
        // db_txn is dropped here — implicit rollback, no side effects
        Ok(output)
    }

    /// Commit the transaction: replay all buffered ops into a real transaction.
    /// Returns the committed ops for changelog recording.
    pub fn commit(&self, txn_id: &str, store: &Store) -> Result<CommitResult, TransactionError> {
        let (_, active) = self
            .txns
            .remove(txn_id)
            .ok_or(TransactionError::NotFound)?;

        let mut db_txn = store
            .start_transaction()
            .map_err(|e| TransactionError::Storage(e.to_string()))?;
        replay_ops(&mut db_txn, &active.ops)?;
        db_txn
            .commit()
            .map_err(|e| TransactionError::Storage(e.to_string()))?;

        tracing::info!(txn_id = %txn_id, "Transaction committed");

        Ok(CommitResult {
            txn_id: txn_id.to_owned(),
            ops: active.ops,
        })
    }

    /// Rollback (discard) a transaction.
    pub fn rollback(&self, txn_id: &str) -> Result<(), TransactionError> {
        self.txns
            .remove(txn_id)
            .ok_or(TransactionError::NotFound)?;
        tracing::info!(txn_id = %txn_id, "Transaction rolled back");
        Ok(())
    }

    /// Remove transactions that have been idle longer than the timeout.
    /// Returns the number of expired transactions cleaned up.
    pub fn cleanup_expired(&self) -> usize {
        let mut expired = Vec::new();
        for entry in &self.txns {
            if entry.last_active.elapsed() > self.timeout {
                expired.push(entry.key().clone());
            }
        }
        let count = expired.len();
        for id in &expired {
            self.txns.remove(id);
            tracing::info!(txn_id = %id, "Expired transaction cleaned up");
        }
        count
    }

}

/// Replay buffered operations into a real `Transaction`.
fn replay_ops(
    txn: &mut oxigraph::store::Transaction<'_>,
    ops: &[BufferedOp],
) -> Result<(), TransactionError> {
    for op in ops {
        match op {
            BufferedOp::InsertQuads(quads) => {
                for quad in quads {
                    txn.insert(quad);
                }
            }
            BufferedOp::RemoveQuads(quads) => {
                for quad in quads {
                    txn.remove(quad);
                }
            }
            BufferedOp::SparqlUpdate(sparql) => {
                txn.update(sparql.as_str())
                    .map_err(|e| TransactionError::Query(e.to_string()))?;
            }
        }
    }
    Ok(())
}

/// Serialize SPARQL query results to a JSON string.
fn serialize_query_results(results: QueryResults<'_>) -> Result<String, TransactionError> {
    use oxigraph::sparql::results::{QueryResultsFormat, QueryResultsSerializer};

    match results {
        QueryResults::Solutions(solutions) => {
            let mut buf = Vec::new();
            let mut writer = QueryResultsSerializer::from_format(QueryResultsFormat::Json)
                .serialize_solutions_to_writer(&mut buf, solutions.variables().to_vec())
                .map_err(|e| TransactionError::Query(e.to_string()))?;
            for solution in solutions {
                let solution = solution.map_err(|e| TransactionError::Query(e.to_string()))?;
                writer
                    .serialize(&solution)
                    .map_err(|e| TransactionError::Query(e.to_string()))?;
            }
            writer
                .finish()
                .map_err(|e| TransactionError::Query(e.to_string()))?;
            String::from_utf8(buf).map_err(|e| TransactionError::Query(e.to_string()))
        }
        QueryResults::Boolean(result) => Ok(format!("{{\"boolean\": {result}}}")),
        QueryResults::Graph(_) => Err(TransactionError::Query(
            "CONSTRUCT/DESCRIBE queries not supported in transaction context".to_owned(),
        )),
    }
}

#[derive(Debug)]
pub enum TransactionError {
    NotFound,
    Parse(String),
    Storage(String),
    Query(String),
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "Transaction not found or already committed/rolled back"),
            Self::Parse(e) => write!(f, "RDF parse error: {e}"),
            Self::Storage(e) => write!(f, "Storage error: {e}"),
            Self::Query(e) => write!(f, "Query error: {e}"),
        }
    }
}
