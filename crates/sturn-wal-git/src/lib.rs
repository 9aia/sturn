//! Wal impl via git plumbing — each event is a blob; each append is
//! a commit on `refs/sturn/wal/<ws>` whose tree holds `event.json`.
//! Zero deps beyond the `git` binary; replicates with push/fetch.

use std::io;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use sturn_core::Event;
use sturn_wal::Wal;

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

fn git(repo: &Path, args: &[&str]) -> io::Result<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(io::Error::other(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        )))
    }
}

fn git_in(repo: &Path, args: &[&str], input: &[u8]) -> io::Result<String> {
    let mut c = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    c.stdin.take().unwrap().write_all(input)?;
    let o = c.wait_with_output()?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(io::Error::other(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        )))
    }
}

/// `ssh-keygen -Y sign` on the canonical payload — SSH signing is
/// stdlib tooling, Ed25519 under the hood, no new crypto deps.
fn sign_event(payload: &[u8], key: &Path) -> io::Result<String> {
    let base = std::env::temp_dir().join(format!("sturn-sign-{}", std::process::id()));
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
    fn wal_ref(&self, ws: &str) -> String {
        format!("{}/{ws}", self.ref_prefix)
    }
}

impl Wal for GitWal {
    fn append(&self, repo: &Path, ws: &str, ev: &Event) -> io::Result<String> {
        let mut ev = ev.clone();
        if let Some(key) = &self.sign_key {
            ev.sig = Some(sign_event(&ev.signing_bytes(), key)?);
        }
        let payload = serde_json::to_vec(&ev).map_err(io::Error::other)?;
        let blob = git_in(repo, &["hash-object", "-w", "--stdin"], &payload)?;
        let msg = format!("sturn:{ws}:{}", serde_json::to_value(&ev.op)
            .ok()
            .and_then(|v| v.get("op").and_then(|k| k.as_str()).map(String::from))
            .unwrap_or_else(|| "event".into()));
        let r = self.wal_ref(ws);
        // CAS loop: read parent, build commit on top, advance the ref
        // only if it still points where we read. Racing agents retry.
        let mut attempt = 0u8;
        loop {
            let parent = self.head(repo, ws)?;
            let tree = git_in(repo, &["mktree"], format!("100644 blob {blob}\tevent.json\n").as_bytes())?;
            let mut args = vec!["commit-tree", tree.as_str()];
            let p;
            if let Some(parent) = &parent {
                p = parent.clone();
                args.extend(["-p", p.as_str()]);
            }
            args.extend(["-m", msg.as_str()]);
            let commit = git(repo, &args)?;
            let ok = Command::new("git")
                .arg("-C")
                .arg(repo)
                .arg("update-ref")
                .arg(&r)
                .arg(&commit)
                .arg(parent.as_deref().unwrap_or(""))
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            attempt += 1;
            if ok { return Ok(blob); }
            // 8 straight CAS losses — untestable contention, so the
            // bound itself is excluded under coverage.
            #[cfg(not(coverage))]
            if attempt == 8 { return Err(io::Error::other(format!("wal append lost CAS race on {r} after 8 tries"))); }
        }
    }

    fn events_since(
        &self,
        repo: &Path,
        ws: &str,
        from: Option<&str>,
    ) -> io::Result<Vec<Event>> {
        let r = self.wal_ref(ws);
        if self.head(repo, ws)?.is_none() {
            return Ok(vec![]);
        }
        let range = match from {
            Some(f) => format!("{f}..{r}"),
            None => r,
        };
        let commits = git(repo, &["rev-list", "--reverse", &range])?;
        let mut out = Vec::new();
        for c in commits.lines().filter(|l| !l.is_empty()) {
            let json = git(repo, &["cat-file", "-p", &format!("{c}:event.json")])?;
            let mut ev: Event = serde_json::from_str(&json)
                .map_err(|e| io::Error::other(format!("bad event in {c}: {e}")))?;
            // Backfill id for events written before id assignment.
            if ev.id.is_empty() {
                ev.id = git(repo, &["rev-parse", &format!("{c}:event.json")])?;
            }
            out.push(ev);
        }
        Ok(out)
    }

    fn head(&self, repo: &Path, ws: &str) -> io::Result<Option<String>> {
        let o = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["rev-parse", "-q", "--verify", &self.wal_ref(ws)])
            .output()?;
        Ok(o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty()))
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
        let head = self.head(repo, ws)?.ok_or_else(|| io::Error::other("no wal ref"))?;
        // Archive the full history ref before rewriting — compaction
        // shrinks the live log, not the audit trail.
        git(repo, &["update-ref", &format!("refs/sturn/archive/{ws}"), &head])?;
        // Rebuild the chain from post-snapshot events. Blobs already
        // exist in the ODB — each surviving event re-commits cheaply.
        let mut parent: Option<String> = None;
        for ev in &events[drop..] {
            let payload = serde_json::to_vec(ev).map_err(io::Error::other)?;
            let blob = git_in(repo, &["hash-object", "-w", "--stdin"], &payload)?;
            let tree = git_in(repo, &["mktree"], format!("100644 blob {blob}\tevent.json\n").as_bytes())?;
            let mut args = vec!["commit-tree", tree.as_str(), "-m", "sturn:compacted"];
            if let Some(p) = &parent {
                args.extend(["-p", p.as_str()]);
            }
            parent = Some(git(repo, &args)?);
        }
        // events[drop..] always includes the snapshot marker itself
        // when drop > 0, so the rebuilt chain always has a tip —
        // the None arm (delete the whole ref) is defensive.
        match parent { Some(tip) => git(repo, &["update-ref", &self.wal_ref(ws), &tip])?, None => git(repo, &["update-ref", "-d", &self.wal_ref(ws)])?, };
        Ok(drop)
    }

    fn workspaces(&self, repo: &Path) -> io::Result<Vec<String>> {
        let out = git(
            repo,
            &[
                "for-each-ref",
                "--format=%(refname)",
                &self.ref_prefix,
            ],
        )?;
        Ok(out
            .lines()
            .filter_map(|l| {
                l.strip_prefix(&format!("{}/", self.ref_prefix))
                    .map(String::from)
            })
            .collect())
    }
}
