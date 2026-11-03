//! Ws port — virtual workspaces over the object store. Agents read
//! blobs and list trees without a checkout; hydration is opt-in for
//! tools that still need POSIX. Git impl namespace:
//! `refs/sturn/ws/<name>`.
//!
//! The port speaks git's vocabulary (refs, rev, sha) as the
//! contract — a non-git impl maps equivalent concepts onto the same
//! semantics. Ids below (`rev`, `sha`) are opaque content/revision
//! identifiers, not necessarily git object ids.

use std::io;
use std::path::Path;

/// A workspace is a pointer, not a checkout — creating one costs
/// nothing, which is what agent swarms actually need.
pub trait Ws: Send + Sync {
    /// Blob contents at `rev:path`.
    fn read(&self, repo: &Path, rev: &str, path: &str) -> io::Result<Vec<u8>>;
    /// Every path in `rev`'s tree (grep/glob is a client filter).
    fn list(&self, repo: &Path, rev: &str) -> io::Result<Vec<String>>;
    /// Materialize `rev` into `dir` — for compilers that need files.
    /// `paths` filters to a subset (pathspec); empty = whole tree.
    fn materialize(
        &self,
        repo: &Path,
        rev: &str,
        dir: &Path,
        paths: &[String],
    ) -> io::Result<()>;
    /// Create a workspace namespace pointing at `base` (git impl:
    /// `refs/sturn/ws/<name>` = `base`).
    fn create(&self, repo: &Path, name: &str, base: &str) -> io::Result<()>;
    /// Blob id at `rev:path` — None when absent. The lookup that
    /// makes overlay reads cheap (content equality = id equality).
    fn entry(&self, repo: &Path, rev: &str, path: &str) -> io::Result<Option<String>>;
    /// Blob contents by id.
    fn blob(&self, repo: &Path, sha: &str) -> io::Result<Vec<u8>>;
    /// Store bytes as a blob; returns its content id.
    fn store(&self, repo: &Path, content: &[u8]) -> io::Result<String>;
    /// Revision the ws points at (base today; WAL overlay lands in M4).
    fn tip(&self, repo: &Path, name: &str) -> io::Result<String>;
    /// Workspace names with a namespace entry — includes
    /// workspaces that haven't appended an op yet (no WAL log).
    fn workspaces(&self, repo: &Path) -> io::Result<Vec<String>>;
}
