use crate::parser::Message;
use crate::search;
use serde::Serialize;

#[derive(Serialize)]
pub struct SearchOutput {
    pub query: String,
    pub sessions_searched: usize,
    pub scope_reached: String,
    pub hit_count: usize,
    pub mode: String,
    pub results: Vec<SessionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct SessionResult {
    pub session_id: String,
    pub project_dir: String,
    pub first_match_timestamp: String,
    pub match_context: Vec<ContextMessage>,
}

#[derive(Serialize)]
pub struct ContextMessage {
    pub role: String,
    pub text: String,
    pub timestamp: String,
    pub matched: bool,
}

pub fn build_session_result(
    messages: &[Message],
    matches: &[search::Match],
    context_size: usize,
    project_dir: &str,
    max_results: usize,
) -> Vec<SessionResult> {
    let matched_indices: std::collections::HashSet<usize> =
        matches.iter().map(|m| m.message_index).collect();
    let mut seen_ranges: Vec<(usize, usize)> = Vec::new();
    let mut results = Vec::new();

    for m in matches {
        if results.len() >= max_results {
            break;
        }
        let (start, end) = search::context_range(m.message_index, context_size, messages.len());
        if seen_ranges.iter().any(|(s, e)| start >= *s && end <= *e) {
            continue;
        }
        seen_ranges.push((start, end));

        let session_id = messages[m.message_index].session_id.clone();
        let first_match_timestamp = messages[m.message_index].timestamp.clone();
        let match_context: Vec<ContextMessage> = (start..end)
            .map(|i| ContextMessage {
                role: messages[i].role.clone(),
                text: messages[i].text.clone(),
                timestamp: messages[i].timestamp.clone(),
                matched: matched_indices.contains(&i),
            })
            .collect();

        results.push(SessionResult {
            session_id,
            project_dir: project_dir.to_string(),
            first_match_timestamp,
            match_context,
        });
    }
    results
}

/// Per-message text is truncated to this many chars in the text view. Keeps the
/// whole result set compact enough that the agent reads it inline instead of
/// reaching for grep/python on a spilled tool-result file. JSON output is full.
const TEXT_MSG_CAP: usize = 800;

fn truncate(s: &str, cap: usize) -> String {
    // Collapse to a tidy block: trim, then cap on a char boundary with a marker.
    let t = s.trim();
    if t.chars().count() <= cap {
        return t.to_string();
    }
    let cut: String = t.chars().take(cap).collect();
    format!("{cut}… [+{} chars]", t.chars().count() - cap)
}

impl SearchOutput {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Compact, human-readable rendering. This is the default the agent consumes:
    /// complete enough to synthesise from directly, small enough not to spill.
    pub fn to_text(&self) -> String {
        let mut o = String::new();
        if let Some(err) = &self.error {
            o.push_str(&format!("rrecall error: {err}\nquery: {}\n", self.query));
            return o;
        }
        o.push_str(&format!(
            "rrecall · mode={} · scope_reached={} · sessions_searched={} · hits={}\nquery: {}\n",
            self.mode, self.scope_reached, self.sessions_searched, self.hit_count, self.query
        ));
        if self.results.is_empty() {
            o.push_str(
                "\nNo matches. The scope ladder already reached the reported scope — \
                 this is a query/ranking miss, not an index gap. Retry with different \
                 vocabulary, pin a rare AND-term, or add an after:/before: date range.\n",
            );
            return o;
        }
        for (i, r) in self.results.iter().enumerate() {
            o.push_str(&format!(
                "\n[{}] {} · {} · {}\n",
                i + 1,
                r.session_id,
                r.project_dir,
                r.first_match_timestamp
            ));
            for m in &r.match_context {
                let marker = if m.matched { "»" } else { " " };
                let body = truncate(&m.text, TEXT_MSG_CAP);
                // Indent continuation lines so the role column stays readable.
                let body = body.replace('\n', "\n        ");
                o.push_str(&format!("  {marker} {:<9} {body}\n", m.role));
            }
        }
        o.push_str(&format!(
            "\n(scope_reached={}, sessions_searched={} — if the session you want isn't here, \
             refine the query; do not search by other means.)\n",
            self.scope_reached, self.sessions_searched
        ));
        o
    }

