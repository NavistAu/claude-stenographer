use std::fs;
use std::path::{Path, PathBuf};

pub fn encode_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    format!("-{}", s.trim_start_matches('/').replace('/', "-"))
}

/// The one rule for "does this project dir belong to this working dir":
/// the encoded name itself, or a subdirectory project (encoded prefix + '-').
/// Shared by directory resolution (lexical) and dense-result scope filtering.
pub fn project_in_scope(project_name: &str, encoded_working_dir: &str) -> bool {
    project_name == encoded_working_dir
        || project_name.starts_with(&format!("{encoded_working_dir}-"))
}

/// Encoded names of the working dir's path ancestors, nearest first,
/// stopping before the filesystem root. Sessions run from a parent
/// directory (e.g. ~/ws for ~/ws/impostarr) land in these projects.
pub fn ancestor_names(working_dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = working_dir;
    while let Some(parent) = cur.parent() {
        if parent.as_os_str().is_empty() || parent == Path::new("/") {
            break;
        }
        names.push(encode_path(parent));
        cur = parent;
    }
    names
}

/// Project dirs that are exact path ancestors of the working dir. Exact
/// match only — a prefix match here would pull in sibling projects
/// (e.g. -Users-x-ws-other for -Users-x-ws).
pub fn resolve_ancestor_dirs(working_dir: &Path, claude_projects_dir: &Path) -> Vec<PathBuf> {
    let names: std::collections::HashSet<String> =
        ancestor_names(working_dir).into_iter().collect();
    let entries = match fs::read_dir(claude_projects_dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };
    entries
        .flatten()
        .filter(|e| names.contains(&e.file_name().to_string_lossy().to_string()))
        .map(|e| e.path())
        .collect()
}

pub fn resolve_project_dirs(
    working_dir: &Path,
    claude_projects_dir: &Path,
    all_projects: bool,
) -> Vec<PathBuf> {
    let entries = match fs::read_dir(claude_projects_dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };
    let encoded = encode_path(working_dir);
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if all_projects || project_in_scope(&name, &encoded) {
            dirs.push(entry.path());
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_dirs(base: &Path) {
        for d in [
            "-Users-alice",
            "-Users-alice-ws-kicad",
            "-Users-alice-ws-kisane",
            "-Users-other",
        ] {
            fs::create_dir_all(base.join(d)).unwrap();
        }
    }

    #[test]
    fn test_encode_path() {
        assert_eq!(encode_path(Path::new("/Users/alice")), "-Users-alice");
        assert_eq!(
            encode_path(Path::new("/Users/alice/ws/kicad")),
            "-Users-alice-ws-kicad"
        );
    }

    #[test]
    fn test_exact_match() {
        let tmp = tempfile::tempdir().unwrap();
        setup_test_dirs(tmp.path());
        let dirs = resolve_project_dirs(Path::new("/Users/alice/ws/kicad"), tmp.path(), false);
        assert_eq!(dirs.len(), 1);
        assert!(dirs[0].ends_with("-Users-alice-ws-kicad"));
    }

    #[test]
    fn test_ancestor_names() {
        assert_eq!(
            ancestor_names(Path::new("/Users/alice/ws/impostarr")),
            vec!["-Users-alice-ws", "-Users-alice", "-Users"]
        );
        assert!(ancestor_names(Path::new("/Users")).is_empty());
    }

    #[test]
    fn test_resolve_ancestor_dirs_exact_match_only() {
        let tmp = tempfile::tempdir().unwrap();
        setup_test_dirs(tmp.path());
        // Ancestors of ws/kicad: -Users-alice-ws (absent), -Users-alice
        // (present), -Users (absent). Sibling -Users-alice-ws-kisane and the
        // project itself must NOT appear.
        let dirs = resolve_ancestor_dirs(Path::new("/Users/alice/ws/kicad"), tmp.path());
        assert_eq!(dirs.len(), 1);
        assert!(dirs[0].ends_with("-Users-alice"));
    }

    #[test]
    fn test_project_in_scope() {
        let enc = "-Users-alice-ws-claude";
        assert!(project_in_scope("-Users-alice-ws-claude", enc));
        assert!(project_in_scope("-Users-alice-ws-claude-scanline", enc));
        assert!(!project_in_scope("-Users-alice-ws-kvmplus", enc));
        // Sibling sharing a name prefix but not a path boundary must not match.
        assert!(!project_in_scope("-Users-alice-ws-claudette", enc));
    }

    #[test]
    fn test_parent_dir_includes_children() {
        let tmp = tempfile::tempdir().unwrap();
        setup_test_dirs(tmp.path());
        let mut dirs = resolve_project_dirs(Path::new("/Users/alice"), tmp.path(), false);
        dirs.sort();
        assert_eq!(dirs.len(), 3);
    }

    #[test]
    fn test_all_projects() {
        let tmp = tempfile::tempdir().unwrap();
        setup_test_dirs(tmp.path());
        let dirs = resolve_project_dirs(Path::new("/Users/alice"), tmp.path(), true);
        assert_eq!(dirs.len(), 4);
    }

    #[test]
    fn test_no_match() {
        let tmp = tempfile::tempdir().unwrap();
        setup_test_dirs(tmp.path());
        let dirs = resolve_project_dirs(Path::new("/Users/nobody"), tmp.path(), false);
        assert!(dirs.is_empty());
    }
}
