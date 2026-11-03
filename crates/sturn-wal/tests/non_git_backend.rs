//! T-004 spike: can the `Wal`/`Vcs` ports be implemented without any
//! git semantics at all? `FileWal` keeps each ws log as JSONL under a
//! plain dir; `head` is a sequence cursor, `append` linearizes via a
//! per-ws lock file. `FlatVcs` "replicates" by copying that dir.
//!
//! Findings this PoC demonstrates:
//! - The trait *signatures* port cleanly — `head`/`events_since`
//!   cursors are opaque `String`s to every caller (cli round-trips
//!   them); git commit shas are only one possible token.
//! - What *doesn't* port is vocabulary in the docs: "commit", "ref",
//!   "CAS on the ref", "heads AND refs/sturn/*". A non-git backend
//!   implements the same contract with different mechanisms
//!   (lock-file linearization instead of `update-ref` CAS).
//! - `append` returning the event id is generic — a content hash of
//!   the event bytes works; it needn't be a *git* blob sha.

use std::io;
use std::path::{Path, PathBuf};
use sturn_core::{Event, Op};
use sturn_vcs::Vcs;
use sturn_wal::Wal;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sturn-ng-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// FNV-1a over the event JSON — a content address that owes nothing
/// to git's sha1 object ids.
fn content_id(payload: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in payload {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// JSONL-per-workspace WAL. `repo` is any dir; `refs/sturn/wal/<ws>`
/// maps to `<repo>/wal/<ws>.jsonl`. `head` returns the line count —
/// an opaque cursor, not a commit.
struct FileWal;

impl FileWal {
    fn log(&self, repo: &Path, ws: &str) -> PathBuf {
        repo.join("wal").join(format!("{ws}.jsonl"))
    }

    fn lines(&self, repo: &Path, ws: &str) -> io::Result<Vec<Event>> {
        match std::fs::read_to_string(self.log(repo, ws)) {
            Ok(s) => s
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| serde_json::from_str(l).map_err(io::Error::other))
                .collect(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e),
        }
    }
}

impl Wal for FileWal {
    fn append(&self, repo: &Path, ws: &str, ev: &Event) -> io::Result<String> {
        std::fs::create_dir_all(self.log(repo, ws).parent().unwrap())?;
        // CAS-equivalent: exclusive create on a lock file serializes
        // appenders; losers retry. The port only requires "survive
        // concurrent appends" — the mechanism is the impl's choice.
        let lock = self.log(repo, ws).with_extension("lock");
        let mut tries = 0u16;
        loop {
            match std::fs::File::create_new(&lock) {
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && tries < 1000 => {
                    tries += 1;
                    std::thread::yield_now();
                }
                Err(e) => return Err(e),
            }
        }
        let r = (|| {
            let mut ev = ev.clone();
            let payload = serde_json::to_vec(&ev).map_err(io::Error::other)?;
            ev.id = content_id(&payload);
            let line = serde_json::to_string(&ev).map_err(io::Error::other)?;
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.log(repo, ws))?;
            writeln!(f, "{line}")?;
            Ok(ev.id)
        })();
        let _ = std::fs::remove_file(&lock);
        r
    }

    fn events_since(&self, repo: &Path, ws: &str, from: Option<&str>) -> io::Result<Vec<Event>> {
        let all = self.lines(repo, ws)?;
        match from {
            None => Ok(all),
            // `from` is a prior `head` token — here a line count. A
            // foreign token (e.g. a git sha) is an error, same as
            // `rev-list <bad-sha>..ref` failing under GitWal.
            Some(f) => match f.parse::<usize>() {
                Ok(n) => Ok(all.into_iter().skip(n).collect()),
                Err(_) => Err(io::Error::other(format!("foreign cursor {f}"))),
            },
        }
    }

    fn head(&self, repo: &Path, ws: &str) -> io::Result<Option<String>> {
        let n = self.lines(repo, ws)?.len();
        Ok((n > 0).then(|| n.to_string()))
    }

    fn workspaces(&self, repo: &Path) -> io::Result<Vec<String>> {
        let dir = repo.join("wal");
        let mut out = vec![];
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if let Some(n) = e.file_name().to_str().and_then(|s| s.strip_suffix(".jsonl")) {
                    out.push(n.to_string());
                }
            }
        }
        out.sort();
        Ok(out)
    }

    fn compact(&self, repo: &Path, ws: &str) -> io::Result<usize> {
        let events = self.lines(repo, ws)?;
        let drop = sturn_core::since_last_snapshot(&events).saturating_sub(1);
        if drop == 0 {
            return Ok(0);
        }
        std::fs::create_dir_all(repo.join("archive"))?;
        std::fs::rename(
            self.log(repo, ws),
            repo.join("archive").join(format!("{ws}.jsonl")),
        )?;
        let mut out = String::new();
        for ev in &events[drop..] {
            out.push_str(&serde_json::to_string(ev).map_err(io::Error::other)?);
            out.push('\n');
        }
        std::fs::write(self.log(repo, ws), out)?;
        Ok(drop)
    }
}

