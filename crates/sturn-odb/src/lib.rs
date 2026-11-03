//! Direct git ODB access — loose-object write/read, ref resolution,
//! tree lookup — so hot paths (`wal.append`, `ws.store`, `ws.read`)
//! skip `git` process spawns entirely.
//!
//! Scope is deliberately narrow: loose objects and refs only. Anything
//! the fast path can't see — packed objects, alternates, unusual revs,
//! corrupt files — reports `None`/errs and the caller falls back to
//! plumbing, so on-disk semantics stay identical to spawning `git`.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// `.git` dir for `repo` — a plain repo, a linked worktree (`.git`
/// file), or a bare repo (has HEAD + objects/). Does *not* walk up
/// to parents or honor `GIT_DIR`; callers fall back to plumbing.
pub fn git_dir(repo: &Path) -> io::Result<PathBuf> {
    let dot = repo.join(".git");
    if dot.is_dir() {
        return Ok(dot);
    }
    if dot.is_file() {
        let s = fs::read_to_string(&dot)?;
        let target = s
            .trim()
            .strip_prefix("gitdir:")
            .map(str::trim)
            .ok_or_else(|| io::Error::other("bad .git file"))?;
        let p = PathBuf::from(target);
        return Ok(if p.is_absolute() { p } else { repo.join(p) });
    }
    if repo.join("HEAD").exists() && repo.join("objects").is_dir() {
        return Ok(repo.to_path_buf());
    }
    Err(io::Error::other(format!(
        "{}: not a git repo",
        repo.display()
    )))
}

/// Dir holding shared objects/refs — follows `commondir` for linked
/// worktrees, else the gitdir itself.
pub fn common_dir(gitdir: &Path) -> PathBuf {
    match fs::read_to_string(gitdir.join("commondir")) {
        Ok(s) => {
            let p = PathBuf::from(s.trim());
            if p.is_absolute() {
                p
            } else {
                gitdir.join(p)
            }
        }
        Err(_) => gitdir.to_path_buf(),
    }
}

fn objects_dir(repo: &Path) -> io::Result<PathBuf> {
    Ok(common_dir(&git_dir(repo)?).join("objects"))
}

/// Fast paths assume SHA-1 ids. On a sha256 repo
/// (`extensions.objectformat`) every entry point must defer to
/// plumbing — a sha1-written object is unresolvable there.
fn sha1_repo(repo: &Path) -> io::Result<bool> {
    let cfg = match fs::read_to_string(common_dir(&git_dir(repo)?).join("config")) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e),
    };
    let mut ext = false;
    for line in cfg.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            ext = l.trim_matches(|c| c == '[' || c == ']').eq_ignore_ascii_case("extensions");
            continue;
        }
        if ext {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case("objectformat") {
                    return Ok(v.trim().eq_ignore_ascii_case("sha1"));
                }
            }
        }
    }
    Ok(true)
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Content address: sha1("<kind> <len>\0<payload>") — the same id
/// `git hash-object` computes.
pub fn object_id(kind: &str, payload: &[u8]) -> String {
    use sha1::Digest;
    let mut h = sha1::Sha1::new();
    h.update(kind.as_bytes());
    h.update(b" ");
    h.update(payload.len().to_string().as_bytes());
    h.update(b"\0");
    h.update(payload);
    hex(&h.finalize())
}

fn deflate(raw: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(raw).expect("zlib into Vec cannot fail");
    e.finish().expect("zlib into Vec cannot fail")
}

fn inflate(z: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(z).read_to_end(&mut out)?;
    Ok(out)
}

/// Write `payload` as a loose object; returns its sha. A `Ok` sha is
/// identical to `git hash-object -w`. Errors mean the ODB couldn't be
/// found or written — callers may retry via plumbing.
pub fn write_object(repo: &Path, kind: &str, payload: &[u8]) -> io::Result<String> {
    if !sha1_repo(repo)? {
        return Err(io::Error::other("sha256 odb — use plumbing"));
    }
    let objects = objects_dir(repo)?;
    let sha = object_id(kind, payload);
    let dir = objects.join(&sha[..2]);
    let dst = dir.join(&sha[2..]);
    if dst.exists() {
        return Ok(sha); // content-addressed: already stored
    }
    fs::create_dir_all(&dir)?;
    let mut raw = Vec::with_capacity(payload.len() + 32);
    raw.extend_from_slice(kind.as_bytes());
    raw.push(b' ');
    raw.extend_from_slice(payload.len().to_string().as_bytes());
    raw.push(0);
    raw.extend_from_slice(payload);
    let z = deflate(&raw);
    // tmp + rename: readers never see a partial object, and racing
    // writers of the same content write identical bytes anyway. No
    // fsync — matches git, which doesn't fsync loose objects unless
    // core.fsyncObjectFiles is set.
    static N: AtomicU64 = AtomicU64::new(0);
    let tmp = dir.join(format!(
        "tmp_obj_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    match fs::write(&tmp, &z).and_then(|()| fs::rename(&tmp, &dst)) {
        Ok(()) => Ok(sha),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            // Lost a same-sha race — the object landed regardless.
            if dst.exists() {
                Ok(sha)
            } else {
                Err(e)
            }
        }
    }
}

