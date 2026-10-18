//! e2e: the M1+M2 promise — ops land in git refs, snap commits are
//! real, claims prevent collisions across workspaces, and a fresh
//! clone replays the WAL with nothing but what pushed in refs.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

fn sturn(dir: &Path, args: &[&str]) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_sturn"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "sturn {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn sturn_in(dir: &Path, args: &[&str], stdin: &[u8]) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sturn"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(stdin).unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(
        o.status.success(),
        "sturn {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn sturn_fails(dir: &Path, args: &[&str]) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_sturn"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(!o.status.success(), "sturn {} unexpectedly ok", args.join(" "));
    String::from_utf8_lossy(&o.stderr).trim().to_string()
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sturn-e2e-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn commit(dir: &Path, msg: &str) {
    git(
        dir,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            msg,
        ],
    );
}

#[test]
fn wal_roundtrip_and_replication() {
    let repo = scratch("repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(
        &repo,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
            "--allow-empty",
        ],
    );
    std::fs::write(repo.join("a.txt"), b"one\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    commit(&repo, "add a");

    sturn(&repo, &["init"]);
    assert!(repo.join("sturn.toml").exists());

    sturn(&repo, &["ws", "new", "agent-1"]);
    sturn_in(&repo, &["write", "b.txt", "--ws", "agent-1"], b"two\n");
    sturn(&repo, &["claim", "*.txt", "--ws", "agent-1"]);

    // Cross-workspace claim conflict — prevented, not merged.
    sturn(&repo, &["ws", "new", "agent-2"]);
    let err = sturn_fails(&repo, &["claim", "b.txt", "--ws", "agent-2"]);
    assert!(err.contains("conflict"), "{err}");

    // Snap produces an ordinary commit with the ops applied.
    sturn(&repo, &["snap", "agent-1", "-m", "work"]);
    assert_eq!(git(&repo, &["show", "HEAD:b.txt"]), "two");

    // The WAL is plain git refs — a clone replays it verbatim.
    let clone = scratch("clone");
    git(&repo, &["clone", "-q", ".", clone.to_str().unwrap()]);
    git(
        &clone,
        &["fetch", "origin", "refs/sturn/*:refs/sturn/*"],
    );
    let log = sturn(&clone, &["log", "--ws", "agent-1"]);
    assert!(log.contains("\"write\"") && log.contains("b.txt"), "{log}");
    assert!(log.contains("\"snapshot\""), "{log}");
}

#[test]
fn second_snap_applies_only_new_ops() {
    let repo = scratch("twosnap");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(
        &repo,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
            "--allow-empty",
        ],
    );
    sturn(&repo, &["init"]);
    sturn(&repo, &["ws", "new", "a"]);
    sturn_in(&repo, &["write", "x.txt", "--ws", "a"], b"1\n");
    sturn(&repo, &["snap", "a", "-m", "first"]);
    let first = git(&repo, &["rev-parse", "HEAD"]);
    sturn_in(&repo, &["write", "y.txt", "--ws", "a"], b"2\n");
    sturn(&repo, &["snap", "a", "-m", "second"]);
    assert_eq!(git(&repo, &["show", "HEAD:y.txt"]), "2");
    assert_eq!(git(&repo, &["show", "HEAD:x.txt"]), "1");
    // `first` is a proper parent — history is linear and ordinary.
    assert_eq!(git(&repo, &["rev-parse", "HEAD^"]), first);
}
