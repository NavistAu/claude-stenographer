use std::path::{Path, PathBuf};
use std::process::Command;

/// Build the rg argument vector for listing files that contain ANY phrase.
/// `-l` list files, `-i` case-insensitive, `-F` fixed-string (phrases are literal),
/// one `-e <phrase>` per phrase (OR semantics), then the search roots.
pub fn rg_args(phrases: &[String], roots: &[PathBuf]) -> Vec<String> {
    let mut args = vec![
        "-l".to_string(),
        "-i".to_string(),
        "-F".to_string(),
        "--glob".to_string(),
        "*.jsonl".to_string(),
    ];
    for p in phrases {
        args.push("-e".to_string());
        args.push(p.clone());
    }
    for r in roots {
        args.push(r.to_string_lossy().to_string());
    }
    args
}

/// Return candidate files via rg, or None if rg can't be used (caller falls back).
pub fn rg_candidates(phrases: &[String], roots: &[PathBuf]) -> Option<Vec<PathBuf>> {
    if phrases.is_empty() {
        return None;
    }
    let output = Command::new("rg")
        .args(rg_args(phrases, roots))
        .output()
        .ok()?;
    // rg exits 1 when there are no matches; that's a valid empty result, not an error.
    if !output.status.success() && output.status.code() != Some(1) {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(
        stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect(),
    )
}

/// Whether a path is a candidate jsonl session file.
pub fn is_jsonl(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rg_args_shape() {
        let args = rg_args(
            &["acl".into(), "jellyfin".into()],
            &[PathBuf::from("/tmp/p")],
        );
        assert_eq!(&args[0..5], &["-l", "-i", "-F", "--glob", "*.jsonl"]);
        assert!(args.windows(2).any(|w| w == ["-e", "acl"]));
        assert!(args.windows(2).any(|w| w == ["-e", "jellyfin"]));
        assert_eq!(args.last().unwrap(), "/tmp/p");
    }

    #[test]
    fn test_no_phrases_means_fallback() {
        assert!(rg_candidates(&[], &[PathBuf::from("/tmp")]).is_none());
    }

    #[test]
    fn test_is_jsonl() {
        assert!(is_jsonl(Path::new("a/b.jsonl")));
        assert!(!is_jsonl(Path::new("a/b.txt")));
    }
}
