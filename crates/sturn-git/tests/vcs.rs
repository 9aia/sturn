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
    // Repo-local identity — commit-tree needs one and the test
    // env may have no global git config.
    git(&d, &["config", "user.email", "t@t"]);
    git(&d, &["config", "user.name", "t"]);
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

// ---------- coverage extensions ----------

#[test]
fn vcs_failures_return_false() {
    let dir = scratch("vcs-false");
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init", "--allow-empty"]);
    let vcs = GitVcs;
    // No remote configured → sync and push fail closed (false).
    assert!(!vcs.sync(&dir));
    assert!(!vcs.push(&dir, "origin"));
    // Missing object → false.
    assert!(!vcs.has_commit(&dir, &"0".repeat(40)));
}

#[test]
fn store_blob_and_snap_errors() {
    // hash-object in a bogus repo → error.
    assert!(sturn_git::store_blob(Path::new("/no/such/repo-xyz"), b"x").is_err());
    // snap_commit on a bogus base → read-tree error.
    let dir = scratch("snap-err");
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init", "--allow-empty"]);
    assert!(sturn_git::snap_commit(&dir, "bogus-base", &[], "m", "main").is_err());
}

#[test]
fn snap_commit_write_then_rename_uses_seen() {
    let dir = scratch("snap-seen");
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.email", "t@t"]);
    git(&dir, &["config", "user.name", "t"]);
    git(&dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init", "--allow-empty"]);
    // Write then rename in the SAME batch — the rename's `from`
    // never existed in base; `seen` supplies the blob.
    let blob = sturn_git::store_blob(&dir, b"content\n").unwrap();
    let events = vec![
        Event::new(
            "w",
            "t",
            Op::Write {
                path: "tmp.txt".into(),
                blob: blob.clone(),
                intent: None,
            },
        ),
        Event::new(
            "w",
            "t",
            Op::Rename {
                from: "tmp.txt".into(),
                to: "final.txt".into(),
            },
        ),
        // Non-tree ops are skipped by the overlay walk.
        Event::new("w", "t", Op::Intent { text: "meta".into() }),
        Event::new(
            "w",
            "t",
            Op::Claim {
                glob: "x".into(),
                ttl_secs: 1,
            },
        ),
    ];
    let base = git(&dir, &["rev-parse", "HEAD"]);
    sturn_git::snap_commit(&dir, &base, &events, "m", "main").unwrap();
    assert_eq!(git(&dir, &["show", "HEAD:final.txt"]), "content");
    // Renamed-to path then deleted in the same batch → seen updates.
    let events = vec![
        Event::new(
            "w",
            "t",
            Op::Rename {
                from: "final.txt".into(),
                to: "gone.txt".into(),
            },
        ),
        Event::new(
            "w",
            "t",
            Op::Delete {
                path: "gone.txt".into(),
            },
        ),
    ];
    let base = git(&dir, &["rev-parse", "HEAD"]);
    sturn_git::snap_commit(&dir, &base, &events, "m", "main").unwrap();
    let o = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["cat-file", "-e", "HEAD:gone.txt"])
        .output()
        .unwrap();
    assert!(!o.status.success(), "gone.txt should be deleted");
}
