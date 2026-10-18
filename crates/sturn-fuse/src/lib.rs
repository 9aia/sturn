//! FUSE impl — mount a ws tree read-only for POSIX tools that can't
//! take the ODB API (editors, file watchers, GUIs). Flat lazily-read
//! files over `Ws`; the tree is resolved at mount.

use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory,
    ReplyEntry, Request,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};
use sturn_ws::Ws;

const TTL: Duration = Duration::from_secs(1);
const ROOT: u64 = 1;

fn attr(ino: u64, size: u64, dir: bool) -> FileAttr {
    FileAttr {
        ino,
        size,
        blocks: size.div_ceil(512),
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind: if dir { FileType::Directory } else { FileType::RegularFile },
        perm: if dir { 0o755 } else { 0o644 },
        nlink: 1,
        uid: 0,
        gid: 0,
        rdev: 0,
        flags: 0,
        blksize: 512,
    }
}

struct Entry {
    ino: u64,
    dir: bool,
    /// blob rev path for files (read on demand), None for dirs.
    path: String,
    size: u64,
}

/// Read-only tree mount — files resolve `read` lazily via the ODB.
/// The mounted view is the ws tip at mount time (overlay later).
pub struct SturnFs<'a> {
    repo: &'a Path,
    rev: String,
    wsi: &'a dyn Ws,
    /// ino → entry; 1 = root.
    entries: HashMap<u64, Entry>,
    /// dir ino → children (name, ino).
    kids: HashMap<u64, Vec<(String, u64)>>,
    /// path → ino for lookups.
    by_path: HashMap<String, u64>,
}

impl<'a> SturnFs<'a> {
    /// Build the inode index for `rev`'s whole tree.
    pub fn new(repo: &'a Path, rev: &str, wsi: &'a dyn Ws) -> io::Result<Self> {
        let mut fs = SturnFs {
            repo,
            rev: rev.into(),
            wsi,
            entries: HashMap::new(),
            kids: HashMap::new(),
            by_path: HashMap::new(),
        };
        fs.entries.insert(
            ROOT,
            Entry {
                ino: ROOT,
                dir: true,
                path: String::new(),
                size: 0,
            },
        );
        fs.by_path.insert(String::new(), ROOT);
        let mut next = 2u64;
        for p in wsi.list(repo, rev)? {
            // ensure parent chain as dir inos
            let mut cur = String::new();
            let parts: Vec<&str> = p.split('/').collect();
            for (i, part) in parts.iter().enumerate() {
                let parent = cur.clone();
                cur = if cur.is_empty() {
                    part.to_string()
                } else {
                    format!("{cur}/{part}")
                };
                let last = i == parts.len() - 1;
                let ino = *fs.by_path.entry(cur.clone()).or_insert_with(|| {
                    let e = next;
                    next += 1;
                    e
                });
                if !fs.entries.contains_key(&ino) {
                    fs.entries.insert(
                        ino,
                        Entry {
                            ino,
                            dir: !last,
                            path: cur.clone(),
                            size: 0,
                        },
                    );
                }
                let parent_ino = fs.by_path[&parent];
                let kids = fs.kids.entry(parent_ino).or_default();
                if !kids.iter().any(|(_, k)| *k == ino) {
                    kids.push((part.to_string(), ino));
                }
            }
        }
        Ok(fs)
    }

    /// Mount (blocking) at `mountpoint`.
    pub fn mount(self, mountpoint: &Path) -> io::Result<()> {
        fuser::mount2(self, mountpoint, &[MountOption::RO, MountOption::FSName("sturn".into())])
    }
}

impl Filesystem for SturnFs<'_> {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let found = self
            .kids
            .get(&parent)
            .and_then(|ks| {
                ks.iter()
                    .find(|(n, _)| n.as_str() == name.to_string_lossy())
            })
            .map(|(_, ino)| *ino);
        match found.and_then(|i| self.entries.get(&i)) {
            Some(e) => reply.entry(&TTL, &attr(e.ino, e.size, e.dir), 0),
            None => reply.error(libc::ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        match self.entries.get(&ino) {
            Some(e) => reply.attr(&TTL, &attr(ino, e.size, e.dir)),
            None => reply.error(libc::ENOENT),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let Some(e) = self.entries.get(&ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        if !e.dir {
            reply.error(libc::ENOTDIR);
            return;
        }
        let mut items: Vec<(u64, FileType, String)> = vec![
            (ino, FileType::Directory, ".".into()),
            (ino, FileType::Directory, "..".into()),
        ];
        if let Some(ks) = self.kids.get(&ino) {
            for (n, k) in ks {
                let t = if self.entries[k].dir {
                    FileType::Directory
                } else {
                    FileType::RegularFile
                };
                items.push((*k, t, n.clone()));
            }
        }
        for (i, (k, t, n)) in items.into_iter().enumerate().skip(offset as usize) {
            if reply.add(k, (i + 1) as i64, t, n) {
                break;
            }
        }
        reply.ok();
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        let Some(e) = self.entries.get(&ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        match self.wsi.read(self.repo, &self.rev, &e.path) {
            Ok(bytes) => {
                let start = (offset as usize).min(bytes.len());
                let end = (start + size as usize).min(bytes.len());
                reply.data(&bytes[start..end]);
            }
            Err(_) => reply.error(libc::EIO),
        }
    }
}
