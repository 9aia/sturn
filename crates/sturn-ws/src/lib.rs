//! Ws port — virtual workspaces over the git ODB. Agents read blobs
//! and list trees without a checkout; hydration is opt-in for tools
//! that still need POSIX. `refs/sturn/ws/<name>` is the namespace.

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
    fn materialize(&self, repo: &Path, rev: &str, dir: &Path) -> io::Result<()>;
    /// Create a workspace namespace: `refs/sturn/ws/<name>` = `base`.
    fn create(&self, repo: &Path, name: &str, base: &str) -> io::Result<()>;
    /// Commit the ws points at (base today; WAL overlay lands in M4).
    fn tip(&self, repo: &Path, name: &str) -> io::Result<String>;
    /// Workspace names with a `refs/sturn/ws/*` ref — includes
    /// workspaces that haven't appended an op yet (no WAL ref).
    fn workspaces(&self, repo: &Path) -> io::Result<Vec<String>>;
}
