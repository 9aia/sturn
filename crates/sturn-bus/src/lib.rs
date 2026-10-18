//! Bus port — topic pub/sub layered over the WAL. The bus is a
//! cache: late subscribers catch up via `Wal::events`, never from
//! the bus. If the bus dies, coordination still replicates through
//! ordinary git push/fetch of `refs/sturn/*`.

use std::io;

/// Pull-based pub/sub — the daemon fans WAL appends onto topics;
/// consumers poll with a cursor. Push transports (unix socket,
/// websocket) are new impl crates, not changes here.
pub trait Bus: Send + Sync {
    /// Append `payload` (event JSON) under `repo`/`topic`; returns
    /// the entry's sequence number.
    fn publish(&self, repo: &str, topic: &str, payload: &[u8]) -> io::Result<u64>;
    /// Entries after `cursor`, oldest first; cursor moves to tail.
    fn poll(&self, repo: &str, topic: &str, cursor: &mut u64) -> io::Result<Vec<Vec<u8>>>;
    /// Topics with entries under `repo`.
    fn topics(&self, repo: &str) -> io::Result<Vec<String>>;
}
