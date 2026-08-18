//! Search-time reconcile: keep the dense index fresh from the binary itself,
//! not just the `SessionEnd` hook. After a dense/hybrid search returns, a
//! detached `rrecall index` is spawned so the index converges to complete even
//! if the hook never fired.
//!
//! macOS ships no `flock(1)`, so the original hook's `flock -n` silently failed
//! there and the index never built. Single-flight is therefore taken in-process
//! via `flock(2)` on the lockfile fd — the OS releases it when the process
//! exits, even on crash, so there are no stale locks.
//!
//! Three guarantees (per ROADMAP "Search-time reconcile insurance"):
//!   * **non-blocking** — search returns immediately; the build runs detached;
//!   * **single-flighted** — the spawned `index` holds [`try_build_lock`] for
//!     the whole build; a second build bails instead of stampeding;
//!   * **throttled** — a burst of searches spawns at most one reconcile, gated
//!     on the lockfile mtime; and it **never recurses** (only search spawns).

use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

/// A burst of searches within this window spawns at most one reconcile.
const THROTTLE: Duration = Duration::from_secs(300);

/// Lock file lives INSIDE the index dir it guards. Anchoring it to TMPDIR
/// broke single-flight: sandboxed and unsandboxed shells see different
/// TMPDIRs, so two builds of the SAME index locked different files and raced
/// each other's (non-atomic) save.
fn lock_path(index_dir: &Path) -> PathBuf {
    index_dir.join(".build.lock")
}

/// Drop the whole process to background scheduling priority. Embedding
/// saturates every core via the ONNX runtime's thread pool (fastembed exposes
/// no thread cap), so an index build at normal priority starves interactive
/// work. On macOS this uses the Darwin background band — CPU/IO throttled and,
/// on Apple Silicon, scheduled onto efficiency cores. Elsewhere: nice 19.
pub fn background_priority() {
    unsafe {
        #[cfg(target_os = "macos")]
        let _ = libc::setpriority(libc::PRIO_DARWIN_PROCESS, 0, libc::PRIO_DARWIN_BG);
        #[cfg(not(target_os = "macos"))]
        let _ = libc::setpriority(libc::PRIO_PROCESS, 0, 19);
    }
}

/// Held for the lifetime of an index build. The advisory `flock` is released
/// automatically when the file descriptor closes (i.e. process exit), so a
/// crashed build never wedges the lock. `None` means another build holds it.
pub struct BuildLock {
    _file: std::fs::File,
}

/// Try to take the exclusive build lock without blocking. Returns `None` if
/// another `rrecall index` is already running against this index dir.
pub fn try_build_lock(index_dir: &Path) -> Option<BuildLock> {
    std::fs::create_dir_all(index_dir).ok()?;
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(lock_path(index_dir))
        .ok()?;
    // LOCK_EX | LOCK_NB: exclusive, fail rather than wait if already held.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Some(BuildLock { _file: file })
    } else {
        None
    }
}

/// After a dense/hybrid search, converge the index in the background so it stays
/// fresh without relying on the `SessionEnd` hook. Detached, throttled, and
/// single-flighted downstream. Set `RRECALL_NO_RECONCILE` to disable (the spawned
/// child sets it, so it can never recurse).
pub fn spawn_reconcile(index_dir: &Path) {
    if std::env::var_os("RRECALL_NO_RECONCILE").is_some() {
        return;
    }
    std::fs::create_dir_all(index_dir).ok();
    let lock = lock_path(index_dir);
    // Throttle: a reconcile was kicked off recently — skip the spawn churn.
    if let Ok(modified) = std::fs::metadata(&lock).and_then(|m| m.modified()) {
        if modified.elapsed().map(|e| e < THROTTLE).unwrap_or(false) {
            return;
        }
    }
    // Touch the marker (update mtime) so a flurry of searches throttles to one.
    if let Ok(f) = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&lock)
    {
        let _ = f.set_modified(SystemTime::now());
    }
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return,
    };
    // Detached: own process group so the harness reaping the search's group
    // doesn't kill the build; stdio to /dev/null so no pipe keeps the caller
    // (the agent's Bash tool) waiting on it.
    let _ = Command::new(exe)
        .arg("index")
        .arg("--all-projects")
        .arg("--index-dir")
        .arg(index_dir)
        .env("RRECALL_NO_RECONCILE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
}
