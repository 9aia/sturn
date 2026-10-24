//! FUSE impl — mount a ws tree for POSIX tools (editors, GUIs).
//! Read-only when `wal` is None; with a WAL, writes are buffered
//! per open file and become Write/Delete/Rename ops on `release`.
//! The mounted view is the ws tip at mount time.

use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};
use sturn_core::{Event, Op};
use sturn_wal::Wal;
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
        kind: if dir {
            FileType::Directory
        } else {
            FileType::RegularFile
        },
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
    path: String,
    size: u64,
}

/// Tree mount over the ODB. With `wal`/`ws`/`actor`, it's read-write:
/// edits flow back as ops — the FUSE mount is just another actor.
pub struct SturnFs<'a> {
    repo: &'a Path,
    rev: String,
    wsi: &'a dyn Ws,
    wal: Option<&'a dyn Wal>,
    ws: &'a str,
    actor: &'a str,
    entries: HashMap<u64, Entry>,
    kids: HashMap<u64, Vec<(String, u64)>>,
    by_path: HashMap<String, u64>,
    /// ino → (path, buffered content) for open writable files.
    open: HashMap<u64, (String, Vec<u8>)>,
    /// ino → read buffer for open read-only files (lazy hydration).
    rdbuf: HashMap<u64, Vec<u8>>,
    next_ino: u64,
}

impl<'a> SturnFs<'a> {
    /// Read-only mount at `rev`.
    pub fn new(repo: &'a Path, rev: &str, wsi: &'a dyn Ws) -> io::Result<Self> {
        Self::build(repo, rev, wsi, None, "", "")
    }

    /// Read-write mount — releases become ops on `ws`.
    pub fn new_rw(
        repo: &'a Path,
        rev: &str,
        wsi: &'a dyn Ws,
        wal: &'a dyn Wal,
        ws: &'a str,
        actor: &'a str,
    ) -> io::Result<Self> {
        Self::build(repo, rev, wsi, Some(wal), ws, actor)
    }

    fn build(
        repo: &'a Path,
        rev: &str,
        wsi: &'a dyn Ws,
        wal: Option<&'a dyn Wal>,
        ws: &'a str,
        actor: &'a str,
    ) -> io::Result<Self> {
        let mut fs = SturnFs {
            repo,
            rev: rev.into(),
            wsi,
            wal,
            ws,
            actor,
            entries: HashMap::new(),
            kids: HashMap::new(),
            by_path: HashMap::new(),
            open: HashMap::new(),
            rdbuf: HashMap::new(),
            next_ino: 2,
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
                if !fs.entries.contains_key(&ino) { fs.entries.insert(ino, Entry { ino, dir: !last, path: cur.clone(), size: 0 }); }
                let parent_ino = fs.by_path[&parent];
                let kids = fs.kids.entry(parent_ino).or_default();
                if !kids.iter().any(|(_, k)| *k == ino) { kids.push((part.to_string(), ino)); }
            }
        }
        fs.next_ino = next;
        Ok(fs)
    }

    /// Mount (blocking) at `mountpoint`; RW iff wal is Some.
    pub fn mount(self, mountpoint: &Path) -> io::Result<()> {
        let opts = if self.wal.is_some() {
            vec![MountOption::RW, MountOption::FSName("sturn".into())]
        } else {
            vec![MountOption::RO, MountOption::FSName("sturn".into())]
        };
        fuser::mount2(self, mountpoint, &opts)
    }

    fn append(&self, op: Op) {
        if let Some(wal) = self.wal { let _ = wal.append(self.repo, self.ws, &Event::new(self.ws, self.actor, op)); }
    }

    fn add_entry(&mut self, path: &str, dir: bool) -> u64 {
        let ino = self.next_ino;
        self.next_ino += 1;
        self.entries.insert(
            ino,
            Entry {
                ino,
                dir,
                path: path.into(),
                size: 0,
            },
        );
        self.by_path.insert(path.into(), ino);
        if let Some((parent, name)) = path.rsplit_once('/') {
            let pin = self.by_path[parent];
            self.kids
                .entry(pin)
                .or_default()
                .push((name.into(), ino));
        } else {
            self.kids
                .entry(ROOT)
                .or_default()
                .push((path.into(), ino));
        }
        ino
    }

    /// Reported size for an ino. Entries are built path-only, so a
    /// file's real size resolves lazily from the ODB on first ask —
    /// the kernel clamps reads to i_size, so reporting 0 would serve
    /// an instant EOF for every read.
    fn file_size(&mut self, ino: u64) -> u64 {
        if let Some(b) = self.rdbuf.get(&ino) { return b.len() as u64; }
        if let Some((_, b)) = self.open.get(&ino) { return b.len() as u64; }
        // Sizes are queried only for issued inos — the miss is defensive.
        let Some((path, size, dir)) = self.entries.get(&ino).map(|e| (e.path.clone(), e.size, e.dir)) else { return 0; };
        if dir || size != 0 { return size; }
        if let Ok(b) = self.wsi.read(self.repo, &self.rev, &path) {
            let n = b.len() as u64;
            if let Some(e) = self.entries.get_mut(&ino) { e.size = n; }
            return n;
        }
        // Unreadable blob: report a byte so the kernel issues read()
        // and surfaces the real EIO — size 0 would fake a clean EOF.
        1
    }
}

