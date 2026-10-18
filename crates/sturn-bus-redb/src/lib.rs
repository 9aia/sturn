//! Bus impl: redb-backed topic store. One table, composite key
//! `repo|topic|seq` (zero-padded for ordering), value = payload.
//! Scaffold-grade: prefix scans walk the whole table — index the
//! `(repo, topic)` counter once this is hot.

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::io;
use std::path::PathBuf;
use sturn_bus::Bus;

const TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("bus");
/// `repo|topic` → next seq — O(1) publish instead of a prefix scan.
const SEQ: TableDefinition<&str, u64> = TableDefinition::new("seq");

/// redb-backed local bus. Durable across daemon restarts — and even
/// if the file is lost, the WAL in git refs rebuilds everything.
pub struct RedbBus {
    db: Database,
}

impl RedbBus {
    /// Open (or create) the store.
    pub fn open(path: PathBuf) -> io::Result<Self> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        Ok(Self {
            db: Database::create(path).map_err(io::Error::other)?,
        })
    }

    fn key(repo: &str, topic: &str, seq: u64) -> String {
        format!("{repo}|{topic}|{seq:020}")
    }

    fn prefix(repo: &str, topic: &str) -> String {
        format!("{repo}|{topic}|")
    }
}

impl Bus for RedbBus {
    fn publish(&self, repo: &str, topic: &str, payload: &[u8]) -> io::Result<u64> {
        let tx = self.db.begin_write().map_err(io::Error::other)?;
        let seq;
        {
            let pfx = Self::prefix(repo, topic);
            let ck = pfx.trim_end_matches('|');
            let mut t = tx.open_table(TABLE).map_err(io::Error::other)?;
            let mut st = tx.open_table(SEQ).map_err(io::Error::other)?;
            seq = st
                .get(ck)
                .map_err(io::Error::other)?
                .map(|v| v.value())
                // Migrate: first publish after upgrade counts existing
                // keys once, then the counter takes over.
                .unwrap_or_else(|| 0);
            let seq = if seq == 0 {
                t.iter()
                    .map_err(io::Error::other)?
                    .flatten()
                    .filter(|(k, _)| k.value().starts_with(&pfx))
                    .count() as u64
            } else {
                seq
            };
            t.insert(Self::key(repo, topic, seq).as_str(), payload)
                .map_err(io::Error::other)?;
            st.insert(ck, seq + 1).map_err(io::Error::other)?;
        }
        tx.commit().map_err(io::Error::other)?;
        Ok(seq)
    }

    fn poll(&self, repo: &str, topic: &str, cursor: &mut u64) -> io::Result<Vec<Vec<u8>>> {
        let tx = self.db.begin_read().map_err(io::Error::other)?;
        let mut out = Vec::new();
        if let Ok(t) = tx.open_table(TABLE) {
            let pfx = Self::prefix(repo, topic);
            for r in t.iter().map_err(io::Error::other)?.flatten() {
                let (k, v) = r;
                if let Some(rest) = k.value().strip_prefix(&pfx) {
                    if let Ok(seq) = rest.parse::<u64>() {
                        if seq > *cursor {
                            out.push((seq, v.value().to_vec()));
                        }
                    }
                }
            }
        }
        out.sort_by_key(|(s, _)| *s);
        if let Some((s, _)) = out.last() {
            *cursor = *s;
        }
        Ok(out.into_iter().map(|(_, v)| v).collect())
    }

    fn topics(&self, repo: &str) -> io::Result<Vec<String>> {
        let tx = self.db.begin_read().map_err(io::Error::other)?;
        let mut out = std::collections::BTreeSet::new();
        if let Ok(t) = tx.open_table(TABLE) {
            let pfx = format!("{repo}|");
            for r in t.iter().map_err(io::Error::other)?.flatten() {
                let (k, _) = r;
                if let Some(rest) = k.value().strip_prefix(&pfx) {
                    if let Some((topic, _)) = rest.rsplit_once('|') {
                        out.insert(topic.to_string());
                    }
                }
            }
        }
        Ok(out.into_iter().collect())
    }
}
