//! Ws impl via git plumbing — `cat-file` reads, `ls-tree` lists,
//! `archive | tar` materializes. The ODB does the work; no index,
//! no checkout, no lock contention.

use std::io;
use std::io::Write;
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

    fn entry(&self, repo: &Path, rev: &str, path: &str) -> io::Result<Option<String>> {
        let o = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["rev-parse", "-q", "--verify", &format!("{rev}:{path}")])
            .output()?;
        Ok(o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty()))
    }

    fn blob(&self, repo: &Path, sha: &str) -> io::Result<Vec<u8>> {
        let o = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["cat-file", "-p", sha])
            .output()?;
        if o.status.success() {
            Ok(o.stdout)
        } else {
            Err(io::Error::other(format!("blob {sha} not found")))
        }
    }

    fn store(&self, repo: &Path, content: &[u8]) -> io::Result<String> {
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

    fn tip(&self, repo: &Path, name: &str) -> io::Result<String> {
        git(repo, &["rev-parse", &ws_ref(name)])
    }

    fn workspaces(&self, repo: &Path) -> io::Result<Vec<String>> {
        let out = git(
            repo,
            &["for-each-ref", "--format=%(refname)", "refs/sturn/ws/"],
        )?;
        Ok(out
            .lines()
            .filter_map(|l| l.strip_prefix("refs/sturn/ws/").map(String::from))
            .collect())
    }
}
