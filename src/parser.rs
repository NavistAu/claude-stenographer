use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub text: String,
    pub timestamp: String,
    pub session_id: String,
}

pub fn parent_session_id(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    if parent.file_name()?.to_str()? != "subagents" {
        return None;
    }
    let grandparent = parent.parent()?;
    Some(grandparent.file_name()?.to_str()?.to_string())
}

pub fn parse_session(path: &Path) -> (Vec<Message>, usize) {
    let override_id = parent_session_id(path);
    parse_session_inner(path, override_id.as_deref())
}

fn parse_session_inner(path: &Path, override_session_id: Option<&str>) -> (Vec<Message>, usize) {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return (vec![], 0),
    };
    let reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut malformed = 0;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => {
                malformed += 1;
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let val: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                malformed += 1;
                continue;
            }
        };

        let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if msg_type != "user" && msg_type != "assistant" {
            continue;
        }

        let session_id = override_session_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                val.get("sessionId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            });
        let timestamp = val
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let message = match val.get("message") {
            Some(m) => m,
            None => continue,
        };

        let text = extract_text(message, msg_type);
        if text.is_empty() {
            continue;
        }

        messages.push(Message {
            role: msg_type.to_string(),
            text,
            timestamp,
            session_id,
        });
    }
    (messages, malformed)
}

fn extract_text(message: &Value, msg_type: &str) -> String {
    let content = match message.get("content") {
        Some(c) => c,
        None => return String::new(),
    };
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    let blocks = match content.as_array() {
        Some(arr) => arr,
        None => return String::new(),
    };
    let mut parts = Vec::new();
    for block in blocks {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    parts.push(text.to_string());
                }
            }
            "tool_use" if msg_type == "assistant" => {
                if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
                    parts.push(format!("[tool: {name}]"));
                }
            }
            _ => {}
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn test_parse_simple_session() {
        let (msgs, malformed) = parse_session(&fixture_path("simple_session.jsonl"));
        assert_eq!(malformed, 0);
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, "user");
        assert!(msgs[0].text.contains("auth middleware"));
        assert_eq!(msgs[1].role, "assistant");
        assert!(msgs[1].text.contains("rewrite the auth middleware"));
        assert!(msgs[1].text.contains("[tool: Read]"));
        assert!(!msgs[1].text.contains("Let me think"));
        assert_eq!(msgs[2].role, "user");
        assert!(msgs[2].text.contains("rewrite approach"));
        assert!(!msgs[2].text.contains("file contents"));
        assert_eq!(msgs[3].role, "assistant");
        assert!(msgs[3].text.contains("token storage migration"));
    }

    #[test]
    fn test_parse_nonexistent_file() {
        let (msgs, malformed) = parse_session(Path::new("/nonexistent/file.jsonl"));
        assert!(msgs.is_empty());
        assert_eq!(malformed, 0);
    }

    #[test]
    fn test_session_ids_preserved() {
        let (msgs, _) = parse_session(&fixture_path("simple_session.jsonl"));
        for msg in &msgs {
            assert_eq!(msg.session_id, "test-session-1");
        }
    }

    #[test]
    fn test_parent_session_id_detection() {
        assert_eq!(
            parent_session_id(Path::new(
                "/projects/-Users-test/abc-123/subagents/agent-xyz.jsonl"
            )),
            Some("abc-123".to_string())
        );
        assert_eq!(
            parent_session_id(Path::new("/projects/-Users-test/session.jsonl")),
            None
        );
    }

    #[test]
    fn test_subagent_logs_attribute_to_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let subagent_dir = tmp.path().join("parent-sess-id").join("subagents");
        std::fs::create_dir_all(&subagent_dir).unwrap();
        let subagent_file = subagent_dir.join("agent-abc.jsonl");
        std::fs::write(&subagent_file,
            r#"{"type":"user","sessionId":"subagent-internal-id","timestamp":"2026-01-01T00:00:00Z","message":{"role":"user","content":"test message"},"uuid":"u1"}"#
        ).unwrap();
        let (msgs, _) = parse_session(&subagent_file);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].session_id, "parent-sess-id");
    }
}
