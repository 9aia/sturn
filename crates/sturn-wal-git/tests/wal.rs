//! Tests for the git-backed WAL: append/read, delta reads,
//! workspaces listing, and compaction — all against real repos.

use std::path::{Path, PathBuf};
use std::process::Command;
use sturn_core::{Event, Op};
use sturn_wal::Wal;
use sturn_wal_git::GitWal;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sturn-wal-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

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
    let d = scratch(name);
    git(&d, &["init", "-q", "-b", "main"]);
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
            "--allow-empty",
        ],
    );
    d
}

fn ev(op: Op) -> Event {
    Event::new("w", "tester", op)
}

#[test]
fn append_and_read_back() {
    let repo = repo("append");
    let wal = GitWal::default();
    let id = wal
        .append(&repo, "ws1", &ev(Op::Intent { text: "why".into() }))
        .unwrap();
    assert!(!id.is_empty());

    let events = wal.events(&repo, "ws1").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].ws, "w");
    assert_eq!(events[0].actor, "tester");
    assert!(matches!(events[0].op, Op::Intent { .. }));
    assert_eq!(events[0].id, id);

    let head = wal.head(&repo, "ws1").unwrap();
    assert!(head.is_some());
}

#[test]
fn events_since_returns_delta() {
    let repo = repo("delta");
    let wal = GitWal::default();
    wal.append(&repo, "ws", &ev(Op::Intent { text: "one".into() }))
        .unwrap();
    let mid = wal.head(&repo, "ws").unwrap().unwrap();
    wal.append(&repo, "ws", &ev(Op::Intent { text: "two".into() }))
        .unwrap();
    wal.append(
        &repo,
        "ws",
        &ev(Op::Intent {
            text: "three".into(),
        }),
    )
    .unwrap();

    let delta = wal.events_since(&repo, "ws", Some(&mid)).unwrap();
    assert_eq!(delta.len(), 2);
    let all = wal.events(&repo, "ws").unwrap();
    assert_eq!(all.len(), 3);
}

#[test]
fn head_and_events_on_unknown_ws() {
    let repo = repo("unknown");
    let wal = GitWal::default();
    assert!(wal.head(&repo, "ghost").unwrap().is_none());
    assert!(wal.events(&repo, "ghost").unwrap().is_empty());
}

#[test]
fn workspaces_lists_ws_refs() {
    let repo = repo("wss");
    let wal = GitWal::default();
    wal.append(&repo, "alpha", &ev(Op::Intent { text: "t".into() }))
        .unwrap();
    wal.append(&repo, "beta", &ev(Op::Intent { text: "t".into() }))
        .unwrap();
    let mut ws = wal.workspaces(&repo).unwrap();
    ws.sort();
    assert_eq!(ws, vec!["alpha", "beta"]);
}

#[test]
fn compact_drops_pre_snapshot_history() {
    let repo = repo("compact");
    let wal = GitWal::default();
    wal.append(
        &repo,
        "ws",
        &ev(Op::Write {
            path: "a".into(),
            blob: "x".into(),
            intent: None,
        }),
    )
    .unwrap();
    wal.append(
        &repo,
        "ws",
        &ev(Op::Snapshot {
            message: "s".into(),
        }),
    )
    .unwrap();
    wal.append(
        &repo,
        "ws",
        &ev(Op::Write {
            path: "b".into(),
            blob: "y".into(),
            intent: None,
        }),
    )
    .unwrap();

    let dropped = wal.compact(&repo, "ws").unwrap();
    assert_eq!(dropped, 1);
    let events = wal.events(&repo, "ws").unwrap();
    // Snapshot marker + post-snapshot op survive.
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0].op, Op::Snapshot { .. }));
    // The audit trail moved to the archive ref.
    let archived = git(
        &repo,
        &["rev-parse", "-q", "--verify", "refs/sturn/archive/ws"],
    );
    assert!(!archived.is_empty());
}

#[test]
fn compact_without_snapshot_is_noop() {
    let repo = repo("compact-noop");
    let wal = GitWal::default();
    wal.append(&repo, "ws", &ev(Op::Intent { text: "t".into() }))
        .unwrap();
    assert_eq!(wal.compact(&repo, "ws").unwrap(), 0);
}

#[test]
fn custom_ref_prefix() {
    let repo = repo("prefix");
    let wal = GitWal {
        ref_prefix: "refs/custom/wal".into(),
        sign_key: None,
    };
    wal.append(&repo, "ws", &ev(Op::Intent { text: "t".into() }))
        .unwrap();
    assert!(wal.head(&repo, "ws").unwrap().is_some());
    assert_eq!(wal.workspaces(&repo).unwrap(), vec!["ws"]);
}
