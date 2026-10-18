//! Wal port — the append-only op log that *is* the state.
//! Git impl in `sturn-wal-git`; libgrite = a future impl.

use std::io;
use std::path::Path;
use sturn_core::Event;

/// Append/read the per-workspace op log under `refs/sturn/wal/*`.
/// Replicates with ordinary git push/fetch — the bus on top is a
/// cache, never the authority.
pub trait Wal: Send + Sync {
    /// Append `ev` to the ws WAL; returns the assigned event id.
    /// Must survive concurrent appends (CAS retry on the ref).
    fn append(&self, repo: &Path, ws: &str, ev: &Event) -> io::Result<String>;
    /// All events for `ws`, oldest first.
    fn events(&self, repo: &Path, ws: &str) -> io::Result<Vec<Event>> {
        self.events_since(repo, ws, None)
    }
    /// Events appended after `from` (a prior `head`, exclusive) —
    /// `None` reads the whole log. Delta reads are what the daemon
    /// and catch-up need; `events` is the full replay.
    fn events_since(
        &self,
        repo: &Path,
        ws: &str,
        from: Option<&str>,
    ) -> io::Result<Vec<Event>>;
    /// Tip commit of the ws WAL ref — cheap change detection.
    fn head(&self, repo: &Path, ws: &str) -> io::Result<Option<String>>;
    /// Workspace ids that have a WAL ref.
    fn workspaces(&self, repo: &Path) -> io::Result<Vec<String>>;
}
