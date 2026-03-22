use crate::transactions::BufferedOp;
use oxigraph::io::{RdfFormat, RdfParser, RdfSerializer};
use oxigraph::model::*;
use oxigraph::store::Store;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const CHANGELOG_GRAPH: &str = "urn:oxigraph:changelog";
const MAX_UNDOABLE_QUADS: usize = 100_000;

/// Records committed transaction deltas and supports undo.
pub struct Changelog {
    enabled: bool,
    retain: usize,
    counter: AtomicU64,
    /// Serialize changelog writes to avoid interleaving
    write_lock: Mutex<()>,
}

/// A changelog entry representing a committed transaction.
#[derive(Debug, Clone)]
pub struct ChangelogEntry {
    pub id: u64,
    pub timestamp: String,
    pub operation: String,
    pub inserted: Vec<Quad>,
    pub removed: Vec<Quad>,
    pub undoable: bool,
}

impl Changelog {
    pub fn new(enabled: bool, retain: usize) -> Self {
        Self {
            enabled,
            retain,
            counter: AtomicU64::new(1),
            write_lock: Mutex::new(()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Initialize the counter from existing changelog entries in the store.
    pub fn init_counter(&self, store: &Store) {
        if !self.enabled {
            return;
        }
        let graph = NamedNode::new_unchecked(CHANGELOG_GRAPH);
        let pred_id = NamedNode::new_unchecked("urn:oxigraph:txnId");
        let mut max_id: u64 = 0;
        for quad in store
            .quads_for_pattern(
                None,
                Some(pred_id.as_ref()),
                None,
                Some(graph.as_ref().into()),
            )
            .flatten()
        {
            if let Term::Literal(lit) = &quad.object {
                if let Ok(id) = lit.value().parse::<u64>() {
                    if id > max_id {
                        max_id = id;
                    }
                }
            }
        }
        self.counter.store(max_id + 1, Ordering::Relaxed);
        if max_id > 0 {
            tracing::info!(max_id = max_id, "Changelog counter initialized from store");
        }
    }

    /// Record a transaction's operations as a changelog entry.
    pub fn record(
        &self,
        store: &Store,
        ops: &[BufferedOp],
        operation: &str,
    ) -> Result<ChangelogEntry, ChangelogError> {
        if !self.enabled {
            return Err(ChangelogError::Disabled);
        }

        let _lock = self
            .write_lock
            .lock()
            .map_err(|e| ChangelogError::Internal(e.to_string()))?;

        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        let timestamp = format_timestamp();

        let mut inserted = Vec::new();
        let mut removed = Vec::new();
        for op in ops {
            match op {
                BufferedOp::InsertQuads(quads) => inserted.extend(quads.iter().cloned()),
                BufferedOp::RemoveQuads(quads) => removed.extend(quads.iter().cloned()),
                BufferedOp::SparqlUpdate(_) => {}
            }
        }

        let has_sparql_updates = ops
            .iter()
            .any(|op| matches!(op, BufferedOp::SparqlUpdate(_)));

        let total_quads = inserted.len() + removed.len();
        let undoable = !has_sparql_updates && total_quads <= MAX_UNDOABLE_QUADS;

        let entry = ChangelogEntry {
            id,
            timestamp,
            operation: operation.to_owned(),
            inserted,
            removed,
            undoable,
        };

        write_entry_to_store(store, &entry)?;

        if self.retain > 0 {
            self.prune(store, id)?;
        }

        tracing::info!(
            changelog_id = id,
            operation = operation,
            inserted = entry.inserted.len(),
            removed = entry.removed.len(),
            undoable = undoable,
            "Changelog entry recorded"
        );

        Ok(entry)
    }

    /// List changelog entries (newest first).
    pub fn list(
        &self,
        store: &Store,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ChangelogEntry>, ChangelogError> {
        if !self.enabled {
            return Err(ChangelogError::Disabled);
        }

        let graph = NamedNode::new_unchecked(CHANGELOG_GRAPH);
        let pred_id = NamedNode::new_unchecked("urn:oxigraph:txnId");
        let pred_ts = NamedNode::new_unchecked("urn:oxigraph:timestamp");
        let pred_op = NamedNode::new_unchecked("urn:oxigraph:operation");
        let pred_undoable = NamedNode::new_unchecked("urn:oxigraph:undoable");

        let mut entries: Vec<(u64, NamedNode)> = Vec::new();
        for quad in store
            .quads_for_pattern(
                None,
                Some(pred_id.as_ref()),
                None,
                Some(graph.as_ref().into()),
            )
            .flatten()
        {
            if let (Term::NamedNode(subj), Term::Literal(lit)) = (quad.subject.into(), &quad.object)
            {
                if let Ok(id) = lit.value().parse::<u64>() {
                    entries.push((id, subj));
                }
            }
        }

        entries.sort_by(|a, b| b.0.cmp(&a.0));
        let page = entries.into_iter().skip(offset).take(limit);

        let mut result = Vec::new();
        for (id, subj) in page {
            let mut timestamp = String::new();
            let mut operation = String::new();
            let mut undoable = false;

            for quad in store
                .quads_for_pattern(
                    Some(subj.as_ref().into()),
                    None,
                    None,
                    Some(graph.as_ref().into()),
                )
                .flatten()
            {
                let pred = quad.predicate;
                if let Term::Literal(lit) = &quad.object {
                    if pred == pred_ts {
                        lit.value().clone_into(&mut timestamp);
                    } else if pred == pred_op {
                        lit.value().clone_into(&mut operation);
                    } else if pred == pred_undoable {
                        undoable = lit.value() == "true";
                    }
                }
            }

            result.push(ChangelogEntry {
                id,
                timestamp,
                operation,
                inserted: Vec::new(),
                removed: Vec::new(),
                undoable,
            });
        }

        Ok(result)
    }

    /// Get a single changelog entry with full delta.
    pub fn get(&self, store: &Store, id: u64) -> Result<Option<ChangelogEntry>, ChangelogError> {
        if !self.enabled {
            return Err(ChangelogError::Disabled);
        }

        let graph = NamedNode::new_unchecked(CHANGELOG_GRAPH);
        let subj = NamedNode::new_unchecked(format!("urn:oxigraph:changelog:{id}"));

        let mut found = false;
        let mut timestamp = String::new();
        let mut operation = String::new();
        let mut undoable = false;
        let mut inserted_nq = String::new();
        let mut removed_nq = String::new();

        let pred_ts = NamedNode::new_unchecked("urn:oxigraph:timestamp");
        let pred_op = NamedNode::new_unchecked("urn:oxigraph:operation");
        let pred_undoable = NamedNode::new_unchecked("urn:oxigraph:undoable");
        let pred_inserted = NamedNode::new_unchecked("urn:oxigraph:insertedNQuads");
        let pred_removed = NamedNode::new_unchecked("urn:oxigraph:removedNQuads");

        for quad in store
            .quads_for_pattern(
                Some(subj.as_ref().into()),
                None,
                None,
                Some(graph.as_ref().into()),
            )
            .flatten()
        {
            found = true;
            let pred = quad.predicate;
            if let Term::Literal(lit) = &quad.object {
                if pred == pred_ts {
                    lit.value().clone_into(&mut timestamp);
                } else if pred == pred_op {
                    lit.value().clone_into(&mut operation);
                } else if pred == pred_undoable {
                    undoable = lit.value() == "true";
                } else if pred == pred_inserted {
                    lit.value().clone_into(&mut inserted_nq);
                } else if pred == pred_removed {
                    lit.value().clone_into(&mut removed_nq);
                }
            }
        }

        if !found {
            return Ok(None);
        }

        Ok(Some(ChangelogEntry {
            id,
            timestamp,
            operation,
            inserted: parse_nquads(&inserted_nq),
            removed: parse_nquads(&removed_nq),
            undoable,
        }))
    }

    /// Undo a changelog entry by applying the inverse operations.
    pub fn undo(&self, store: &Store, id: u64) -> Result<ChangelogEntry, ChangelogError> {
        let entry = self.get(store, id)?.ok_or(ChangelogError::NotFound)?;

        if !entry.undoable {
            return Err(ChangelogError::NotUndoable);
        }

        let mut txn = store
            .start_transaction()
            .map_err(|e| ChangelogError::Internal(e.to_string()))?;

        for quad in &entry.removed {
            txn.insert(quad);
        }
        for quad in &entry.inserted {
            txn.remove(quad);
        }
        txn.commit()
            .map_err(|e| ChangelogError::Internal(e.to_string()))?;

        let inverse_ops = vec![
            BufferedOp::InsertQuads(entry.removed.clone()),
            BufferedOp::RemoveQuads(entry.inserted.clone()),
        ];
        let undo_entry = self.record(store, &inverse_ops, &format!("undo:{id}"))?;

        tracing::info!(
            original_id = id,
            undo_id = undo_entry.id,
            "Transaction undone"
        );
        Ok(undo_entry)
    }

    /// Purge all changelog entries.
    pub fn purge(&self, store: &Store) -> Result<usize, ChangelogError> {
        if !self.enabled {
            return Err(ChangelogError::Disabled);
        }

        let graph = NamedNode::new_unchecked(CHANGELOG_GRAPH);
        let quads: Vec<Quad> = store
            .quads_for_pattern(None, None, None, Some(graph.as_ref().into()))
            .flatten()
            .collect();
        let count = quads.len();

        let mut txn = store
            .start_transaction()
            .map_err(|e| ChangelogError::Internal(e.to_string()))?;
        for quad in &quads {
            txn.remove(quad);
        }
        txn.commit()
            .map_err(|e| ChangelogError::Internal(e.to_string()))?;

        tracing::info!(count = count, "Changelog purged");
        Ok(count)
    }

    fn prune(&self, store: &Store, current_id: u64) -> Result<(), ChangelogError> {
        if current_id <= self.retain as u64 {
            return Ok(());
        }
        let cutoff_id = current_id - self.retain as u64;

        let graph = NamedNode::new_unchecked(CHANGELOG_GRAPH);
        let pred_id = NamedNode::new_unchecked("urn:oxigraph:txnId");

        let mut to_delete_subjects: Vec<NamedNode> = Vec::new();
        for quad in store
            .quads_for_pattern(
                None,
                Some(pred_id.as_ref()),
                None,
                Some(graph.as_ref().into()),
            )
            .flatten()
        {
            if let Term::Literal(lit) = &quad.object {
                if let Ok(id) = lit.value().parse::<u64>() {
                    if id <= cutoff_id {
                        if let Term::NamedNode(subj) = quad.subject.into() {
                            to_delete_subjects.push(subj);
                        }
                    }
                }
            }
        }

        if to_delete_subjects.is_empty() {
            return Ok(());
        }

        let mut txn = store
            .start_transaction()
            .map_err(|e| ChangelogError::Internal(e.to_string()))?;

        for subj in &to_delete_subjects {
            for quad in store
                .quads_for_pattern(
                    Some(subj.as_ref().into()),
                    None,
                    None,
                    Some(graph.as_ref().into()),
                )
                .flatten()
            {
                txn.remove(&quad);
            }
        }

        txn.commit()
            .map_err(|e| ChangelogError::Internal(e.to_string()))?;

        tracing::debug!(
            pruned = to_delete_subjects.len(),
            cutoff_id = cutoff_id,
            "Changelog entries pruned"
        );
        Ok(())
    }
}

fn write_entry_to_store(store: &Store, entry: &ChangelogEntry) -> Result<(), ChangelogError> {
    let graph = NamedNode::new_unchecked(CHANGELOG_GRAPH);
    let subj = NamedNode::new_unchecked(format!("urn:oxigraph:changelog:{}", entry.id));

    let mut txn = store
        .start_transaction()
        .map_err(|e| ChangelogError::Internal(e.to_string()))?;

    txn.insert(QuadRef::new(
        subj.as_ref(),
        NamedNodeRef::new_unchecked("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        NamedNodeRef::new_unchecked("urn:oxigraph:ChangelogEntry"),
        graph.as_ref(),
    ));
    txn.insert(QuadRef::new(
        subj.as_ref(),
        NamedNodeRef::new_unchecked("urn:oxigraph:txnId"),
        Literal::new_simple_literal(entry.id.to_string()).as_ref(),
        graph.as_ref(),
    ));
    txn.insert(QuadRef::new(
        subj.as_ref(),
        NamedNodeRef::new_unchecked("urn:oxigraph:timestamp"),
        Literal::new_simple_literal(&entry.timestamp).as_ref(),
        graph.as_ref(),
    ));
    txn.insert(QuadRef::new(
        subj.as_ref(),
        NamedNodeRef::new_unchecked("urn:oxigraph:operation"),
        Literal::new_simple_literal(&entry.operation).as_ref(),
        graph.as_ref(),
    ));
    txn.insert(QuadRef::new(
        subj.as_ref(),
        NamedNodeRef::new_unchecked("urn:oxigraph:undoable"),
        Literal::new_simple_literal(entry.undoable.to_string()).as_ref(),
        graph.as_ref(),
    ));
    txn.insert(QuadRef::new(
        subj.as_ref(),
        NamedNodeRef::new_unchecked("urn:oxigraph:insertCount"),
        Literal::new_simple_literal(entry.inserted.len().to_string()).as_ref(),
        graph.as_ref(),
    ));
    txn.insert(QuadRef::new(
        subj.as_ref(),
        NamedNodeRef::new_unchecked("urn:oxigraph:removeCount"),
        Literal::new_simple_literal(entry.removed.len().to_string()).as_ref(),
        graph.as_ref(),
    ));

    if !entry.inserted.is_empty() {
        let nq = quads_to_nquads(&entry.inserted);
        txn.insert(QuadRef::new(
            subj.as_ref(),
            NamedNodeRef::new_unchecked("urn:oxigraph:insertedNQuads"),
            Literal::new_simple_literal(&nq).as_ref(),
            graph.as_ref(),
        ));
    }

    if !entry.removed.is_empty() {
        let nq = quads_to_nquads(&entry.removed);
        txn.insert(QuadRef::new(
            subj.as_ref(),
            NamedNodeRef::new_unchecked("urn:oxigraph:removedNQuads"),
            Literal::new_simple_literal(&nq).as_ref(),
            graph.as_ref(),
        ));
    }

    txn.commit()
        .map_err(|e| ChangelogError::Internal(e.to_string()))?;
    Ok(())
}

fn quads_to_nquads(quads: &[Quad]) -> String {
    let mut buf = Vec::new();
    {
        let mut ser = RdfSerializer::from_format(RdfFormat::NQuads).for_writer(&mut buf);
        for quad in quads {
            drop(ser.serialize_quad(quad));
        }
        drop(ser.finish());
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn parse_nquads(nq: &str) -> Vec<Quad> {
    if nq.is_empty() {
        return Vec::new();
    }
    RdfParser::from_format(RdfFormat::NQuads)
        .for_slice(nq)
        .flatten()
        .collect()
}

pub fn entry_to_list_json(entry: &ChangelogEntry) -> String {
    format!(
        "{{\"id\":{},\"timestamp\":\"{}\",\"operation\":\"{}\",\"undoable\":{}}}",
        entry.id,
        escape_json(&entry.timestamp),
        escape_json(&entry.operation),
        entry.undoable,
    )
}

pub fn entry_to_detail_json(entry: &ChangelogEntry) -> String {
    let inserted_nq = quads_to_nquads(&entry.inserted);
    let removed_nq = quads_to_nquads(&entry.removed);
    format!(
        "{{\"id\":{},\"timestamp\":\"{}\",\"operation\":\"{}\",\"undoable\":{},\"insertCount\":{},\"removeCount\":{},\"inserted\":\"{}\",\"removed\":\"{}\"}}",
        entry.id,
        escape_json(&entry.timestamp),
        escape_json(&entry.operation),
        entry.undoable,
        entry.inserted.len(),
        entry.removed.len(),
        escape_json(&inserted_nq),
        escape_json(&removed_nq),
    )
}

fn format_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86_400;
    let time_of_day = secs % 86_400;
    let hours = time_of_day / 3_600;
    let minutes = (time_of_day % 3_600) / 60;
    let seconds = time_of_day % 60;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[derive(Debug)]
pub enum ChangelogError {
    Disabled,
    NotFound,
    NotUndoable,
    Internal(String),
}

impl std::fmt::Display for ChangelogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "Changelog is not enabled (use --changelog to enable)"),
            Self::NotFound => write!(f, "Changelog entry not found"),
            Self::NotUndoable => write!(f, "Changelog entry is not undoable"),
            Self::Internal(e) => write!(f, "Internal changelog error: {e}"),
        }
    }
}
