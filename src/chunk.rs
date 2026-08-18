use crate::parser::Message;

/// A small window of a session, the unit that gets embedded.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub text: String,
    pub offset: usize,
    pub timestamp: String,
}

/// Window size in messages.
pub const WINDOW: usize = 3;
/// Step between window starts (overlap = WINDOW - STEP).
pub const STEP: usize = 2;

/// Split a session into overlapping windows of `WINDOW` messages.
pub fn chunk_session(messages: &[Message]) -> Vec<Chunk> {
    if messages.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        let end = (i + WINDOW).min(messages.len());
        let text = messages[i..end]
            .iter()
            .map(|m| format!("{}: {}", m.role, m.text))
            .collect::<Vec<_>>()
            .join("\n");
        chunks.push(Chunk {
            text,
            offset: i,
            timestamp: messages[i].timestamp.clone(),
        });
        if end == messages.len() {
            break;
        }
        i += STEP;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Message;
    fn m(r: &str, t: &str) -> Message {
        Message {
            role: r.into(),
            text: t.into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            session_id: "s".into(),
        }
    }
    #[test]
    fn test_chunks_cover_and_overlap() {
        let msgs: Vec<Message> = (0..5).map(|i| m("user", &format!("line {i}"))).collect();
        let cs = chunk_session(&msgs);
        assert!(!cs.is_empty());
        assert_eq!(cs[0].offset, 0);
        assert!(cs[0].text.contains("line 0") && cs[0].text.contains("line 2"));
        assert!(cs.last().unwrap().text.contains("line 4"));
    }
    #[test]
    fn test_empty_session() {
        assert!(chunk_session(&[]).is_empty());
    }
}
