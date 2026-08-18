use crate::parser::Message;
use crate::query::{matches_text, CompiledQuery};

#[derive(Debug)]
pub struct Match {
    pub message_index: usize,
}

/// Concatenate the lowercased text of messages, optionally restricted to a role.
fn session_text(messages: &[Message], role: &Option<String>) -> String {
    messages
        .iter()
        .filter(|m| role.as_ref().is_none_or(|r| &m.role == r))
        .map(|m| m.text.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Result of evaluating a query against one session.
pub struct SessionEval {
    /// Message indices (role-filtered) that contain a positive phrase, or the
    /// first role-eligible message as an anchor for pure-NOT/filter queries.
    pub matches: Vec<Match>,
    /// Indices into `query.positive_phrases` that appear in the session. The
    /// caller weights these by inverse document frequency to rank by relevance
    /// (rare terms count more) without a session-length bias.
    pub matched_terms: Vec<usize>,
}

/// Session-scope evaluation in a single pass over the (role-filtered) text.
pub fn evaluate(messages: &[Message], query: &CompiledQuery) -> SessionEval {
    let role = &query.filters.role;
    let text = session_text(messages, role);
    if !matches_text(query, &text) {
        return SessionEval {
            matches: Vec::new(),
            matched_terms: Vec::new(),
        };
    }

    let matched_terms: Vec<usize> = query
        .positive_phrases
        .iter()
        .enumerate()
        .filter(|(_, p)| text.contains(p.as_str()))
        .map(|(i, _)| i)
        .collect();

    let mut matches = Vec::new();
    for (i, m) in messages.iter().enumerate() {
        if let Some(r) = role {
            if &m.role != r {
                continue;
            }
        }
        let lower = m.text.to_lowercase();
        if query.positive_phrases.iter().any(|p| lower.contains(p)) {
            matches.push(Match { message_index: i });
        }
    }
    if matches.is_empty() {
        if let Some((i, _)) = messages
            .iter()
            .enumerate()
            .find(|(_, m)| role.as_ref().is_none_or(|r| &m.role == r))
        {
            matches.push(Match { message_index: i });
        }
    }
    SessionEval {
        matches,
        matched_terms,
    }
}

/// Session-scope match indices only (thin wrapper over [`evaluate`]). Test-only.
#[cfg(test)]
fn find_matches(messages: &[Message], query: &CompiledQuery) -> Vec<Match> {
    evaluate(messages, query).matches
}

pub fn context_range(match_idx: usize, context: usize, total: usize) -> (usize, usize) {
    let start = match_idx.saturating_sub(context);
    let end = (match_idx + context + 1).min(total);
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Message;
    use crate::query::compile;

    fn msg(role: &str, text: &str) -> Message {
        Message {
            role: role.to_string(),
            text: text.to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            session_id: "test".to_string(),
        }
    }

    #[test]
    fn test_matches_across_distant_messages() {
        let mut msgs = vec![msg("user", "question about acl")];
        for i in 0..10 {
            msgs.push(msg("assistant", &format!("filler {i}")));
        }
        msgs.push(msg("assistant", "the jellyfin permission failure"));
        let q = compile("acl AND jellyfin");
        let matches = find_matches(&msgs, &q);
        assert!(
            !matches.is_empty(),
            "session-scope AND must match terms any distance apart"
        );
    }

    #[test]
    fn test_no_match_returns_empty() {
        let msgs = vec![msg("user", "hello world")];
        let q = compile("nonexistent");
        assert!(find_matches(&msgs, &q).is_empty());
    }

    #[test]
    fn test_role_filter_restricts_match() {
        let msgs = vec![
            msg("user", "tell me about sandbox"),
            msg("assistant", "sandbox uses excludedCommands"),
        ];
        let q = compile("role:user excludedCommands");
        assert!(find_matches(&msgs, &q).is_empty());
        let q2 = compile("role:assistant excludedCommands");
        assert!(!find_matches(&msgs, &q2).is_empty());
    }

    #[test]
    fn test_match_points_are_positive_messages() {
        let msgs = vec![
            msg("user", "unrelated intro"),
            msg("user", "the acl thing"),
            msg("user", "closing"),
        ];
        let q = compile("acl");
        let matches = find_matches(&msgs, &q);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].message_index, 1);
    }

    #[test]
    fn test_matched_terms_are_distinct_present_terms() {
        let msgs = vec![msg(
            "user",
            "an acl and permission discussion, no third term",
        )];
        let q = compile("acl permission jellyfin"); // OR default
        let eval = evaluate(&msgs, &q);
        assert_eq!(
            eval.matched_terms.len(),
            2,
            "acl + permission present, jellyfin absent"
        );
        assert!(!eval.matches.is_empty());
    }

    #[test]
    fn test_matched_terms_empty_when_no_positive_phrases() {
        let msgs = vec![msg("user", "anything")];
        let q = compile("role:user"); // no residual terms => empty positive_phrases
        let eval = evaluate(&msgs, &q);
        assert!(eval.matched_terms.is_empty());
    }

    #[test]
    fn test_context_range_middle() {
        assert_eq!(context_range(5, 3, 10), (2, 9));
    }
    #[test]
    fn test_context_range_start() {
        assert_eq!(context_range(1, 3, 10), (0, 5));
    }
    #[test]
    fn test_context_range_end() {
        assert_eq!(context_range(9, 3, 10), (6, 10));
    }
}
