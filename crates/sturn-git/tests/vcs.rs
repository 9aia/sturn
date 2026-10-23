//! Tests for the git-backed Vcs impl and the snap bridge:
//! store_blob, snap_commit folding, has_commit, push to a bare
//! remote, sync.

use std::path::{Path, PathBuf};
use std::process::Command;
use sturn_core::{Event, Op};
use sturn_git::{snap_commit, store_blob, GitVcs};
use sturn_vcs::Vcs;

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

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sturn-git-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn repo(name: &str) -> PathBuf {
    let d = scratch(name);
    git(&d, &["init", "-q", "-b", "main"]);
    std::fs::write(d.join("a.txt"), b"base\n").unwrap();
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

fn ev(op: Op) -> Event {
    Event::new("ws", "tester", op)
}

#[test]
fn store_blob_writes_object() {
    let repo = repo("blob");
    let sha = store_blob(&repo, b"data").unwrap();
    assert_eq!(sha.len(), 40);
    assert_eq!(git(&repo, &["cat-file", "-p", &sha]), "data");
}

#[test]
fn has_commit_checks_objects() {
    let repo = repo("has");
    let vcs = GitVcs;
    let tip = git(&repo, &["rev-parse", "HEAD"]);
    assert!(vcs.has_commit(&repo, &tip));
    assert!(!vcs.has_commit(&repo, "0000000000000000000000000000000000000000"));
}

#[test]
fn snap_commit_folds_tree_ops() {
    let repo = repo("snap");
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let blob = store_blob(&repo, b"new contents\n").unwrap();
    let events = vec![
        ev(Op::Write {
            path: "b.txt".into(),
            blob: blob.clone(),
            intent: None,
        }),
        ev(Op::Rename {
            from: "a.txt".into(),
            to: "moved.txt".into(),
        }),
        // Coordination ops never touch the tree.
        ev(Op::Claim {
            glob: "src/**".into(),
            ttl_secs: 60,
        }),
        ev(Op::Intent { text: "why".into() }),
    ];
    let commit = snap_commit(&repo, &base, &events, "snap it", "main").unwrap();
    assert_eq!(git(&repo, &["rev-parse", "main"]), commit);
    let names = git(&repo, &["show", "--format=", "--name-only", &commit]);
    assert!(names.contains("b.txt"));
    assert!(names.contains("moved.txt"));
    assert_eq!(
        git(&repo, &["cat-file", "-p", &format!("{commit}:b.txt")]),
        "new contents"
    );
    assert_eq!(
        git(&repo, &["cat-file", "-p", &format!("{commit}:moved.txt")]),
        "base"
    );
    // Source is gone.
    assert!(Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["cat-file", "-e", &format!("{commit}:a.txt")])
        .status()
        .map(|s| !s.success())
        .unwrap_or(false));
}

#[test]
fn snap_commit_deletes_paths() {
    let repo = repo("snapdel");
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let events = vec![ev(Op::Delete {
        path: "a.txt".into(),
    })];
    let commit = snap_commit(&repo, &base, &events, "del", "main").unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["cat-file", "-e", &format!("{commit}:a.txt")])
        .status()
        .map(|s| !s.success())
        .unwrap_or(false));
}

#[test]
fn snap_commit_rename_resolves_from_base() {
    let repo = repo("snapren");
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let events = vec![ev(Op::Rename {
        from: "a.txt".into(),
        to: "b.txt".into(),
    })];
    let commit = snap_commit(&repo, &base, &events, "mv", "main").unwrap();
    assert_eq!(
        git(&repo, &["cat-file", "-p", &format!("{commit}:b.txt")]),
        "base"
    );
}

#[test]
fn push_and_sync_bare_remote() {
    let repo = repo("push-src");
    let remote = scratch("push-dst");
    git(&remote, &["init", "-q", "--bare"]);
    git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );

    // Give the repo a sturn ref so the coordination push has content.
    let commit = git(
        &repo,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit-tree",
            &git(&repo, &["write-tree"]),
            "-m",
            "wal",
        ],
    );
    git(&repo, &["update-ref", "refs/sturn/wal/ws1", &commit]);

    let vcs = GitVcs;
    assert!(vcs.push(&repo, "origin"));
    assert!(vcs.sync(&repo));

    // The remote now has both the branch and the WAL ref.
    let refs = git(&remote, &["for-each-ref", "--format=%(refname)"]);
    assert!(refs.contains("refs/heads/main"));
    assert!(refs.contains("refs/sturn/wal/ws1"));
}
