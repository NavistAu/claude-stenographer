use std::collections::HashMap;

/// Score-aware weighted fusion of ranked, scored session lists.
///
/// Each input is `(items, weight)` where `items` are `(session_id, raw_score)`
/// sorted best-first. Per list, scores are min-max normalized to [0,1], then
/// accumulated with the list's weight; a session absent from a list contributes
/// 0 there. Returns ids best-first.
///
/// Why not rank-only RRF: RRF scores `Σ 1/(k+rank)`, so a high-confidence hit
/// present in only ONE list (e.g. a dense semantic rescue with no lexical term)
/// gets a single small term and is outranked by mediocre sessions that appear in
/// BOTH lists — it buries exactly the rescues we want. Here a strong normalized
/// score carries through even from one list, while agreement across lists still
/// accumulates the most.
pub fn weighted_fuse(lists: &[(Vec<(String, f64)>, f64)]) -> Vec<String> {
    let mut combined: HashMap<String, f64> = HashMap::new();
    for (items, weight) in lists {
        if items.is_empty() {
            continue;
        }
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for (_, s) in items {
            lo = lo.min(*s);
            hi = hi.max(*s);
        }
        let range = (hi - lo).max(1e-9);
        for (id, s) in items {
            let norm = (s - lo) / range;
            *combined.entry(id.clone()).or_insert(0.0) += weight * norm;
        }
    }
    let mut out: Vec<(String, f64)> = combined.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out.into_iter().map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_list_high_score_is_not_buried() {
        // The RRF failure mode: a high-confidence item present in only the dense
        // list must still rank well, not be buried under both-list mediocrities.
        let lex = vec![("a".to_string(), 1.0), ("b".to_string(), 0.9)];
        let dense = vec![("z".to_string(), 0.95), ("a".to_string(), 0.2)];
        let fused = weighted_fuse(&[(lex, 1.0), (dense, 1.0)]);
        let pz = fused.iter().position(|x| x == "z").unwrap();
        let pb = fused.iter().position(|x| x == "b").unwrap();
        assert!(
            pz < pb,
            "dense-only high-confidence hit must outrank a mid lexical-only one"
        );
    }

    #[test]
    fn test_agreement_across_lists_wins() {
        let lex = vec![("a".to_string(), 1.0), ("b".to_string(), 0.5)];
        let dense = vec![("a".to_string(), 1.0), ("c".to_string(), 0.5)];
        let fused = weighted_fuse(&[(lex, 1.0), (dense, 1.0)]);
        assert_eq!(
            fused[0], "a",
            "a is strong in both lists -> should rank first"
        );
    }

    #[test]
    fn test_empty_and_single() {
        assert!(weighted_fuse(&[]).is_empty());
        let only = vec![("x".to_string(), 0.5), ("y".to_string(), 0.1)];
        assert_eq!(
            weighted_fuse(&[(only, 1.0)]),
            vec!["x".to_string(), "y".to_string()]
        );
    }
}
