//! Ws impl — reads/lists/entry lookups go through the shared
//! `lasius-vcs` port (loose-object fast path + plumbing fallback
//! inside lasius-vcs-git, P-049), `archive` materializes via
//! `exec_in` + `tar`. No index, no checkout, no lock contention.

use std::io;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use sturn_ws::Ws;

use lasius_vcs::Vcs;
use lasius_vcs_git::GitVcs;

/// ODB-backed workspaces. `tip` returns the ws ref commit — the WAL
/// overlay (uncommitted ops visible in `read`) lands in M4.
pub struct GitWs;

fn ws_ref(name: &str) -> String {
    format!("refs/sturn/ws/{name}")
}

impl Ws for GitWs {
    fn read(&self, repo: &Path, rev: &str, path: &str) -> io::Result<Vec<u8>> {
        GitVcs
            .read_at(repo, rev, path)?
            .ok_or_else(|| io::Error::other(format!("{rev}:{path} not found")))
    }

    fn list(&self, repo: &Path, rev: &str) -> io::Result<Vec<String>> {
        GitVcs.list_tree(repo, rev)
    }

    fn materialize(&self, repo: &Path, rev: &str, dir: &Path, paths: &[String]) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut args = vec!["archive".to_string(), rev.to_string()];
        if !paths.is_empty() {
            args.push("--".to_string());
            args.extend(paths.iter().cloned());
        }
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let tar = GitVcs.exec_in(repo, &arg_refs, &[])?;
        let mut tar_x = Command::new("tar")
            .args(["-x", "-C"])
            .arg(dir)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // take() closes stdin on drop — tar sees EOF after the bytes.
        let write = tar_x.stdin.take().map(|mut s| s.write_all(&tar));
        let to = tar_x.wait_with_output()?;
        if let Some(Err(e)) = write {
            return Err(e);
        }
        if to.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "tar -x failed: {}",
                String::from_utf8_lossy(&to.stderr).trim()
            )))
        }
    }

    fn create(&self, repo: &Path, name: &str, base: &str) -> io::Result<()> {
        GitVcs.update_ref(repo, &ws_ref(name), base, None)
    }

    fn entry(&self, repo: &Path, rev: &str, path: &str) -> io::Result<Option<String>> {
        GitVcs.entry_oid(repo, rev, path)
    }

    fn blob(&self, repo: &Path, sha: &str) -> io::Result<Vec<u8>> {
        match GitVcs.read_object(repo, sha)? {
            Some((_, bytes)) => Ok(bytes),
            None => Err(io::Error::other(format!("blob {sha} not found"))),
        }
    }

    fn store(&self, repo: &Path, content: &[u8]) -> io::Result<String> {
        GitVcs.write_object(repo, "blob", content)
    }

    fn tip(&self, repo: &Path, name: &str) -> io::Result<String> {
        GitVcs
            .resolve(repo, &ws_ref(name))
            .ok_or_else(|| io::Error::other(format!("{}: not found", ws_ref(name))))
    }

    fn workspaces(&self, repo: &Path) -> io::Result<Vec<String>> {
        let refs = GitVcs.list_refs(repo, "refs/sturn/ws/")?;
        Ok(refs
            .iter()
            .filter_map(|l| l.strip_prefix("refs/sturn/ws/").map(String::from))
            .collect())
    }
}