/// Loose-object existence check only — packs and alternates are the
/// caller's fallback (`git cat-file -e`).
pub fn object_exists(repo: &Path, sha: &str) -> bool {
    if !sha1_repo(repo).unwrap_or(false) {
        return false;
    }
    let Ok(objects) = objects_dir(repo) else {
        return false;
    };
    sha.len() == 40
        && sha.bytes().all(|b| b.is_ascii_hexdigit())
        && objects.join(&sha[..2]).join(&sha[2..]).is_file()
}

/// Read a loose object → (kind, payload). `None` when the object
/// isn't a loose file (packed, alternate, or absent) — fall back to
/// `git cat-file`.
pub fn read_object(repo: &Path, sha: &str) -> io::Result<Option<(String, Vec<u8>)>> {
    if !sha1_repo(repo)? || sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let objects = objects_dir(repo)?;
    let z = match fs::read(objects.join(&sha[..2]).join(&sha[2..])) {
        Ok(z) => z,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let raw = inflate(&z)?;
    let nul = raw
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| io::Error::other("corrupt loose object"))?;
    let hdr = std::str::from_utf8(&raw[..nul])
        .map_err(|_| io::Error::other("corrupt loose object header"))?;
    let (kind, _len) = hdr
        .split_once(' ')
        .ok_or_else(|| io::Error::other("corrupt loose object header"))?;
    Ok(Some((kind.to_string(), raw[nul + 1..].to_vec())))
}

fn loose_ref(dir: &Path, refname: &str, depth: u8) -> io::Result<Option<String>> {
    if depth > 4 {
        return Ok(None);
    }
    match fs::read_to_string(dir.join(refname)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
        Ok(s) => {
            let s = s.trim();
            if let Some(target) = s.strip_prefix("ref:") {
                return loose_ref(dir, target.trim(), depth + 1);
            }
            Ok(Some(s.to_string()))
        }
    }
}

fn packed_ref(common: &Path, refname: &str) -> io::Result<Option<String>> {
    let s = match fs::read_to_string(common.join("packed-refs")) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    for line in s.lines() {
        if line.starts_with('#') || line.starts_with('^') || line.is_empty() {
            continue;
        }
        if let Some((sha, r)) = line.split_once(' ') {
            if r == refname {
                return Ok(Some(sha.to_string()));
            }
        }
    }
    Ok(None)
}

/// Resolve a ref to its sha — loose ref beats packed, worktree gitdir
/// beats commondir. `None` = the ref does not exist.
pub fn read_ref(repo: &Path, refname: &str) -> io::Result<Option<String>> {
    let gitdir = git_dir(repo)?;
    let common = common_dir(&gitdir);
    if let Some(v) = loose_ref(&gitdir, refname, 0)? {
        return Ok(Some(v));
    }
    if common != gitdir {
        if let Some(v) = loose_ref(&common, refname, 0)? {
            return Ok(Some(v));
        }
    }
    packed_ref(&common, refname)
}

/// `rev-parse`-lite: a full sha, or a ref under the standard prefixes
/// (as given, `refs/*`, heads, tags). `None` → caller falls back to
/// `git rev-parse` for the long tail of rev syntax.
pub fn resolve(repo: &Path, rev: &str) -> io::Result<Option<String>> {
    if !sha1_repo(repo)? {
        return Ok(None);
    }
    if rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(Some(rev.to_string()));
    }
    for cand in [
        rev.to_string(),
        format!("refs/{rev}"),
        format!("refs/heads/{rev}"),
        format!("refs/tags/{rev}"),
    ] {
        if let Some(sha) = read_ref(repo, &cand)? {
            return Ok(Some(sha));
        }
    }
    Ok(None)
}

/// Build a tree object's payload — entries are (mode, name, sha).
/// Sorted per git rules (dirs sort as `name/`); `None` on a bad sha.
pub fn tree_object(entries: &[(u32, &str, &str)]) -> Option<Vec<u8>> {
    let mut bufs: Vec<(String, Vec<u8>)> = Vec::with_capacity(entries.len());
    for (mode, name, sha) in entries {
        let raw = unhex(sha)?;
        if raw.len() != 20 {
            return None;
        }
        let mut line = Vec::with_capacity(name.len() + 28);
        line.extend_from_slice(format!("{mode:o}").as_bytes());
        line.push(b' ');
        line.extend_from_slice(name.as_bytes());
        line.push(0);
        line.extend_from_slice(&raw);
        let key = if *mode == 0o40000 {
            format!("{name}/")
        } else {
            (*name).to_string()
        };
        bufs.push((key, line));
    }
    bufs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::new();
    for (_, line) in bufs {
        out.extend_from_slice(&line);
    }
    Some(out)
}

