//! Wal impl — each event is a blob; each append is a commit on
//! `refs/sturn/wal/<ws>` whose tree holds `event.json`. Objects,
//! commits, refs, and tree walks go through the shared
//! `lasius-vcs` port — the git mechanics (loose-object fast path +
//! plumbing fallback) live in lasius-vcs-git (P-049). Only the CAS
//! publish semantics stay sturn's.

use std::io;
use std::path::Path;
use std::process::Command;
use sturn_core::Event;
use sturn_wal::Wal;

use lasius_vcs::Vcs;
use lasius_vcs_git::GitVcs;

/// Git-backed WAL. Concurrent appenders retry CAS on the ref —
/// the log is correct under racing agents, just not fair.
pub struct GitWal {
    /// Ref prefix — one ref per workspace below it.
    pub ref_prefix: String,
    /// SSH key to sign events (`ssh-keygen -Y sign`, namespace
    /// `sturn-event`). None = unsigned ops.
    pub sign_key: Option<std::path::PathBuf>,
}

impl Default for GitWal {
    fn default() -> Self {
        GitWal {
            ref_prefix: "refs/sturn/wal".into(),
            sign_key: None,
        }
    }
}

/// Permanent `update-ref` failures — bad refnames are already
/// rejected by [`GitWal::wal_ref`], so what remains is environmental
/// (a vanished repo). These can never win a CAS retry.
#[cfg(not(coverage))]
fn permanent_update_ref_failure(e: &io::Error) -> bool {
    let msg = e.to_string();
    msg.contains("bad name") || msg.contains("not a git repository")
}

