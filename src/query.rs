use regex::Regex;
use tantivy_query_grammar::{parse_query, Occur, UserInputAst, UserInputLeaf};

#[derive(Debug, Default, PartialEq)]
pub struct Filters {
    pub role: Option<String>,    // "user" | "assistant"
    pub project: Option<String>, // substring of the project dir name
    pub after: Option<String>,   // ISO date "YYYY-MM-DD"
    pub before: Option<String>,  // ISO date "YYYY-MM-DD"
}

/// Strip `role:`/`project:`/`after:`/`before:` tokens from `raw`, returning
/// the extracted filters and the residual query (Lucene boolean expression).
pub fn extract_filters(raw: &str) -> (Filters, String) {
    let mut filters = Filters::default();
    let re = Regex::new(r"(?i)\b(role|project|after|before):([-\w./@]+)").unwrap();
    let residual = re.replace_all(raw, |caps: &regex::Captures| {
        let key = caps[1].to_lowercase();
        let val = caps[2].to_string();
        match key.as_str() {
            "role" => filters.role = Some(val.to_lowercase()),
            "project" => filters.project = Some(val),
            "after" => filters.after = Some(val),
            "before" => filters.before = Some(val),
            _ => {}
        }
        String::new()
    });
    (filters, residual.trim().to_string())
}

/// A parsed, ready-to-evaluate query.
pub struct CompiledQuery {
    pub filters: Filters,
    ast: Option<UserInputAst>,         // None when the residual is empty
    pub positive_phrases: Vec<String>, // lowercased MUST/SHOULD literal phrases
}

pub fn compile(raw: &str) -> CompiledQuery {
    let (filters, residual) = extract_filters(raw);
    let ast = if residual.is_empty() {
        None
    } else {
        parse_query(&residual).ok()
    };
    let mut positive_phrases = Vec::new();
    if let Some(a) = &ast {
        collect_positive(a, false, &mut positive_phrases);
    }
    CompiledQuery {
        filters,
        ast,
        positive_phrases,
    }
}

fn collect_positive(ast: &UserInputAst, negated: bool, out: &mut Vec<String>) {
    match ast {
        UserInputAst::Leaf(leaf) => {
            if !negated {
                if let UserInputLeaf::Literal(lit) = leaf.as_ref() {
                    out.push(lit.phrase.to_lowercase());
                }
            }
        }
        UserInputAst::Boost(inner, _) => collect_positive(inner, negated, out),
        UserInputAst::Clause(children) => {
            for (occur, child) in children {
                let child_negated = matches!(occur, Some(Occur::MustNot));
                collect_positive(child, negated || child_negated, out);
            }
        }
    }
}

/// True if `text_lower` (already lowercased) satisfies the query.
/// An empty AST matches everything (filters still apply upstream).
pub fn matches_text(q: &CompiledQuery, text_lower: &str) -> bool {
    match &q.ast {
        None => true,
        Some(ast) => eval(ast, text_lower),
    }
}

fn eval(ast: &UserInputAst, text_lower: &str) -> bool {
    match ast {
        UserInputAst::Leaf(leaf) => eval_leaf(leaf, text_lower),
        UserInputAst::Boost(inner, _) => eval(inner, text_lower),
        UserInputAst::Clause(children) => {
            let mut has_must = false;
            let mut all_must = true;
            let mut has_should = false;
            let mut any_should = false;
            let mut excluded = false;
            for (occur, child) in children {
                let m = eval(child, text_lower);
                match occur.unwrap_or(Occur::Should) {
                    Occur::Must => {
                        has_must = true;
                        if !m {
                            all_must = false;
                        }
                    }
                    Occur::MustNot => {
                        if m {
                            excluded = true;
                        }
                    }
                    Occur::Should => {
                        has_should = true;
                        if m {
                            any_should = true;
                        }
                    }
                }
            }
            if excluded {
                return false;
            }
            if has_must {
                all_must
            } else if has_should {
                any_should
            } else {
                true
            }
        }
    }
}

