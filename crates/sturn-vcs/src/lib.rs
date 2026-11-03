//! Vcs port — the host-side VCS operations sturn still needs.
//! Remotes stay the backend's transport; the WAL rides along on
//! push. Impl: `sturn-git`.
//!
//! The port speaks git's vocabulary (refs, sha, commit) as the
//! contract — a non-git impl maps equivalent concepts (bookmark,
//! change-id, object id) onto the same semantics.

use std::path::Path;

/// All methods return bool success — callers decide whether a
/// failure is retryable (offline remote) or fatal.
pub trait Vcs: Send + Sync {
    /// Fetch + prune so `remote`'s refs are current.
    fn sync(&self, repo: &Path, remote: &str) -> bool;
    /// Does the object exist locally?
    fn has_commit(&self, repo: &Path, sha: &str) -> bool;
    /// Resolve a ref/revision to an object id — `None` when
    /// unresolvable.
    fn resolve(&self, repo: &Path, rev: &str) -> Option<String>;
    /// Publish local branches and the coordination namespace —
    /// state replicates for free on the same remote (git impl:
    /// heads + `refs/sturn/*`).
    fn push(&self, repo: &Path, remote: &str) -> bool;
}