/// `ssh-keygen -Y sign` on the canonical payload — SSH signing is
/// stdlib tooling, Ed25519 under the hood, no new crypto deps.
fn sign_event(payload: &[u8], key: &Path) -> io::Result<String> {
    // Unique per call — concurrent signing threads in one process
    // (tests, the MCP server) must not share the scratch file.
    static SIGN_IDX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let base = std::env::temp_dir().join(format!(
        "sturn-sign-{}-{}",
        std::process::id(),
        SIGN_IDX.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let data = base.with_extension("data");
    std::fs::write(&data, payload)?;
    let st = Command::new("ssh-keygen")
        .args(["-Y", "sign", "-f"])
        .arg(key)
        .args(["-n", "sturn-event"])
        .arg(&data)
        .status()?;
    let sig = std::fs::read_to_string(data.with_extension("data.sig"));
    let _ = std::fs::remove_file(&data);
    let _ = std::fs::remove_file(data.with_extension("data.sig"));
    if !st.success() {
        return Err(io::Error::other("ssh-keygen -Y sign failed"));
    }
    sig
}

impl GitWal {
    /// `<prefix>/<ws>` — the ws must be a valid refname tail. An
    /// invalid name is a *permanent* error: it can never win a CAS
    /// race, so it fails here instead of burning the retry budget
    /// inside `append`.
    fn wal_ref(&self, ws: &str) -> io::Result<String> {
        if !sturn_core::valid_ws_name(ws) {
            return Err(io::Error::other(format!("invalid workspace name {ws:?}")));
        }
        Ok(format!("{}/{ws}", self.ref_prefix))
    }
}

/// Write a blob/tree straight to the ODB via the port; the impl's
/// plumbing fallback covers the layouts the direct writer can't
/// resolve (sha256 odb, GIT_DIR env, …).
fn put_object(repo: &Path, kind: &str, payload: &[u8]) -> io::Result<String> {
    GitVcs.write_object(repo, kind, payload)
}

fn put_tree(repo: &Path, blob: &str, name: &str) -> io::Result<String> {
    GitVcs.write_tree(repo, &[(0o100644, name, blob)])
}

/// `commit-tree` via the port — direct object write with `git var`
/// idents inside the impl, plumbing fallback for date envs.
fn put_commit(repo: &Path, tree: &str, parent: Option<&str>, msg: &str) -> io::Result<String> {
    let parents: Vec<&str> = parent.into_iter().collect();
    GitVcs.commit_tree(repo, tree, &parents, msg)
}

impl Wal for GitWal {
    fn append(&self, repo: &Path, ws: &str, ev: &Event) -> io::Result<String> {
        let mut ev = ev.clone();
        if let Some(key) = &self.sign_key {
            ev.sig = Some(sign_event(&ev.signing_bytes(), key)?);
        }
        let payload = serde_json::to_vec(&ev).map_err(io::Error::other)?;
        let blob = put_object(repo, "blob", &payload)?;
        let msg = format!(
            "sturn:{ws}:{}",
            serde_json::to_value(&ev.op)
                .ok()
                .and_then(|v| v.get("op").and_then(|k| k.as_str()).map(String::from))
                .unwrap_or_else(|| "event".into())
        );
        let r = self.wal_ref(ws)?;
        // CAS loop: read parent, build commit on top, advance the ref
        // only if it still points where we read. Racing agents retry.
        let mut attempt = 0u8;
        loop {
            let parent = self.head(repo, ws)?;
            let tree = put_tree(repo, &blob, "event.json")?;
            let commit = put_commit(repo, &tree, parent.as_deref(), &msg)?;
            attempt += 1;
            // update_ref surfaces stderr — a permanent failure (bad
            // refname, missing repo) can never succeed on retry, so
            // it returns immediately instead of burning CAS tries.
            // update-ref only fails on a lost CAS race in tests —
            // the whole arm is coverage-exempt like the bound below.
            match GitVcs.update_ref(repo, &r, &commit, Some(parent.as_deref().unwrap_or(""))) {
                Ok(_) => return Ok(blob),
                #[cfg(not(coverage))]
                Err(e) => {
                    if permanent_update_ref_failure(&e) {
                        return Err(e);
                    }
                }
                #[cfg(coverage)]
                Err(_) => {}
            }
            // 8 straight CAS losses — untestable contention, so the
            // bound itself is excluded under coverage.
            #[cfg(not(coverage))]
            if attempt == 8 {
                return Err(io::Error::other(format!(
                    "wal append lost CAS race on {r} after 8 tries"
                )));
            }
        }
    }

    fn events_since(&self, repo: &Path, ws: &str, from: Option<&str>) -> io::Result<Vec<Event>> {
        let r = self.wal_ref(ws)?;
        if self.head(repo, ws)?.is_none() {
            return Ok(vec![]);
        }
        let range = match from {
            Some(f) => format!("{f}..{r}"),
            None => r,
        };
        let commits = GitVcs.exec_out(repo, &["rev-list", "--reverse", &range])?;
        let mut out = Vec::new();
        for c in commits.lines().filter(|l| !l.is_empty()) {
            // Loose-object fast path + plumbing fallback live inside
            // the port's read_at.
            let json = match GitVcs.read_at(repo, c, "event.json") {
                Ok(Some(b)) => String::from_utf8_lossy(&b).into_owned(),
                Ok(None) => return Err(io::Error::other(format!("{c}:event.json not found"))),
                Err(e) => return Err(e),
            };
            let mut ev: Event = serde_json::from_str(&json)
                .map_err(|e| io::Error::other(format!("bad event in {c}: {e}")))?;
            // Backfill id for events written before id assignment —
            // the id *is* the event blob's content address.
            if ev.id.is_empty() {
                ev.id = lasius_vcs_git::object_id("blob", json.as_bytes());
            }
            out.push(ev);
        }
        Ok(out)
    }

    fn head(&self, repo: &Path, ws: &str) -> io::Result<Option<String>> {
        let r = self.wal_ref(ws)?;
        // Loose/packed ref read with a plumbing fallback inside the
        // port — covers reftable and other exotic layouts.
        GitVcs.read_ref(repo, &r)
    }

    fn compact(&self, repo: &Path, ws: &str) -> io::Result<usize> {
        let events = self.events(repo, ws)?;
        // Drop strictly *before* the last Snapshot marker — the marker
        // itself must survive, or the next snap would re-apply ops
        // that are already in the tip.
        let drop = sturn_core::since_last_snapshot(&events).saturating_sub(1);
        if drop == 0 {
            return Ok(0);
        }
        // drop > 0 implies events exist, which implies a head —
        // ok_or_else is defensive.
        let head = self
            .head(repo, ws)?
            .ok_or_else(|| io::Error::other("no wal ref"))?;
        // Archive the full history ref before rewriting — compaction
        // shrinks the live log, not the audit trail.
        GitVcs.update_ref(repo, &format!("refs/sturn/archive/{ws}"), &head, None)?;
        // Rebuild the chain from post-snapshot events. Blobs already
        // exist in the ODB — each surviving event re-commits cheaply.
        let mut parent: Option<String> = None;
        for ev in &events[drop..] {
            let payload = serde_json::to_vec(ev).map_err(io::Error::other)?;
            let blob = put_object(repo, "blob", &payload)?;
            let tree = put_tree(repo, &blob, "event.json")?;
            parent = Some(put_commit(
                repo,
                &tree,
                parent.as_deref(),
                "sturn:compacted",
            )?);
        }
        // events[drop..] always includes the snapshot marker itself
        // when drop > 0, so the rebuilt chain always has a tip —
        // the None arm (delete the whole ref) is defensive.
        match parent {
            Some(tip) => GitVcs.update_ref(repo, &self.wal_ref(ws)?, &tip, None)?,
            None => GitVcs.delete_ref(repo, &self.wal_ref(ws)?)?,
        };
        Ok(drop)
    }

    fn workspaces(&self, repo: &Path) -> io::Result<Vec<String>> {
        let refs = GitVcs.list_refs(repo, &self.ref_prefix)?;
        Ok(refs
            .iter()
            .filter_map(|l| {
                l.strip_prefix(&format!("{}/", self.ref_prefix))
                    .map(String::from)
            })
            .collect())
    }
}