fn eval_leaf(leaf: &UserInputLeaf, text_lower: &str) -> bool {
    match leaf {
        UserInputLeaf::Literal(lit) => text_lower.contains(&lit.phrase.to_lowercase()),
        UserInputLeaf::All => true,
        // NOTE: recompiles the pattern per session. Only reached for explicit
        // /regex/ query terms (rare; the agent uses literal OR terms), and is
        // dominated by the per-session JSON parse cost. Precompile into
        // CompiledQuery if regex queries ever become common (Phase 2 candidate).
        UserInputLeaf::Regex { pattern, .. } => Regex::new(&format!("(?i){pattern}"))
            .map(|r| r.is_match(text_lower))
            .unwrap_or(false),
        // Range/Set/Exists are not used in Phase 1 (date/project are pre-extracted).
        _ => false,
    }
}

#[cfg(test)]
mod eval_tests {
    use super::*;

    #[test]
    fn test_or_recall_friendly_default() {
        let q = compile("acl import permission");
        assert!(matches_text(&q, "discussion about acl inheritance"));
    }

    #[test]
    fn test_explicit_and_requires_all() {
        let q = compile("acl AND jellyfin");
        assert!(matches_text(&q, "acl problem in jellyfin playback"));
        assert!(!matches_text(&q, "acl problem only"));
    }

    #[test]
    fn test_not_excludes() {
        let q = compile("acl NOT smb");
        assert!(matches_text(&q, "acl on nfs"));
        assert!(!matches_text(&q, "acl on smb share"));
    }

    #[test]
    fn test_phrase() {
        let q = compile("\"finished downloads\"");
        assert!(matches_text(&q, "moving finished downloads into place"));
        assert!(!matches_text(&q, "finished the downloads later"));
    }

    #[test]
    fn test_positive_phrases_exclude_negated() {
        let q = compile("acl NOT smb");
        assert!(q.positive_phrases.contains(&"acl".to_string()));
        assert!(!q.positive_phrases.contains(&"smb".to_string()));
    }

    #[test]
    fn test_empty_residual_matches_all() {
        let q = compile("role:user");
        assert!(matches_text(&q, "anything at all"));
        assert_eq!(q.filters.role, Some("user".to_string()));
    }

    #[test]
    fn test_malformed_query_matches_all() {
        // A query that fails to parse yields ast=None, which matches everything
        // (filters still apply upstream) and contributes no positive phrases.
        let q = compile("\"");
        assert!(matches_text(&q, "anything at all"));
        assert!(q.positive_phrases.is_empty());
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    #[test]
    fn test_extract_role_and_residual() {
        let (f, residual) = extract_filters("role:user acl OR permission");
        assert_eq!(f.role, Some("user".to_string()));
        assert_eq!(residual, "acl OR permission");
    }

    #[test]
    fn test_extract_all_fields() {
        let (f, residual) =
            extract_filters("project:alpha.sh after:2026-05-15 before:2026-05-20 jellyfin");
        assert_eq!(f.project, Some("alpha.sh".to_string()));
        assert_eq!(f.after, Some("2026-05-15".to_string()));
        assert_eq!(f.before, Some("2026-05-20".to_string()));
        assert_eq!(residual, "jellyfin");
    }

    #[test]
    fn test_no_fields() {
        let (f, residual) = extract_filters("acl import permission");
        assert_eq!(f, Filters::default());
        assert_eq!(residual, "acl import permission");
    }

    #[test]
    fn test_value_stops_at_paren() {
        let (f, _residual) = extract_filters("(role:user) acl");
        assert_eq!(
            f.role,
            Some("user".to_string()),
            "value must not capture the closing paren"
        );
    }

    #[test]
    fn test_duplicate_key_last_wins() {
        // Documented behaviour: the last occurrence wins.
        let (f, _residual) = extract_filters("role:user role:assistant foo");
        assert_eq!(f.role, Some("assistant".to_string()));
    }
}
