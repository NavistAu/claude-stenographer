use std::path::PathBuf;
use std::process::Command;

fn binary_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target/debug/rrecall");
    path
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn test_search_finds_match_with_claude_dir_override() {
    Command::new("cargo")
        .args(["build"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to build");

    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("-Users-test-ws-myproject");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::copy(
        fixture_dir().join("simple_session.jsonl"),
        project_dir.join("test-session.jsonl"),
    )
    .unwrap();

    // --index-dir isolates the test from any real dense index on the machine;
    // with no index there, hybrid falls back to lexical.
    let empty_index = tmp.path().join("no-index");
    let output = Command::new(binary_path())
        .args([
            "search",
            "auth middleware",
            "--project-dir",
            "/Users/test/ws/myproject",
            "--claude-dir",
            tmp.path().to_str().unwrap(),
            "--index-dir",
            empty_index.to_str().unwrap(),
            "--max-results",
            "5",
            "--format",
            "json",
        ])
        .env("RRECALL_NO_RECONCILE", "1")
        .output()
        .expect("Failed to run rrecall");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("Output should be valid JSON");

    assert_eq!(parsed["query"], "auth middleware");
    assert_eq!(parsed["sessions_searched"], 1);
    let results = parsed["results"]
        .as_array()
        .expect("results should be array");
    assert!(
        !results.is_empty(),
        "Should find matches for 'auth middleware'"
    );
    assert!(!results[0]["match_context"].as_array().unwrap().is_empty());
}

#[test]
fn test_search_no_results() {
    Command::new("cargo")
        .args(["build"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to build");

    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("-Users-test-ws-myproject");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::copy(
        fixture_dir().join("simple_session.jsonl"),
        project_dir.join("test-session.jsonl"),
    )
    .unwrap();

    // --index-dir isolates the test from any real dense index on the machine;
    // with no index there, hybrid falls back to lexical.
    let empty_index = tmp.path().join("no-index");
    let output = Command::new(binary_path())
        .args([
            "search",
            "nonexistent_query_xyz",
            "--project-dir",
            "/Users/test/ws/myproject",
            "--claude-dir",
            tmp.path().to_str().unwrap(),
            "--index-dir",
            empty_index.to_str().unwrap(),
            "--format",
            "json",
        ])
        .env("RRECALL_NO_RECONCILE", "1")
        .output()
        .expect("Failed to run rrecall");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("Output should be valid JSON");
    let results = parsed["results"]
        .as_array()
        .expect("results should be array");
    assert!(results.is_empty());
}

#[test]
fn test_ancestor_tier_finds_parent_dir_session() {
    Command::new("cargo")
        .args(["build"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to build");

    let tmp = tempfile::tempdir().unwrap();
    // The session about the project lives in the PARENT dir's project
    // (run from /Users/test/ws); the project's own dir has no sessions.
    let parent_project = tmp.path().join("-Users-test-ws");
    let own_project = tmp.path().join("-Users-test-ws-myproject");
    std::fs::create_dir_all(&parent_project).unwrap();
    std::fs::create_dir_all(&own_project).unwrap();
    std::fs::copy(
        fixture_dir().join("simple_session.jsonl"),
        parent_project.join("parent-session.jsonl"),
    )
    .unwrap();

    // --index-dir isolates the test from any real dense index on the machine;
    // with no index there, hybrid falls back to lexical.
    let empty_index = tmp.path().join("no-index");
    let output = Command::new(binary_path())
        .args([
            "search",
            "auth middleware",
            "--project-dir",
            "/Users/test/ws/myproject",
            "--claude-dir",
            tmp.path().to_str().unwrap(),
            "--index-dir",
            empty_index.to_str().unwrap(),
            "--format",
            "json",
        ])
        .env("RRECALL_NO_RECONCILE", "1")
        .output()
        .expect("Failed to run rrecall");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("Output should be valid JSON");
    assert_eq!(
        parsed["scope_reached"], "ancestor_projects",
        "got: {stdout}"
    );
    let results = parsed["results"]
        .as_array()
        .expect("results should be array");
    assert!(
        !results.is_empty(),
        "ancestor tier should find the parent-dir session; got: {stdout}"
    );
}

#[test]
fn test_missing_claude_dir() {
    Command::new("cargo")
        .args(["build"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to build");

    let output = Command::new(binary_path())
        .args([
            "search",
            "test",
            "--claude-dir",
            "/nonexistent/path",
            "--format",
            "json",
        ])
        .env("RRECALL_NO_RECONCILE", "1")
        .output()
        .expect("Failed to run rrecall");

    assert!(
        !output.status.success(),
        "Should exit non-zero for missing dir"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("Output should be valid JSON even on error");
    assert!(parsed["error"].as_str().unwrap().contains("not found"));
}

#[test]
fn test_acl_canary_vocabulary_mismatch() {
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let bin = env!("CARGO_BIN_EXE_rrecall");
    // --index-dir isolates the test from any real dense index on the machine;
    // with no index there, hybrid falls back to lexical.
    let empty_index = tempfile::tempdir().unwrap();
    let output = Command::new(bin)
        .args([
            "search",
            "acl permission inherited",
            "--all-projects",
            "--claude-dir",
            fixtures,
            "--index-dir",
            empty_index.path().join("no-index").to_str().unwrap(),
            "--target",
            "1",
            "--format",
            "json",
        ])
        .env("RRECALL_NO_RECONCILE", "1")
        .output()
        .expect("run rrecall");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("acl-canary"),
        "must find the canary session from vocabulary-mismatched OR terms; got: {}",
        stdout
    );
    assert!(stdout.contains("\"hit_count\""), "must report hit_count");
    assert!(
        stdout.contains("\"scope_reached\": \"all_projects\""),
        "--all-projects must report scope_reached=all_projects; got: {}",
        stdout
    );
}