    pub fn error(query: &str, msg: &str) -> Self {
        SearchOutput {
            query: query.to_string(),
            sessions_searched: 0,
            scope_reached: "none".to_string(),
            hit_count: 0,
            mode: "none".into(),
            results: vec![],
            error: Some(msg.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Message;
    use crate::search::Match;

    fn msg(role: &str, text: &str, ts: &str) -> Message {
        Message {
            role: role.to_string(),
            text: text.to_string(),
            timestamp: ts.to_string(),
            session_id: "sess-1".to_string(),
        }
    }

    #[test]
    fn test_build_session_result() {
        let msgs = vec![
            msg("user", "before", "2026-01-01T00:00:00Z"),
            msg("assistant", "also before", "2026-01-01T00:01:00Z"),
            msg("user", "auth middleware question", "2026-01-01T00:02:00Z"),
            msg("assistant", "auth answer", "2026-01-01T00:03:00Z"),
            msg("user", "after", "2026-01-01T00:04:00Z"),
        ];
        let matches = vec![Match { message_index: 2 }];
        let results = build_session_result(&msgs, &matches, 1, "-test-project", 20);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "sess-1");
        assert_eq!(results[0].match_context.len(), 3);
        assert!(!results[0].match_context[0].matched);
        assert!(results[0].match_context[1].matched);
        assert!(!results[0].match_context[2].matched);
    }

    #[test]
    fn test_error_output() {
        let output = SearchOutput::error("test", "something went wrong");
        let json = output.to_json();
        assert!(json.contains("something went wrong"));
        assert!(json.contains("\"sessions_searched\": 0"));
        assert!(json.contains("\"mode\": \"none\""));
    }

    #[test]
    fn test_output_reports_scope_and_hit_count() {
        let out = SearchOutput {
            query: "acl".to_string(),
            sessions_searched: 3900,
            scope_reached: "all_projects".to_string(),
            hit_count: 2,
            mode: "lexical".into(),
            results: vec![],
            error: None,
        };
        let json = out.to_json();
        assert!(json.contains("\"scope_reached\": \"all_projects\""));
        assert!(json.contains("\"hit_count\": 2"));
        assert!(json.contains("\"mode\": \"lexical\""));
    }

    #[test]
    fn test_deduplication() {
        let msgs = vec![
            msg("user", "a", "t0"),
            msg("user", "b auth", "t1"),
            msg("user", "c auth", "t2"),
            msg("user", "d", "t3"),
        ];
        let matches = vec![Match { message_index: 1 }, Match { message_index: 2 }];
        let results = build_session_result(&msgs, &matches, 1, "-proj", 20);
        assert_eq!(results.len(), 2);
    }

    fn sample_output() -> SearchOutput {
        SearchOutput {
            query: "acl permission".into(),
            sessions_searched: 738,
            scope_reached: "all_projects".into(),
            hit_count: 1,
            mode: "hybrid".into(),
            results: vec![SessionResult {
                session_id: "960b089f".into(),
                project_dir: "-Users-alice-ws-alpha-sh".into(),
                first_match_timestamp: "2026-06-03T02:16:29Z".into(),
                match_context: vec![
                    ContextMessage {
                        role: "user".into(),
                        text: "what about acls".into(),
                        timestamp: "t".into(),
                        matched: false,
                    },
                    ContextMessage {
                        role: "assistant".into(),
                        text: "the permission model".into(),
                        timestamp: "t".into(),
                        matched: true,
                    },
                ],
            }],
            error: None,
        }
    }

    #[test]
    fn test_to_text_is_readable_not_json() {
        let t = sample_output().to_text();
        assert!(t.contains("mode=hybrid"));
        assert!(t.contains("sessions_searched=738"));
        assert!(t.contains("[1] 960b089f"));
        assert!(t.contains("» assistant")); // matched line marked
        assert!(!t.contains('{')); // not JSON
    }

    #[test]
    fn test_to_text_empty_reports_coverage() {
        let mut out = sample_output();
        out.results.clear();
        out.hit_count = 0;
        let t = out.to_text();
        assert!(t.contains("No matches"));
        assert!(t.contains("sessions_searched=738"));
    }

    #[test]
    fn test_to_text_error() {
        let t = SearchOutput::error("q", "boom").to_text();
        assert!(t.contains("rrecall error: boom"));
    }

    #[test]
    fn test_truncate_marks_overflow() {
        let long = "x".repeat(900);
        let out = truncate(&long, 800);
        assert!(out.contains("… [+100 chars]"));
        assert_eq!(truncate("short", 800), "short");
    }
}
