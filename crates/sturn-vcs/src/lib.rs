//! Vcs port — the ordinary-git operations sturn still needs.
//! Remotes stay git's transport; the WAL rides along on push.
//! Impl: `sturn-git`.

use std::path::Path;

/// All methods return bool success — callers decide whether a
/// failure is retryable (offline remote) or fatal.
pub trait Vcs: Send + Sync {
    /// Fetch + prune so remote refs are current.
    fn sync(&self, repo: &Path) -> bool;
    /// Does the object exist locally?
    fn has_commit(&self, repo: &Path, sha: &str) -> bool;
    /// Push heads AND `refs/sturn/*` — coordination state replicates
    /// for free on the same remote, same signed objects.
    fn push(&self, repo: &Path, remote: &str) -> bool;
}
