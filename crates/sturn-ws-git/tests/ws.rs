//! Tests for the git-backed Ws impl: workspace refs, blob reads,
//! tree listing, entry lookup, and materialize — real ODB, no
//! checkout.

use std::path::{Path, PathBuf};
use std::process::Command;
use sturn_ws::Ws;
use sturn_ws_git::GitWs;

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn repo(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sturn-ws-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    git(&d, &["init", "-q", "-b", "main"]);
    std::fs::write(d.join("a.txt"), b"hello\n").unwrap();
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("src/b.rs"), b"fn main() {}\n").unwrap();
    git(&d, &["add", "-A"]);
    git(
        &d,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ],
    );
    d
}

#[test]
fn create_tip_and_workspaces() {
    let repo = repo("create");
    let ws = GitWs;
    let base = git(&repo, &["rev-parse", "HEAD"]);
    ws.create(&repo, "agent-1", &base).unwrap();
    ws.create(&repo, "agent-2", &base).unwrap();
    assert_eq!(ws.tip(&repo, "agent-1").unwrap(), base);
    let mut names = ws.workspaces(&repo).unwrap();
    names.sort();
    assert_eq!(names, vec!["agent-1", "agent-2"]);
}

#[test]
fn read_list_and_entry() {
    let repo = repo("read");
    let ws = GitWs;
    let tip = git(&repo, &["rev-parse", "HEAD"]);
    assert_eq!(ws.read(&repo, &tip, "a.txt").unwrap(), b"hello\n");
    assert_eq!(ws.read(&repo, &tip, "src/b.rs").unwrap(), b"fn main() {}\n");
    assert!(ws.read(&repo, &tip, "nope").is_err());

    let mut paths = ws.list(&repo, &tip).unwrap();
    paths.sort();
    assert_eq!(paths, vec!["a.txt", "src/b.rs"]);

    assert!(ws.entry(&repo, &tip, "a.txt").unwrap().is_some());
    assert!(ws.entry(&repo, &tip, "missing").unwrap().is_none());
}

#[test]
fn store_and_blob_roundtrip() {
    let repo = repo("blob");
    let ws = GitWs;
    let sha = ws.store(&repo, b"contents").unwrap();
    assert_eq!(ws.blob(&repo, &sha).unwrap(), b"contents");
    assert!(ws
        .blob(&repo, "0000000000000000000000000000000000000000")
        .is_err());
}

#[test]
fn materialize_whole_and_subset() {
    let repo = repo("mat");
    let ws = GitWs;
    let tip = git(&repo, &["rev-parse", "HEAD"]);
    let out = std::env::temp_dir().join(format!("sturn-ws-mat-{}-out", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);

    ws.materialize(&repo, &tip, &out, &[]).unwrap();
    assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"hello\n");
    assert!(out.join("src/b.rs").exists());

    let sub = std::env::temp_dir().join(format!("sturn-ws-mat-{}-sub", std::process::id()));
    let _ = std::fs::remove_dir_all(&sub);
    ws.materialize(&repo, &tip, &sub, &["a.txt".to_string()])
        .unwrap();
    assert!(sub.join("a.txt").exists());
    assert!(!sub.join("src/b.rs").exists());
}

#[test]
fn tip_missing_ws_errors() {
    let repo = repo("tip-miss");
    assert!(GitWs.tip(&repo, "ghost").is_err());
}