/// One resolved ident line: `"Name <email>"` plus the local tz.
type Ident = (String, String);

/// Author/committer as ("Name <email>", tz) pairs — from `git var -l`,
/// which resolves env (`GIT_AUTHOR_*`) and config through the same
/// machinery `commit-tree` uses. Cached per gitdir: config/ident
/// changing mid-process is out of scope.
fn idents(repo: &Path) -> io::Result<Option<(Ident, Ident)>> {
    use std::process::Command;
    use std::sync::Mutex;
    // Explicit date envs accept many formats — defer to plumbing.
    if std::env::var_os("GIT_AUTHOR_DATE").is_some()
        || std::env::var_os("GIT_COMMITTER_DATE").is_some()
    {
        return Ok(None);
    }
    static CACHE: Mutex<Option<std::collections::HashMap<PathBuf, (Ident, Ident)>>> =
        Mutex::new(None);
    let gitdir = git_dir(repo)?;
    if let Some(hit) = CACHE
        .lock()
        .unwrap()
        .get_or_insert_with(Default::default)
        .get(&gitdir)
    {
        return Ok(Some(hit.clone()));
    }
    let o = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["var", "-l"])
        .output()?;
    if !o.status.success() {
        return Ok(None); // ident unset — let commit-tree say so
    }
    let out = String::from_utf8_lossy(&o.stdout);
    let parse = |key: &str| -> Option<(String, String)> {
        let line = out.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('='))?;
        // "Name <email> <ts> <tz>" → ("Name <email>", "<tz>")
        let (head, tz) = line.rsplit_once(' ')?;
        let (ident, _ts) = head.rsplit_once(' ')?;
        Some((ident.to_string(), tz.to_string()))
    };
    let pair = match (parse("GIT_AUTHOR_IDENT"), parse("GIT_COMMITTER_IDENT")) {
        (Some(a), Some(c)) => (a, c),
        _ => return Ok(None),
    };
    CACHE
        .lock()
        .unwrap()
        .get_or_insert_with(Default::default)
        .insert(gitdir, pair.clone());
    Ok(Some(pair))
}

/// Build a commit object (`git commit-tree` equivalent) and store
/// it — the only difference is no spawn. `None` → fall back to
/// `git commit-tree` (ident unresolvable or date envs set).
pub fn commit(repo: &Path, tree: &str, parents: &[&str], message: &str) -> io::Result<String> {
    let Some(((a_ident, a_tz), (c_ident, c_tz))) = idents(repo)? else {
        return Err(io::Error::other("ident unavailable — use plumbing"));
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut payload = format!("tree {tree}\n").into_bytes();
    for p in parents {
        payload.extend_from_slice(format!("parent {p}\n").as_bytes());
    }
    payload.extend_from_slice(format!("author {a_ident} {ts} {a_tz}\n").as_bytes());
    payload.extend_from_slice(format!("committer {c_ident} {ts} {c_tz}\n").as_bytes());
    payload.push(b'\n');
    // `commit-tree` stripspaces the message — sturn messages are
    // single-line tokens, so appending raw text is equivalent.
    payload.extend_from_slice(message.as_bytes());
    payload.push(b'\n');
    write_object(repo, "commit", &payload)
}

fn commit_tree(data: &[u8]) -> Option<String> {
    let line = data.split(|&b| b == b'\n').next()?;
    let hex = std::str::from_utf8(line.strip_prefix(b"tree ")?).ok()?;
    Some(hex.trim().to_string())
}

/// One entry of a decoded tree object — (mode, name, sha-hex).
fn tree_entry<'a>(tree: &'a [u8], pos: &mut usize) -> Option<(u32, &'a [u8], String)> {
    let rest = &tree[*pos..];
    let sp = rest.iter().position(|&b| b == b' ')?;
    let mode = std::str::from_utf8(&rest[..sp]).ok()?;
    let mode = u32::from_str_radix(mode, 8).ok()?;
    let rest = &rest[sp + 1..];
    let nul = rest.iter().position(|&b| b == 0)?;
    let name = &rest[..nul];
    let raw = rest.get(nul + 1..nul + 21)?;
    *pos = tree.len() - rest.len() + nul + 21;
    Some((mode, name, hex(raw)))
}

