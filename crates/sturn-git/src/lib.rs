//! Git impl of `Vcs`, plus the snap bridge: fold WAL ops into a
//! real commit — the crossing from the evented world back to
//! ordinary git history. `snap` is why GitHub sees a normal repo.

use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use sturn_core::{Event, Op};
use sturn_vcs::Vcs;

pub struct GitVcs;

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

impl Vcs for GitVcs {
    fn sync(&self, repo: &Path) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["fetch", "origin", "--prune"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn has_commit(&self, repo: &Path, sha: &str) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["cat-file", "-e", sha])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn push(&self, repo: &Path, remote: &str) -> bool {
        let heads = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["push", remote, "--all"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let sturn = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["push", remote, "refs/sturn/*:refs/sturn/*"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        heads && sturn
    }
}

/// Store `content` as a blob in the ODB; returns its sha.
pub fn store_blob(repo: &Path, content: &[u8]) -> io::Result<String> {
    let mut c = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["hash-object", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    c.stdin.take().unwrap().write_all(content)?;
    let o = c.wait_with_output()?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(io::Error::other("hash-object failed"))
    }
}

/// Overlay `events`' Write/Rename/Delete ops on `base`, commit to
/// `branch`, return the new commit sha. Coordination ops (claims,
/// intents, reviews) are metadata — they never touch the tree.
///
/// Uses a scratch index (`GIT_INDEX_FILE`) — no working tree, no
/// checkout, no lock on the user's index.
pub fn snap_commit(
    repo: &Path,
    base: &str,
    events: &[Event],
    message: &str,
    branch: &str,
) -> io::Result<String> {
    let idx: PathBuf =
        std::env::temp_dir().join(format!("sturn-index-{}", std::process::id()));
    let gi = |args: &[&str]| -> io::Result<String> {
        let o = Command::new("git")
            .arg("-C")
            .arg(repo)
            .env("GIT_INDEX_FILE", &idx)
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
    };

    gi(&["read-tree", base])?;
    for ev in events {
        match &ev.op {
            Op::Write { path, blob, .. } => {
                gi(&[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("100644,{blob},{path}"),
                ])?;
            }
            Op::Delete { path } => {
                gi(&["update-index", "--force-remove", path])?;
            }
            Op::Rename { from, to } => {
                let blob = git(repo, &["rev-parse", &format!("{base}:{from}")])?;
                gi(&["update-index", "--force-remove", from])?;
                gi(&[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("100644,{blob},{to}"),
                ])?;
            }
            _ => {}
        }
    }
    let tree = gi(&["write-tree"])?;
    let _ = std::fs::remove_file(&idx);
    let commit = git(repo, &["commit-tree", &tree, "-p", base, "-m", message])?;
    git(repo, &["update-ref", &format!("refs/heads/{branch}"), &commit])?;
    Ok(commit)
}
