//! FUSE integration tests — real mounts on /dev/fuse over in-memory
//! Ws/Wal fakes. Read-only mounts serve the tree; RW mounts turn
//! edits into Write/Delete/Rename ops.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use sturn_core::Event;
use sturn_fuse::SturnFs;
use sturn_wal::Wal;
use sturn_ws::Ws;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sturn-fuse-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Wait until the mountpoint answers a readdir (the FUSE session is
/// live once `mount2` returns, but the VFS handshake lags a tick).
fn wait_mounted(mp: &Path) {
    for _ in 0..500 {
        if std::fs::read_dir(mp)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("mount at {} never came up", mp.display());
}

fn unmount(mp: &Path) {
    for i in 0..10 {
        // Plain -u: the kernel revokes and the session exits
        // deterministically (lazy -z can leave the session's own
        // mount reference alive and hang the scope join).
        let o = Command::new("fusermount3").arg("-u").arg(mp).output().unwrap();
        if o.status.success() {
            return;
        }
        if i == 9 {
            panic!("fusermount3 -u: {}", String::from_utf8_lossy(&o.stderr));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// In-memory Ws — a tip "tree" plus a blob store.
struct MemWs {
    files: BTreeMap<String, Vec<u8>>,
    blobs: Mutex<BTreeMap<String, Vec<u8>>>,
    n: Mutex<u64>,
}

impl MemWs {
    fn new() -> Self {
        let mut files = BTreeMap::new();
        files.insert("a.txt".into(), b"alpha\n".to_vec());
        files.insert("dir/nested.txt".into(), b"nested\n".to_vec());
        Self {
            files,
            blobs: Mutex::new(BTreeMap::new()),
            n: Mutex::new(0),
        }
    }
}

impl Ws for MemWs {
    fn read(&self, _r: &Path, _rev: &str, path: &str) -> std::io::Result<Vec<u8>> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| std::io::Error::other("not found"))
    }
    fn list(&self, _r: &Path, _rev: &str) -> std::io::Result<Vec<String>> {
        Ok(self.files.keys().cloned().collect())
    }
    fn materialize(&self, _r: &Path, _rev: &str, _d: &Path, _p: &[String]) -> std::io::Result<()> {
        Ok(())
    }
    fn create(&self, _r: &Path, _n: &str, _b: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn entry(&self, _r: &Path, _rev: &str, path: &str) -> std::io::Result<Option<String>> {
        Ok(self.files.get(path).map(|_| format!("sha-{path}")))
    }
    fn blob(&self, _r: &Path, sha: &str) -> std::io::Result<Vec<u8>> {
        self.blobs
            .lock()
            .unwrap()
            .get(sha)
            .cloned()
            .ok_or_else(|| std::io::Error::other("no blob"))
    }
    fn store(&self, _r: &Path, content: &[u8]) -> std::io::Result<String> {
        let mut n = self.n.lock().unwrap();
        *n += 1;
        let sha = format!("blob{n}");
        self.blobs.lock().unwrap().insert(sha.clone(), content.to_vec());
        Ok(sha)
    }
    fn tip(&self, _r: &Path, _n: &str) -> std::io::Result<String> {
        Ok("tip".into())
    }
    fn workspaces(&self, _r: &Path) -> std::io::Result<Vec<String>> {
        Ok(vec!["w".into()])
    }
}

/// In-memory Wal — records every appended event.
struct MemWal {
    events: Mutex<Vec<Event>>,
}

impl MemWal {
    fn new() -> Self {
        Self {
            events: Mutex::new(vec![]),
        }
    }
}

impl Wal for MemWal {
    fn append(&self, _r: &Path, _ws: &str, e: &Event) -> std::io::Result<String> {
        let mut ev = e.clone();
        let n = self.events.lock().unwrap().len() + 1;
        ev.id = format!("e{n}");
        self.events.lock().unwrap().push(ev.clone());
        Ok(ev.id)
    }
    fn events_since(&self, _r: &Path, _ws: &str, from: Option<&str>) -> std::io::Result<Vec<Event>> {
        let all = self.events.lock().unwrap().clone();
        Ok(match from {
            None => all,
            Some(f) => {
                let pos = all.iter().position(|e| e.id == f).map_or(0, |i| i + 1);
                all[pos..].to_vec()
            }
        })
    }
    fn head(&self, _r: &Path, _ws: &str) -> std::io::Result<Option<String>> {
        Ok(self.events.lock().unwrap().last().map(|e| e.id.clone()))
    }
    fn workspaces(&self, _r: &Path) -> std::io::Result<Vec<String>> {
        Ok(vec!["w".into()])
    }
    fn compact(&self, _r: &Path, _ws: &str) -> std::io::Result<usize> {
        Ok(0)
    }
}

#[test]
fn read_only_mount_serves_tree() {
    let wsi = MemWs::new();
    let mp = scratch("ro-mp");

    std::thread::scope(|s| {
        let fs = SturnFs::new(Path::new("/repo"), "tip", &wsi).unwrap();
        let mp2 = mp.clone();
        let h = s.spawn(move || fs.mount(&mp2));
        wait_mounted(&mp);

        // lookup + getattr + read through the real kernel path.
        assert_eq!(std::fs::read_to_string(mp.join("a.txt")).unwrap(), "alpha\n");
        assert_eq!(
            std::fs::read_to_string(mp.join("dir/nested.txt")).unwrap(),
            "nested\n"
        );
        assert!(std::fs::metadata(mp.join("a.txt")).unwrap().is_file());
        assert!(std::fs::metadata(mp.join("dir")).unwrap().is_dir());
        // lookup ENOENT.
        assert!(!mp.join("nope.txt").exists());
        // readdir.
        let names: Vec<_> = std::fs::read_dir(&mp)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"a.txt".to_string()) && names.contains(&"dir".to_string()));
        let sub: Vec<_> = std::fs::read_dir(mp.join("dir"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(sub, vec!["nested.txt".to_string()]);
        // Offset read through a std::fs::File.
        let mut f = std::fs::File::open(mp.join("a.txt")).unwrap();
        let mut buf = [0u8; 3];
        std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(1)).unwrap();
        let n = f.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"lph");
        // Writes on a RO mount are refused by the VFS itself.
        assert!(std::fs::write(mp.join("x.txt"), b"x").is_err());
        // An open handle makes fusermount3 -u return EBUSY — and a
        // failed unmount deadlocks the scope join on the live mount.
        drop(f);

        unmount(&mp);
        h.join().unwrap().unwrap();
    });
}

#[test]
fn build_runs_over_tree_without_mounting() {
    // Construction alone walks the whole tree — root, nested dirs,
    // per-path inos, the kids index — no kernel needed.
    let wsi = MemWs::new();
    SturnFs::new(Path::new("/repo"), "tip", &wsi).unwrap();
    SturnFs::new_rw(Path::new("/repo"), "tip", &wsi, &MemWal::new(), "w", "t").unwrap();
}

/// RW-mount coverage lives in sturn-cli's e2e (`sturn mount --rw`
/// drives the same handlers through a child-process mount — an
/// in-process RW mount is flaky under the kernel and can wedge the
/// FUSE session on a failed unmount).
///
/// A Ws impl whose reads fail — drives the EIO branch in `read`.
struct FailWs;
impl Ws for FailWs {
    fn read(&self, _r: &Path, _rev: &str, _p: &str) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::other("read failed"))
    }
    fn list(&self, _r: &Path, _rev: &str) -> std::io::Result<Vec<String>> {
        Ok(vec!["x.txt".into()])
    }
    fn materialize(&self, _r: &Path, _rev: &str, _d: &Path, _p: &[String]) -> std::io::Result<()> {
        Ok(())
    }
    fn create(&self, _r: &Path, _n: &str, _b: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn entry(&self, _r: &Path, _rev: &str, _p: &str) -> std::io::Result<Option<String>> {
        Ok(None)
    }
    fn blob(&self, _r: &Path, _s: &str) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::other("no blob"))
    }
    fn store(&self, _r: &Path, _c: &[u8]) -> std::io::Result<String> {
        Err(std::io::Error::other("store failed"))
    }
    fn tip(&self, _r: &Path, _n: &str) -> std::io::Result<String> {
        Ok("t".into())
    }
    fn workspaces(&self, _r: &Path) -> std::io::Result<Vec<String>> {
        Ok(vec![])
    }
}

#[test]
fn read_error_surfaces_eio() {
    let mp = scratch("eio-mp");
    std::thread::scope(|s| {
        let fs = SturnFs::new(Path::new("/repo"), "rev", &FailWs).unwrap();
        let mp2 = mp.clone();
        let h = s.spawn(move || fs.mount(&mp2));
        wait_mounted(&mp);
        let mut buf = Vec::new();
        let err = std::fs::File::open(mp.join("x.txt"))
            .and_then(|mut f| f.read_to_end(&mut buf).map(|_| ()))
            .unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::EIO));
        unmount(&mp);
        h.join().unwrap().unwrap();
    });
}