/// `<rev>:<path>` walk → entry sha. Loose objects only; `None` when
/// anything isn't loose, the path is absent, or rev syntax is fancy.
fn entry_at(repo: &Path, rev: &str, path: &str) -> io::Result<Option<String>> {
    let Some(mut sha) = resolve(repo, rev)? else {
        return Ok(None);
    };
    // Peel a commit (or annotated tag) to its tree.
    if let Some((kind, data)) = read_object(repo, &sha)? {
        match kind.as_str() {
            "commit" => match commit_tree(&data) {
                Some(t) => sha = t,
                None => return Ok(None),
            },
            "tree" => {}
            _ => return Ok(None),
        }
    } else {
        return Ok(None);
    }
    let mut it = path.split('/').filter(|c| !c.is_empty()).peekable();
    while let Some(c) = it.next() {
        let Some((kind, data)) = read_object(repo, &sha)? else {
            return Ok(None);
        };
        if kind != "tree" {
            return Ok(None);
        }
        let mut pos = 0;
        let mut hit = None;
        while pos < data.len() {
            let Some((_mode, name, e)) = tree_entry(&data, &mut pos) else {
                break;
            };
            if name == c.as_bytes() {
                hit = Some(e);
                break;
            }
        }
        match hit {
            Some(e) => sha = e,
            None => return Ok(None),
        }
        if it.peek().is_none() {
            return Ok(Some(sha));
        }
    }
    Ok(None)
}

/// `git cat-file -p <rev>:<path>` over loose objects — returns blob
/// bytes. `None` → fall back to plumbing (packs, weird revs, trees).
pub fn read_at(repo: &Path, rev: &str, path: &str) -> io::Result<Option<Vec<u8>>> {
    let Some(sha) = entry_at(repo, rev, path)? else {
        return Ok(None);
    };
    match read_object(repo, &sha)? {
        Some((kind, payload)) if kind == "blob" => Ok(Some(payload)),
        _ => Ok(None),
    }
}

/// `<rev>:<path>` → the entry's object sha (blob or tree). `None` →
/// fall back to `git rev-parse <rev>:<path>`.
pub fn entry_oid(repo: &Path, rev: &str, path: &str) -> io::Result<Option<String>> {
    entry_at(repo, rev, path)
}

/// `git ls-tree -r --name-only <rev>` over loose objects. `None` →
/// fall back to plumbing.
pub fn list_tree(repo: &Path, rev: &str) -> io::Result<Option<Vec<String>>> {
    let Some(mut sha) = resolve(repo, rev)? else {
        return Ok(None);
    };
    if let Some((kind, data)) = read_object(repo, &sha)? {
        match kind.as_str() {
            "commit" => match commit_tree(&data) {
                Some(t) => sha = t,
                None => return Ok(None),
            },
            "tree" => {}
            _ => return Ok(None),
        }
    } else {
        return Ok(None);
    }
    let mut out = Vec::new();
    if walk(repo, &sha, "", &mut out)? {
        Ok(Some(out))
    } else {
        Ok(None)
    }
}

fn walk(repo: &Path, sha: &str, prefix: &str, out: &mut Vec<String>) -> io::Result<bool> {
    let Some((kind, data)) = read_object(repo, sha)? else {
        return Ok(false);
    };
    if kind != "tree" {
        return Ok(false);
    }
    let mut pos = 0;
    while pos < data.len() {
        let Some((mode, name, e)) = tree_entry(&data, &mut pos) else {
            break;
        };
        let name = String::from_utf8_lossy(name).into_owned();
        if mode == 0o40000 {
            if !walk(repo, &e, &format!("{prefix}{name}/"), out)? {
                return Ok(false);
            }
        } else {
            out.push(format!("{prefix}{name}"));
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_id_matches_git_sha1() {
        // `git hash-object --stdin` for "hello\n" → ce013625030ba8dba906f756967f9e9ca394464a
        assert_eq!(
            object_id("blob", b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
        // Empty blob id is a well-known constant.
        assert_eq!(
            object_id("blob", b""),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
    }

    #[test]
    fn sha256_repo_defers_to_plumbing() {
        let dir = std::env::temp_dir().join(format!("sturn-odb-sha256-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q", "--object-format=sha256"])
            .arg(&dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return; // git < 2.29 — no sha256 support to test against
        }
        assert!(write_object(&dir, "blob", b"x").is_err());
        assert_eq!(resolve(&dir, "HEAD").unwrap(), None);
        assert_eq!(read_object(&dir, &"0".repeat(64)).unwrap(), None);
        assert!(!object_exists(&dir, &"0".repeat(64)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tree_object_sorts_like_git() {
        // 'a.txt' then 'b.txt' — git sorts entries by name.
        let a = object_id("blob", b"a");
        let b = object_id("blob", b"b");
        let t = tree_object(&[(0o100644, "b.txt", &b), (0o100644, "a.txt", &a)]).unwrap();
        let s = String::from_utf8_lossy(&t);
        assert!(s.find("a.txt").unwrap() < s.find("b.txt").unwrap());
    }
}
