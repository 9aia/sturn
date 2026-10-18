//! Ws impl via git plumbing — `cat-file` reads, `ls-tree` lists,
//! `archive | tar` materializes. The ODB does the work; no index,
//! no checkout, no lock contention.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use sturn_ws::Ws;

/// ODB-backed workspaces. `tip` returns the ws ref commit — the WAL
/// overlay (uncommitted ops visible in `read`) lands in M4.
pub struct GitWs;

fn git(repo: &Path, args: &[&str]) -> io::Result<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim_end().to_string())
    } else {
        Err(io::Error::other(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        )))
    }
}

fn ws_ref(name: &str) -> String {
    format!("refs/sturn/ws/{name}")
}

impl Ws for GitWs {
    fn read(&self, repo: &Path, rev: &str, path: &str) -> io::Result<Vec<u8>> {
        let o = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["cat-file", "-p", &format!("{rev}:{path}")])
            .output()?;
        if o.status.success() {
            Ok(o.stdout)
        } else {
            Err(io::Error::other(format!("{rev}:{path} not found")))
        }
    }

    fn list(&self, repo: &Path, rev: &str) -> io::Result<Vec<String>> {
        let out = git(repo, &["ls-tree", "-r", "--name-only", rev])?;
        Ok(out.lines().map(String::from).collect())
    }

    fn materialize(&self, repo: &Path, rev: &str, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut arc = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["archive", rev])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let tar = Command::new("tar")
            .args(["-x", "-C"])
            .arg(dir)
            .stdin(Stdio::from(arc.stdout.take().unwrap()))
            .stderr(Stdio::piped())
            .spawn()?;
        let st = arc.wait()?;
        let to = tar.wait_with_output()?;
        if st.success() && to.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "archive|tar failed: {}",
                String::from_utf8_lossy(&to.stderr).trim()
            )))
        }
    }

    fn create(&self, repo: &Path, name: &str, base: &str) -> io::Result<()> {
        git(repo, &["update-ref", &ws_ref(name), base])?;
        Ok(())
    }

    fn tip(&self, repo: &Path, name: &str) -> io::Result<String> {
        git(repo, &["rev-parse", &ws_ref(name)])
    }
}
