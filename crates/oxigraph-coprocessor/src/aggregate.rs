//! Aggregate pushdown: COUNT, MIN, MAX.

#[derive(Debug, Clone)]
pub struct CountResult {
    pub count: u64,
    pub scanned_keys: u64,
}

#[derive(Debug, Clone)]
pub struct MinMaxResult {
    pub min_key: Option<Vec<u8>>,
    pub max_key: Option<Vec<u8>>,
    pub scanned_keys: u64,
}

pub fn execute_count<'a>(
    table_prefix: u8,
    key_prefix: &[u8],
    pairs: impl Iterator<Item = (&'a [u8], &'a [u8])>,
) -> CountResult {
    let full_prefix = {
        let mut p = vec![table_prefix];
        p.extend_from_slice(key_prefix);
        p
    };
    let mut result = CountResult {
        count: 0,
        scanned_keys: 0,
    };
    for (key, _) in pairs {
        if !key.starts_with(&full_prefix) {
            continue;
        }
        result.scanned_keys += 1;
        result.count += 1;
    }
    result
}

pub fn execute_min_max<'a>(
    table_prefix: u8,
    key_prefix: &[u8],
    pairs: impl Iterator<Item = (&'a [u8], &'a [u8])>,
) -> MinMaxResult {
    let full_prefix = {
        let mut p = vec![table_prefix];
        p.extend_from_slice(key_prefix);
        p
    };
    let mut result = MinMaxResult {
        min_key: None,
        max_key: None,
        scanned_keys: 0,
    };
    for (key, _) in pairs {
        if !key.starts_with(&full_prefix) {
            continue;
        }
        result.scanned_keys += 1;
        if result.min_key.as_deref().is_none_or(|min| key < min) {
            result.min_key = Some(key.to_vec());
        }
        if result.max_key.as_deref().is_none_or(|max| key > max) {
            result.max_key = Some(key.to_vec());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count() {
        let data: Vec<(Vec<u8>, Vec<u8>)> = vec![
            (vec![0x02, 1, 2], b"a".to_vec()),
            (vec![0x02, 1, 3], b"b".to_vec()),
            (vec![0x03, 1, 1], b"d".to_vec()),
        ];
        let result = execute_count(
            0x02,
            &[1],
            data.iter().map(|(k, v)| (k.as_slice(), v.as_slice())),
        );
        assert_eq!(result.count, 2);
    }

    #[test]
    fn test_min_max() {
        let data: Vec<(Vec<u8>, Vec<u8>)> = vec![
            (vec![0x02, 5], b"a".to_vec()),
            (vec![0x02, 1], b"b".to_vec()),
            (vec![0x02, 9], b"c".to_vec()),
        ];
        let result = execute_min_max(
            0x02,
            &[],
            data.iter().map(|(k, v)| (k.as_slice(), v.as_slice())),
        );
        assert_eq!(result.min_key, Some(vec![0x02, 1]));
        assert_eq!(result.max_key, Some(vec![0x02, 9]));
    }
}
