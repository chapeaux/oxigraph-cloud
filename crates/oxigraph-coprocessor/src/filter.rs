//! Filter expression evaluation on encoded key bytes.

use crate::encoded_term_len;

#[derive(Clone, Debug)]
pub enum FilterPredicate {
    TermEquals { position: usize, value: Vec<u8> },
    TermTypeEquals { position: usize, type_byte: u8 },
    And(Vec<FilterPredicate>),
    Or(Vec<FilterPredicate>),
}

pub fn evaluate_filter(key: &[u8], predicate: &FilterPredicate) -> bool {
    match predicate {
        FilterPredicate::TermEquals { position, value } => {
            extract_term_at(key, *position).map_or(false, |t| t == value.as_slice())
        }
        FilterPredicate::TermTypeEquals { position, type_byte } => {
            extract_term_at(key, *position).map_or(false, |t| !t.is_empty() && t[0] == *type_byte)
        }
        FilterPredicate::And(preds) => preds.iter().all(|p| evaluate_filter(key, p)),
        FilterPredicate::Or(preds) => preds.iter().any(|p| evaluate_filter(key, p)),
    }
}

fn extract_term_at(key: &[u8], position: usize) -> Option<&[u8]> {
    let mut offset = 0;
    for i in 0..=position {
        if offset >= key.len() { return None; }
        let len = encoded_term_len(&key[offset..]).ok()?;
        if i == position { return Some(&key[offset..offset + len]); }
        offset += len;
    }
    None
}

pub fn filter_scan_results<'a>(
    pairs: impl Iterator<Item = (&'a [u8], &'a [u8])>,
    predicate: &FilterPredicate,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    pairs
        .filter(|(key, _)| {
            if key.is_empty() { return false; }
            evaluate_filter(&key[1..], predicate)
        })
        .map(|(k, v)| (k.to_vec(), v.to_vec()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_term_type_equals() {
        let mut key = vec![1u8]; // NamedNode type
        key.extend_from_slice(&[0xAA; 16]);
        let pred = FilterPredicate::TermTypeEquals { position: 0, type_byte: 1 };
        assert!(evaluate_filter(&key, &pred));
        let pred_wrong = FilterPredicate::TermTypeEquals { position: 0, type_byte: 8 };
        assert!(!evaluate_filter(&key, &pred_wrong));
    }

    #[test]
    fn test_and_filter() {
        let mut key = Vec::new();
        key.push(1); key.extend_from_slice(&[0xAA; 16]); // NamedNode
        key.push(28); // BooleanLiteral true
        let pred = FilterPredicate::And(vec![
            FilterPredicate::TermTypeEquals { position: 0, type_byte: 1 },
            FilterPredicate::TermTypeEquals { position: 1, type_byte: 28 },
        ]);
        assert!(evaluate_filter(&key, &pred));
    }
}
