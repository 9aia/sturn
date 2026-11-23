//! Git helpers for sturn — the snap bridge (WAL ops → real
//! commit) plus thin wrappers over the shared `lasius-vcs` port
//! for the coordination ops (resolve/has_commit/push/store). All
//! git mechanics live in lasius (P-019/P-049); this crate keeps
//! only the sturn-flavored folding of WAL events into tree ops.

use lasius_vcs::{TreeOp, Vcs};
use lasius_vcs_git::GitVcs as LasiusGit;
use std::collections::HashMap;
use std::io;
use std::path::Path;
use sturn_core::{Event, Op};

/// Resolve `rev` via the shared port — snap commits may not be
/// referenced yet, but `rev-parse` resolves loose objects fine.
pub fn resolve(repo: &Path, rev: &str) -> Option<String> {
    LasiusGit.resolve(repo, rev)
}

/// Object-exists probe via the shared port (`cat-file -e`, with a
/// loose-object fast path inside the impl).
pub fn has_commit(repo: &Path, sha: &str) -> bool {
    LasiusGit.has_commit(repo, sha)
}

/// Push heads + `refs/sturn/*` — the coordination-specific part
/// sturn keeps; the mechanics are lasius-vcs-git's.
pub fn push(repo: &Path, remote: &str) -> bool {
    LasiusGit.push(repo, remote, &["--all"])
        && LasiusGit.push(repo, remote, &["refs/sturn/*:refs/sturn/*"])
}

/// Store `content` as a blob in the ODB; returns its sha.
pub fn store_blob(repo: &Path, content: &[u8]) -> io::Result<String> {
    LasiusGit.write_object(repo, "blob", content)
}

/// Overlay `events`' Write/Rename/Delete ops on `base`, commit to
/// `branch`, return the new commit sha. Coordination ops (claims,
/// intents, reviews) are metadata — they never touch the tree.
///
/// The port's `commit_overlay` uses a scratch index
/// (`GIT_INDEX_FILE`) — no working tree, no checkout, no lock on
/// the user's index.
pub fn snap_commit(
    repo: &Path,
    base: &str,
    events: &[Event],
    message: &str,
    branch: &str,
) -> io::Result<String> {
    // Track the overlay as we walk — a Rename's `from` may have been
    // created by an earlier Write in the same batch (never in base).
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut ops: Vec<TreeOp> = Vec::new();
    for ev in events {
        match &ev.op {
            Op::Write { path, blob, .. } => {
                ops.push(TreeOp::Write {
                    path: path.clone(),
                    oid: blob.clone(),
                });
                seen.insert(path.clone(), blob.clone());
            }
            Op::Delete { path } => {
                ops.push(TreeOp::Delete { path: path.clone() });
                seen.remove(path.as_str());
            }
            Op::Rename { from, to } => {
                let blob = match seen.get(from.as_str()) {
                    Some(b) => b.clone(),
                    None => LasiusGit
                        .entry_oid(repo, base, from)?
                        .ok_or_else(|| io::Error::other(format!("{base}:{from}: no such entry")))?,
                };
                ops.push(TreeOp::Delete { path: from.clone() });
                ops.push(TreeOp::Write {
                    path: to.clone(),
                    oid: blob.clone(),
                });
                seen.remove(from.as_str());
                seen.insert(to.clone(), blob);
            }
            _ => {}
        }
    }
    LasiusGit.commit_overlay(repo, base, &ops, message, branch)
}