impl Filesystem for SturnFs<'_> {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let found = self
            .kids
            .get(&parent)
            .and_then(|ks| ks.iter().find(|(n, _)| n.as_str() == name.to_string_lossy()))
            .map(|(_, ino)| *ino);
        match found.and_then(|i| self.entries.get(&i).map(|e| (e.ino, e.dir))) {
            Some((i, dir)) => reply.entry(&TTL, &attr(i, self.file_size(i), dir), 0),
            None => reply.error(libc::ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        // The kernel only asks about inos we issued — None is defensive.
        let Some(dir) = self.entries.get(&ino).map(|e| e.dir) else { reply.error(libc::ENOENT); return; };
        reply.attr(&TTL, &attr(ino, self.file_size(ino), dir));
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        // Kernel sends readdir only for issued dir inos — defensive.
        let Some(e) = self.entries.get(&ino) else { reply.error(libc::ENOENT); return; };
        if !e.dir { reply.error(libc::ENOTDIR); return; }
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

    fn open(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        // Kernel opens only issued file inos (dirs go through opendir).
        let Some(e) = self.entries.get(&ino) else { reply.error(libc::ENOENT); return; };
        if e.dir { reply.error(libc::EISDIR); return; }
        if self.wal.is_some() {
            // hydrate lazily — read current content into the write buf
            let bytes = self
                .wsi
                .read(self.repo, &self.rev, &e.path)
                .unwrap_or_default();
            self.rdbuf.insert(ino, bytes);
        }
        reply.opened(ino, 0);
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
        if let Some(buf) = self.rdbuf.get(&ino) {
            let start = (offset as usize).min(buf.len());
            let end = (start + size as usize).min(buf.len());
            reply.data(&buf[start..end]);
            return;
        }
        // Kernel reads only issued inos — defensive.
        let Some(e) = self.entries.get(&ino) else { reply.error(libc::ENOENT); return; };
        match self.wsi.read(self.repo, &self.rev, &e.path) {
            Ok(bytes) => {
                let start = (offset as usize).min(bytes.len());
                let end = (start + size as usize).min(bytes.len());
                reply.data(&bytes[start..end]);
            }
            Err(_) => reply.error(libc::EIO),
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _wf: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyWrite,
    ) {
        // RO mounts are enforced by the VFS (MS_RDONLY) — defensive.
        if self.wal.is_none() { reply.error(libc::EROFS); return; }
        if !self.open.contains_key(&ino) {
            // first write on this handle: seed buffer from rdbuf
            let Some(path) = self.entries.get(&ino).map(|e| e.path.clone()) else { reply.error(libc::ENOENT); return; };
            let buf = self.rdbuf.remove(&ino).unwrap_or_default();
            self.open.insert(ino, (path, buf));
        }
        let Some((_, buf)) = self.open.get_mut(&ino) else { reply.error(libc::ENOENT); return; };
        let off = offset as usize;
        if buf.len() < off + data.len() { buf.resize(off + data.len(), 0); }
        buf[off..off + data.len()].copy_from_slice(data);
        reply.written(data.len() as u32);
    }

    fn create(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        // VFS refuses create on MS_RDONLY mounts — defensive.
        if self.wal.is_none() { reply.error(libc::EROFS); return; }
        let pname = name.to_string_lossy().into_owned();
        let path = {
            let pe = &self.entries[&parent];
            if pe.path.is_empty() {
                pname.clone()
            } else {
                format!("{}/{pname}", pe.path)
            }
        };
        let ino = self.add_entry(&path, false);
        self.open.insert(ino, (path, Vec::new()));
        reply.created(&TTL, &attr(ino, 0, false), 0, ino, 0);
    }

    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _a: Option<fuser::TimeOrNow>,
        _m: Option<fuser::TimeOrNow>,
        _c: Option<std::time::SystemTime>,
        _fh: Option<u64>,
        _cr: Option<std::time::SystemTime>,
        _chg: Option<std::time::SystemTime>,
        _bk: Option<std::time::SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        // Kernel sets attrs only on issued inos — defensive.
        let Some(e) = self.entries.get_mut(&ino) else { reply.error(libc::ENOENT); return; };
        if let Some(sz) = size {
            // truncate — O_TRUNC on open lands here before write()
            if let Some((_, buf)) = self.open.get_mut(&ino) {
                buf.resize(sz as usize, 0);
            } else {
                let buf = self
                    .wsi
                    .read(self.repo, &self.rev, &e.path)
                    .unwrap_or_default();
                let mut b = buf;
                b.resize(sz as usize, 0);
                self.open.insert(ino, (e.path.clone(), b));
            }
            e.size = sz;
        }
        reply.attr(&TTL, &attr(ino, e.size, e.dir));
    }

    fn release(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        _flags: i32,
        _lock: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        // The write-back: buffered content becomes a Write op.
        if let Some((path, buf)) = self.open.remove(&ino) {
            if let Ok(blob) = self.wsi.store(self.repo, &buf) { self.append(Op::Write { path, blob, intent: Some("fuse".into()) }); }
        }
        self.rdbuf.remove(&ino);
        reply.ok();
    }

    fn fsync(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        // durability point — emit the op NOW, not on release.
        // editors that fsync-per-save get their op immediately.
        if let Some((path, buf)) = self.open.get(&ino) { if let Ok(blob) = self.wsi.store(self.repo, buf) { self.append(Op::Write { path: path.clone(), blob, intent: Some("fuse fsync".into()) }); } }
        reply.ok();
    }

    fn flush(&mut self, _req: &Request, _ino: u64, _fh: u64, _lock: u64, reply: ReplyEmpty) {
        // close(dup(fd)) flushes; release does the real write-back.
        reply.ok();
    }

    fn mkdir(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        // VFS refuses mkdir on MS_RDONLY mounts — defensive.
        if self.wal.is_none() { reply.error(libc::EROFS); return; }
        // dirs are implicit in git trees — local entry only,
        // no op until a file lands inside.
        let pname = name.to_string_lossy().into_owned();
        let pe = &self.entries[&parent];
        let path = if pe.path.is_empty() {
            pname
        } else {
            format!("{}/{pname}", pe.path)
        };
        let ino = self.add_entry(&path, true);
        reply.entry(&TTL, &attr(ino, 0, true), 0);
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        // VFS refuses rmdir on MS_RDONLY mounts — defensive.
        if self.wal.is_none() { reply.error(libc::EROFS); return; }
        let pname = name.to_string_lossy().into_owned();
        let pe = &self.entries[&parent];
        let path = if pe.path.is_empty() {
            pname
        } else {
            format!("{}/{pname}", pe.path)
        };
        // refuse non-empty dirs — files inside are real ops
        let has_kids = self
            .kids
            .get(&self.by_path[&path])
            .map(|k| !k.is_empty())
            .unwrap_or(false);
        if has_kids {
            reply.error(libc::ENOTEMPTY);
            return;
        }
        if let Some(ino) = self.by_path.remove(&path) { self.entries.remove(&ino); if let Some(k) = self.kids.get_mut(&parent) { k.retain(|(_, i)| *i != ino); } }
        reply.ok();
    }

    fn unlink(&mut self, _req: &Request, _parent: u64, name: &OsStr, reply: ReplyEmpty) {
        // VFS refuses unlink on MS_RDONLY mounts — defensive.
        if self.wal.is_none() { reply.error(libc::EROFS); return; }
        // resolve via children of parent
        let pname = name.to_string_lossy().into_owned();
        let path = {
            let p_ino = _parent;
            self.kids
                .get(&p_ino)
                .and_then(|ks| ks.iter().find(|(n, _)| *n == pname))
                .and_then(|(_, i)| self.entries.get(i))
                .map(|e| e.path.clone())
        };
        // The kernel resolves the name before unlink — else is defensive.
        let Some(p) = path else { reply.error(libc::ENOENT); return; };
        self.append(Op::Delete { path: p.clone() });
        // Drop the entry so later lookups/readdir agree.
        if let Some(ino) = self.by_path.remove(&p) { self.entries.remove(&ino); if let Some(ks) = self.kids.get_mut(&_parent) { ks.retain(|(_, i)| *i != ino); } }
        reply.ok();
    }

    fn rename(
        &mut self,
        _req: &Request,
        _parent: u64,
        name: &OsStr,
        _newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        // VFS refuses rename on MS_RDONLY mounts — defensive.
        if self.wal.is_none() { reply.error(libc::EROFS); return; }
        let resolve = |parent: u64, n: &OsStr| -> Option<String> {
            self.kids
                .get(&parent)
                .and_then(|ks| {
                    ks.iter()
                        .find(|(k, _)| k.as_str() == n.to_string_lossy())
                })
                .and_then(|(_, i)| self.entries.get(i))
                .map(|e| e.path.clone())
        };
        let found = (
            resolve(_parent, name),
            resolve(_newparent, newname).or_else(|| {
                let np = &self.entries.get(&_newparent)?.path;
                Some(if np.is_empty() { newname.to_string_lossy().into_owned() } else { format!("{np}/{}", newname.to_string_lossy()) })
            }),
        );
        // The kernel resolves the source before rename — else is defensive.
        let (Some(from), Some(to)) = found else { reply.error(libc::ENOENT); return; };
        self.append(Op::Rename {
            from: from.clone(),
            to: to.clone(),
        });
        // Keep the tree view in sync — later ops resolve by
        // path, and a stale map returns ENOENT for the moved name.
        // A stale source mapping is defensive in the same way.
        let Some(ino) = self.by_path.remove(&from) else { reply.ok(); return; };
        if let Some(ks) = self.kids.get_mut(&_parent) { ks.retain(|(_, i)| *i != ino); }
        self.kids
            .entry(_newparent)
            .or_default()
            .push((newname.to_string_lossy().into_owned(), ino));
        if let Some(e) = self.entries.get_mut(&ino) { e.path = to.clone(); }
        if let Some(o) = self.open.get_mut(&ino) { o.0 = to.clone(); }
        self.by_path.insert(to.clone(), ino);
        // A dir move drags its descendants' paths along.
        let prefix = format!("{from}/");
        let rewrites: Vec<(u64, String, String)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.path.starts_with(&prefix))
            .map(|(i, e)| (*i, e.path.clone(), format!("{to}/{}", &e.path[prefix.len()..])))
            .collect();
        for (i, old, new) in rewrites {
            self.by_path.remove(&old);
            self.by_path.insert(new.clone(), i);
            if let Some(e) = self.entries.get_mut(&i) { e.path = new.clone(); }
            if let Some(o) = self.open.get_mut(&i) { o.0 = new; }
        }
        reply.ok();
    }
}