/// "Remote" = another dir. sync pulls `wal/` in, push copies `wal/`
/// plus `heads/` out — the port's "heads AND refs/sturn/*" contract
/// expressed without refs at all.
struct FlatVcs;

impl Vcs for FlatVcs {
    fn sync(&self, repo: &Path) -> bool {
        let remote = repo.join("remote");
        copy_tree(&remote.join("wal"), &repo.join("wal"))
    }
    fn has_commit(&self, repo: &Path, sha: &str) -> bool {
        // "object exists" maps to any store lookup — here, a file.
        repo.join("objects").join(sha).is_file()
    }
    fn push(&self, repo: &Path, _remote: &str) -> bool {
        let dst = repo.join("remote");
        copy_tree(&repo.join("wal"), &dst.join("wal"))
            && copy_tree(&repo.join("heads"), &dst.join("heads"))
    }
}

fn copy_tree(src: &Path, dst: &Path) -> bool {
    if !src.exists() {
        return true;
    }
    if std::fs::create_dir_all(dst).is_err() {
        return false;
    }
    for e in std::fs::read_dir(src).into_iter().flatten().flatten() {
        if std::fs::copy(e.path(), dst.join(e.file_name())).is_err() {
            return false;
        }
    }
    true
}

#[test]
fn wal_port_is_git_free() {
    let dir = scratch("wal");
    let wal = FileWal;

    assert!(wal.head(&dir, "w1").unwrap().is_none());
    assert!(wal.events(&dir, "w1").unwrap().is_empty());
    assert!(wal.workspaces(&dir).unwrap().is_empty());

    let id1 = wal
        .append(&dir, "w1", &Event::new("w1", "a", Op::Intent { text: "hi".into() }))
        .unwrap();
    let id2 = wal
        .append(&dir, "w1", &Event::new("w1", "a", Op::Snapshot { message: "s".into() }))
        .unwrap();
    assert_ne!(id1, id2);
    assert_eq!(id1.len(), 16); // fnv hex, not a git sha

    let head = wal.head(&dir, "w1").unwrap().unwrap();
    let all = wal.events(&dir, "w1").unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, id1);
    assert!(wal.events_since(&dir, "w1", Some(&head)).unwrap().is_empty());
    assert_eq!(wal.events_since(&dir, "w1", Some("0")).unwrap().len(), 2);
    assert!(wal.events_since(&dir, "w1", Some("not-a-cursor")).is_err());
    assert_eq!(wal.workspaces(&dir).unwrap(), vec!["w1".to_string()]);

    // compact drops pre-snapshot history, archives the rest.
    assert_eq!(wal.compact(&dir, "w1").unwrap(), 1);
    assert_eq!(wal.events(&dir, "w1").unwrap().len(), 1);
    assert!(dir.join("archive").join("w1.jsonl").is_file());
}

#[test]
fn wal_append_is_atomic_under_contention() {
    let dir = scratch("race");
    std::thread::scope(|s| {
        for t in 0..4 {
            let dir = &dir;
            s.spawn(move || {
                for i in 0..10 {
                    FileWal
                        .append(
                            dir,
                            "w",
                            &Event::new("w", format!("t{t}"), Op::Intent { text: i.to_string() }),
                        )
                        .unwrap();
                }
            });
        }
    });
    assert_eq!(FileWal.events(&dir, "w").unwrap().len(), 40);
    assert_eq!(FileWal.head(&dir, "w").unwrap().as_deref(), Some("40"));
}

#[test]
fn vcs_port_is_git_free() {
    let dir = scratch("vcs");
    let vcs = FlatVcs;
    let wal = FileWal;
    wal.append(&dir, "w", &Event::new("w", "a", Op::Intent { text: "x".into() }))
        .unwrap();

    assert!(!vcs.has_commit(&dir, "deadbeef"));
    std::fs::create_dir_all(dir.join("objects")).unwrap();
    std::fs::write(dir.join("objects").join("deadbeef"), b"").unwrap();
    assert!(vcs.has_commit(&dir, "deadbeef"));

    assert!(vcs.push(&dir, "origin"));
    assert!(dir.join("remote").join("wal").join("w.jsonl").is_file());
    assert!(vcs.sync(&dir));
}
