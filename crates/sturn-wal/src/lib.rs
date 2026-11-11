//! Wal port — the append-only op log that *is* the state.
//! Git impl in `sturn-wal-git`; libgrite = a future impl.
//!
//! The ports speak git's vocabulary (refs, sha, commit) as the
//! contract — a non-git impl maps equivalent concepts (bookmark,
//! change-id, object id) onto the same semantics.

use std::io;
use std::path::Path;
use sturn_core::Event;

/// Append/read the per-workspace op log — one log per workspace,
/// serialized per workspace, parallel across them (git impl:
/// commits on `refs/sturn/wal/<ws>`). Replicates with ordinary
/// push/fetch — the bus on top is a cache, never the authority.
pub trait Wal: Send + Sync {
    /// Append `ev` to the ws WAL; returns the assigned event id.
    /// Must be linearizable under concurrent appends — mechanism
    /// is impl-defined (git: CAS retry on the ref).
    fn append(&self, repo: &Path, ws: &str, ev: &Event) -> io::Result<String>;
    /// All events for `ws`, oldest first.
    fn events(&self, repo: &Path, ws: &str) -> io::Result<Vec<Event>> {
        self.events_since(repo, ws, None)
    }
    /// Events appended after `from` (an opaque cursor returned by
    /// `head`, exclusive) — `None` reads the whole log. Delta reads
    /// are what the daemon and catch-up need; `events` is the full
    /// replay.
    fn events_since(&self, repo: &Path, ws: &str, from: Option<&str>) -> io::Result<Vec<Event>>;
    /// Opaque tip cursor for the ws log — cheap change detection.
    /// Callers must not interpret the value; feed it back to
    /// `events_since`.
    fn head(&self, repo: &Path, ws: &str) -> io::Result<Option<String>>;
    /// Workspace ids that have a log.
    fn workspaces(&self, repo: &Path) -> io::Result<Vec<String>>;
    /// Drop pre-snapshot history: events before the last `Snapshot`
    /// are already in the tip — replaying them is waste. Returns
    /// events dropped. Old history moves to an archive namespace
    /// (git impl: `refs/sturn/archive/<ws>`) — compaction loses
    /// nothing.
    fn compact(&self, repo: &Path, ws: &str) -> io::Result<usize>;
}
