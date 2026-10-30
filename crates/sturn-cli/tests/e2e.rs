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

/// Repo-local identity — commit-tree needs one and the test env
/// may have no global git config.
fn ident(dir: &Path) {
    git(dir, &["config", "user.email", "t@t"]);
    git(dir, &["config", "user.name", "t"]);
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
    ident(&repo);
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
    ident(&repo);
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

/// One JSON-RPC exchange with `sturn mcp` — newline-delimited, per
/// the MCP stdio transport.
struct Mcp {
    stdin: Option<std::process::ChildStdin>,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    child: std::process::Child,
    n: u64,
}

impl Mcp {
    fn spawn(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sturn"))
            .current_dir(dir)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Mcp {
            stdin: Some(child.stdin.take().unwrap()),
            stdout: std::io::BufReader::new(child.stdout.take().unwrap()),
            child,
            n: 0,
        }
    }

    fn rpc(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        use std::io::BufRead;
        self.n += 1;
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": self.n, "method": method, "params": params,
        });
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{req}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        // Close stdin → serve() sees EOF and exits cleanly. A bare
        // SIGKILL drops the child's profraw (coverage) entirely.
        drop(self.stdin.take());
        if self.child.wait().is_err() {
            let _ = self.child.kill();
        }
    }
}

fn call(m: &mut Mcp, tool: &str, args: serde_json::Value) -> String {
    let r = m.rpc(
        "tools/call",
        serde_json::json!({"name": tool, "arguments": args}),
    );
    let content = &r["result"]["content"][0]["text"];
    assert!(
        r["result"]["isError"].as_bool() != Some(true),
        "{tool} failed: {content}"
    );
    content.as_str().unwrap().to_string()
}

#[test]
fn mcp_agent_surface() {
    let repo = scratch("mcp");
    git(&repo, &["init", "-q", "-b", "main"]);
    ident(&repo);
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
    sturn(&repo, &["ws", "new", "a"]);
    sturn(&repo, &["ws", "new", "b"]);

    let mut m = Mcp::spawn(&repo);
    let init = m.rpc(
        "initialize",
        serde_json::json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name":"t","version":"0"}}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "sturn");
    let tools = m.rpc("tools/list", serde_json::json!({}));
    assert_eq!(
        tools["result"]["tools"].as_array().unwrap().len(),
        16,
        "expected the tool surface"
    );

    // Overlay read: ws_write makes the file visible before any commit.
    call(
        &mut m,
        "ws_write",
        serde_json::json!({"ws":"a","path":"src/x.rs","content":"fn x(){}\n","intent":"t"}),
    );
    let got = call(
        &mut m,
        "ws_read",
        serde_json::json!({"ws":"a","path":"src/x.rs"}),
    );
    assert_eq!(got, "fn x(){}\n");
    // …and it shows up in glob listing too.
    let glob = call(
        &mut m,
        "ws_glob",
        serde_json::json!({"ws":"a","pattern":"src/**"}),
    );
    assert!(glob.contains("src/x.rs"), "{glob}");

    // rename lands in the overlay before any commit
    call(
        &mut m,
        "ws_rename",
        serde_json::json!({"ws":"a","from":"src/x.rs","to":"src/y.rs"}),
    );
    let got = call(
        &mut m,
        "ws_read",
        serde_json::json!({"ws":"a","path":"src/y.rs"}),
    );
    assert_eq!(got, "fn x(){}\n");

    // Claims across ws boundary: a holds src/**, b is denied.
    call(
        &mut m,
        "claim",
        serde_json::json!({"ws":"a","glob":"src/**","ttl_secs":60}),
    );
    let denied = m.rpc(
        "tools/call",
        serde_json::json!({"name":"claim","arguments":{"ws":"b","glob":"src/x.rs"}}),
    );
    assert_eq!(denied["result"]["isError"].as_bool(), Some(true));

    // Snap via MCP produces a real commit.
    call(
        &mut m,
        "snap",
        serde_json::json!({"ws":"a","message":"mcp work"}),
    );
    assert_eq!(git(&repo, &["show", "HEAD:src/y.rs"]), "fn x(){}");
}

#[test]
fn exec_runs_on_uncommitted_overlay() {
    // A zero-dep crate — cargo check works fully offline.
    let repo = scratch("exec");
    git(&repo, &["init", "-q", "-b", "main"]);
    ident(&repo);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        b"[package]\nname = \"t\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(repo.join("src/lib.rs"), b"pub fn ok() -> i32 { 1 }\n").unwrap();
    git(&repo, &["add", "."]);
    commit(&repo, "crate");
    sturn(&repo, &["init"]);
    sturn(&repo, &["ws", "new", "a"]);

    // Broken code exists ONLY as an op — hydration must include it.
    sturn_in(
        &repo,
        &["write", "src/lib.rs", "--ws", "a"],
        b"pub fn ok() -> i32 { \"nope\" }\n",
    );
    let o = Command::new(env!("CARGO_BIN_EXE_sturn"))
        .current_dir(&repo)
        .args(["exec", "--ws", "a", "--", "cargo", "check", "--offline"])
        .output()
        .unwrap();
    assert!(
        !o.status.success(),
        "cargo check should fail on the overlaid broken write"
    );

    // Fix via op only — check passes; disk never had the files.
    sturn_in(
        &repo,
        &["write", "src/lib.rs", "--ws", "a"],
        b"pub fn ok() -> i32 { 2 }\n",
    );
    sturn(&repo, &["exec", "--ws", "a", "--", "cargo", "check", "--offline"]);
}

#[test]
fn leave_sweeps_claims_and_compact_shrinks_wal() {
    let repo = scratch("sweep");
    git(&repo, &["init", "-q", "-b", "main"]);
    ident(&repo);
    git(&repo, &["-c","user.email=t@t","-c","user.name=t","commit","-qm","init","--allow-empty"]);
    sturn(&repo, &["init"]);
    sturn(&repo, &["ws", "new", "a"]);
    sturn(&repo, &["join", "a"]);
    sturn(&repo, &["claim", "x/**", "--ws", "a"]);
    assert!(sturn(&repo, &["claims"]).contains("x/**"));
    // leaving sweeps the claim — dead agents can't hold locks
    sturn(&repo, &["leave", "a"]);
    assert!(!sturn(&repo, &["claims"]).contains("x/**"));

    // compact: pre-snapshot events drop, marker survives
    sturn_in(&repo, &["write", "f.txt", "--ws", "a"], b"1\n");
    sturn(&repo, &["snap", "a", "-m", "one"]);
    let before = sturn(&repo, &["log", "--ws", "a"]).lines().count();
    let out = sturn(&repo, &["compact", "--ws", "a"]);
    assert!(out.contains("dropped"), "{out}");
    let after = sturn(&repo, &["log", "--ws", "a"]).lines().count();
    assert!(after < before, "before={before} after={after}");
    assert!(sturn(&repo, &["log", "--ws", "a"]).contains("snapshot"));
    // archive ref kept the history
    assert!(git(&repo, &["rev-parse", "refs/sturn/archive/a"]).len() == 40);
}

#[test]
fn review_gate_blocks_unreviewed_ops() {
    let repo = scratch("gate");
    git(&repo, &["init", "-q", "-b", "main"]);
    ident(&repo);
    git(&repo, &["-c","user.email=t@t","-c","user.name=t","commit","-qm","init","--allow-empty"]);
    sturn(&repo, &["init"]);
    // gate on
    std::fs::write(
        repo.join("sturn.toml"),
        b"[repo]\ndir = \".\"\n\n[review]\nrequire = true\n",
    )
    .unwrap();
    sturn(&repo, &["ws", "new", "a"]);
    sturn_in(&repo, &["write", "f.txt", "--ws", "a"], b"1\n");
    // the event id, straight from the WAL (not stdout parsing)
    let log = sturn(&repo, &["log", "--ws", "a"]);
    let eid = log
        .lines()
        .find(|l| l.contains("\"write\""))
        .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let err = sturn_fails(&repo, &["snap", "a", "-m", "nope"]);
    assert!(err.contains("review gate"), "{err}");

    // approve the write event — reviews live in a reviewer ws
    sturn(&repo, &["ws", "new", "rev"]);
    sturn(&repo, &["review", &eid, "approve", "--ws", "rev"]);
    sturn(&repo, &["snap", "a", "-m", "ok"]);
    assert_eq!(git(&repo, &["show", "HEAD:f.txt"]), "1");
}

#[test]
fn merge_snap_folds_two_workspaces() {
    let repo = scratch("merge");
    git(&repo, &["init", "-q", "-b", "main"]);
    ident(&repo);
    git(&repo, &["-c","user.email=t@t","-c","user.name=t","commit","-qm","init","--allow-empty"]);
    sturn(&repo, &["init"]);
    sturn(&repo, &["ws", "new", "a"]);
    sturn(&repo, &["ws", "new", "b"]);
    sturn_in(&repo, &["write", "x.txt", "--ws", "a"], b"ax\n");
    sturn_in(&repo, &["write", "y.txt", "--ws", "b"], b"by\n");
    sturn(&repo, &["snap", "a", "b", "-m", "merged"]);
    assert_eq!(git(&repo, &["show", "HEAD:x.txt"]), "ax");
    assert_eq!(git(&repo, &["show", "HEAD:y.txt"]), "by");
}

// ---------- coverage extensions: every subcommand + error path ----------

fn git_in(dir: &Path, args: &[&str], input: &[u8]) -> String {
    let mut c = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input).unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(
        o.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Spawn sturn with extra env vars; HOME is redirected to a scratch
/// dir so no real user config/env leaks in.
fn sturn_full(dir: &Path, args: &[&str], envs: &[(&str, &str)], stdin: &[u8]) -> std::process::Output {
    let home = std::env::temp_dir().join(format!("sturn-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sturn"));
    cmd.current_dir(dir)
        .args(args)
        .env("HOME", &home)
        .env_remove("STURN_CONFIG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut c = cmd.spawn().unwrap();
    if !stdin.is_empty() {
        c.stdin.take().unwrap().write_all(stdin).unwrap();
    }
    c.wait_with_output().unwrap()
}

fn sturn_ok(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let o = sturn_full(dir, args, envs, &[]);
    assert!(
        o.status.success(),
        "sturn {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn sturn_err(dir: &Path, args: &[&str]) -> String {
    let o = sturn_full(dir, args, &[], &[]);
    assert!(
        !o.status.success(),
        "sturn {} unexpectedly ok",
        args.join(" ")
    );
    String::from_utf8_lossy(&o.stderr).trim().to_string()
}

fn sturn_ok_in(dir: &Path, args: &[&str], stdin: &[u8]) -> String {
    let o = sturn_full(dir, args, &[], stdin);
    assert!(
        o.status.success(),
        "sturn {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn bare_repo(name: &str) -> PathBuf {
    let d = scratch(name);
    git(&d, &["init", "-q", "-b", "main"]);
    ident(&d);
    git(
        &d,
        &[
            "-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init",
            "--allow-empty",
        ],
    );
    d
}

#[test]
fn init_errors_and_idempotent_and_repo_flag() {
    // Not a git repo → init refuses.
    let dir = scratch("notgit");
    let err = sturn_err(&dir, &["init"]);
    assert!(err.contains("not a git repo"), "{err}");

    // --repo points at a real repo while cwd isn't one.
    let repo = bare_repo("init-repo");
    let rarg = repo.to_str().unwrap().to_string();
    sturn_ok(&dir, &["--repo", &rarg, "init"], &[]);
    // sturn.toml was written into the *cwd*.
    assert!(dir.join("sturn.toml").exists());
    // Second init keeps the file and stays quiet about it.
    let out = sturn_ok(&dir, &["--repo", &rarg, "init"], &[]);
    assert!(out.contains("sturn ready"));

    // --repo / → repo_id falls back to "repo" (no file_name).
    let _ = sturn_full(&dir, &["--repo", "/", "init"], &[], &[]);
}

#[test]
fn config_driven_and_env_fallbacks() {
    let repo = bare_repo("cfg");
    // Full config exercises every cfg-driven branch in run().
    std::fs::write(
        repo.join("custom.toml"),
        b"[repo]\ndir = \".\"\nid = \"9aia/x\"\nbranch = \"dev\"\n\n\
          [actor]\nname = \"cfg-actor\"\n\n\
          [wal]\nref_prefix = \"refs/custom/wal\"\n\n\
          [bus]\nstore = \"bus.redb\"\npoll_ms = 50\nrepos = []\n\n\
          [review]\nrequire = false\n",
    )
    .unwrap();
    let cfg = repo.join("custom.toml");
    let cfgarg = cfg.to_str().unwrap();
    sturn_ok(&repo, &["--config", cfgarg, "ws", "new", "a"], &[]);
    sturn_ok_in(
        &repo,
        &["--config", cfgarg, "write", "f.txt", "--ws", "a"],
        b"x\n",
    );
    // WAL ref landed under the custom prefix, actor from config.
    let refs = git(&repo, &["for-each-ref", "--format=%(refname)", "refs/custom"]);
    assert!(refs.contains("refs/custom/wal/a"), "{refs}");
    let log = sturn_ok(&repo, &["--config", cfgarg, "log"], &[]);
    assert!(log.contains("cfg-actor"));
    sturn_ok(&repo, &["--config", cfgarg, "snap", "a", "-m", "m"], &[]);
    assert_eq!(git(&repo, &["rev-parse", "refs/heads/dev"]).len(), 40);

    // No config at all (scratch HOME, no sturn.toml): every
    // unwrap_or_else fallback — env actor, basename repo id.
    let bare = bare_repo("defaults");
    sturn_ok(&bare, &["ws", "new", "a"], &[("STURN_ACTOR", "env-actor")]);
    let log = sturn_ok(&bare, &["join", "a"], &[("STURN_ACTOR", "env-actor")]);
    assert!(log.contains("env-actor"), "{log}");
    // STURN_ACTOR + USER unset → "anon".
    let o = Command::new(env!("CARGO_BIN_EXE_sturn"))
        .current_dir(&bare)
        .args(["join", "a"])
        .env("HOME", std::env::temp_dir().join(format!("sturn-home-{}", std::process::id())))
        .env_remove("STURN_ACTOR")
        .env_remove("USER")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn write_rename_delete_release_intent_ops() {
    let repo = bare_repo("ops");
    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    // --file path + --intent flag.
    let f = scratch("payload").join("p.txt");
    std::fs::write(&f, b"payload\n").unwrap();
    let out = sturn_ok(
        &repo,
        &[
            "write", "p.txt", "--ws", "a", "--file",
            f.to_str().unwrap(), "--intent", "because",
        ],
        &[],
    );
    assert!(out.contains("write p.txt"), "{out}");
    // Missing file → error.
    let err = sturn_err(&repo, &["write", "p.txt", "--ws", "a", "--file", "/nope/nope"]);
    assert!(!err.is_empty());
    sturn_ok(&repo, &["rename", "p.txt", "q.txt", "--ws", "a"], &[]);
    sturn_ok(&repo, &["delete", "q.txt", "--ws", "a"], &[]);
    sturn_ok(&repo, &["intent", "record the why", "--ws", "a"], &[]);
    sturn_ok(&repo, &["claim", "*.txt", "--ws", "a", "--ttl", "60"], &[]);
    sturn_ok(&repo, &["release", "*.txt", "--ws", "a"], &[]);
    let log = sturn_ok(&repo, &["log", "--ws", "a"], &[]);
    for needle in ["\"write\"", "\"rename\"", "\"delete\"", "\"intent\"", "\"claim\"", "\"release\""] {
        assert!(log.contains(needle), "{needle} missing from {log}");
    }
    assert!(log.contains("because"));
}

#[test]
fn claim_conflict_force_and_claims_listing() {
    let repo = bare_repo("claimf");
    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    sturn_ok(&repo, &["ws", "new", "b"], &[]);
    sturn_ok(&repo, &["claim", "src/**", "--ws", "a"], &[]);
    // Conflict without --force → denied.
    let err = sturn_err(&repo, &["claim", "src/x.rs", "--ws", "b"]);
    assert!(err.contains("conflict") || err.contains("denied"), "{err}");
    // --force takes it anyway.
    sturn_ok(&repo, &["claim", "src/x.rs", "--ws", "b", "--force"], &[]);
    // Same-ws overlap is fine — no conflict.
    sturn_ok(&repo, &["claim", "src/y.rs", "--ws", "a"], &[]);
    let claims = sturn_ok(&repo, &["claims"], &[]);
    assert!(claims.contains("src/**") && claims.contains("src/x.rs"), "{claims}");
    // Expired-ttl claim doesn't list.
    sturn_ok(&repo, &["claim", "gone/**", "--ws", "a", "--ttl", "0"], &[]);
    assert!(!sturn_ok(&repo, &["claims"], &[]).contains("gone/**"));
}

#[test]
fn review_verdict_paths() {
    let repo = bare_repo("revv");
    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    let err = sturn_err(&repo, &["review", "e1", "maybe", "--ws", "a"]);
    assert!(err.contains("approve|reject"), "{err}");
    let out = sturn_ok(&repo, &["review", "e1", "reject", "--ws", "a"], &[]);
    assert!(out.contains("reject e1"), "{out}");
}

#[test]
fn log_and_compact_across_all_workspaces() {
    let repo = bare_repo("allws");
    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    sturn_ok(&repo, &["ws", "new", "b"], &[]);
    sturn_ok_in(&repo, &["write", "f.txt", "--ws", "a"], b"1\n");
    sturn_ok(&repo, &["snap", "a", "-m", "one"], &[]);
    sturn_ok_in(&repo, &["write", "g.txt", "--ws", "b"], b"2\n");
    let log = sturn_ok(&repo, &["log"], &[]);
    assert!(log.contains("f.txt") && log.contains("g.txt"), "{log}");
    // compact without --ws sweeps every wal.
    let out = sturn_ok(&repo, &["compact"], &[]);
    assert!(out.contains("dropped"), "{out}");
}

#[test]
fn snap_branch_flag_and_merge_fallback() {
    let repo = bare_repo("branches");
    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    sturn_ok(&repo, &["ws", "new", "b"], &[]);
    sturn_ok_in(&repo, &["write", "x", "--ws", "a"], b"1\n");
    // Explicit --branch creates+updates that branch.
    sturn_ok(&repo, &["snap", "a", "-m", "to-dev", "--branch", "dev"], &[]);
    assert_eq!(git(&repo, &["rev-parse", "refs/heads/dev"]).len(), 40);
    sturn_ok_in(&repo, &["write", "y", "--ws", "b"], b"2\n");
    // Merge on an existing branch resolves its head as base.
    sturn_ok(&repo, &["snap", "a", "b", "-m", "merge", "--branch", "dev"], &[]);
    assert_eq!(git(&repo, &["show", "dev:y"]), "2");
    // Merge onto a branch that doesn't exist → base falls back to HEAD.
    sturn_ok_in(&repo, &["write", "z", "--ws", "a"], b"3\n");
    sturn_ok_in(&repo, &["write", "w", "--ws", "b"], b"4\n");
    sturn_ok(&repo, &["snap", "a", "b", "-m", "m2", "--branch", "fresh"], &[]);
    assert_eq!(git(&repo, &["show", "fresh:z"]), "3");
    // Missing ws → snap errors.
    sturn_err(&repo, &["snap", "ghost", "-m", "x"]);
}

#[test]
fn merge_snap_review_gate() {
    let repo = bare_repo("mergegate");
    std::fs::write(
        repo.join("sturn.toml"),
        b"[repo]\ndir = \".\"\n\n[review]\nrequire = true\n",
    )
    .unwrap();
    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    sturn_ok(&repo, &["ws", "new", "b"], &[]);
    sturn_ok_in(&repo, &["write", "x", "--ws", "a"], b"1\n");
    sturn_ok_in(&repo, &["write", "y", "--ws", "b"], b"2\n");
    let err = sturn_err(&repo, &["snap", "a", "b", "-m", "x"]);
    assert!(err.contains("review gate"), "{err}");
    // Approve both write events → the merge goes through.
    let log = sturn_ok(&repo, &["log"], &[]);
    for l in log.lines().filter(|l| l.contains("\"write\"")) {
        let id = serde_json::from_str::<serde_json::Value>(l).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        sturn_ok(&repo, &["review", &id, "approve", "--ws", "a"], &[]);
    }
    sturn_ok(&repo, &["snap", "a", "b", "-m", "ok"], &[]);
    assert_eq!(git(&repo, &["show", "HEAD:y"]), "2");
}

/// Append a raw (possibly malformed or unsigned) event to a ws's
/// WAL ref via git plumbing — for signature tests.
fn wal_commit(repo: &Path, ws: &str, json: &str) {
    let blob = git_in(repo, &["hash-object", "-w", "--stdin"], json.as_bytes());
    let tree = git_in(
        repo,
        &["mktree"],
        format!("100644 blob {blob}\tevent.json\n").as_bytes(),
    );
    let o = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/sturn/wal/{ws}"),
        ])
        .output()
        .unwrap();
    let head = String::from_utf8_lossy(&o.stdout).trim().to_string();
    let commit = if head.is_empty() {
        git(repo, &["commit-tree", &tree, "-m", "raw"])
    } else {
        git(repo, &["commit-tree", &tree, "-p", &head, "-m", "raw"])
    };
    git(
        repo,
        &["update-ref", &format!("refs/sturn/wal/{ws}"), &commit],
    );
}

#[test]
fn verify_signed_unsigned_and_bad() {
    let repo = bare_repo("verify");
    // No allowed_signers anywhere → verify refuses.
    let err = sturn_err(&repo, &["verify"]);
    assert!(err.contains("allowed_signers"), "{err}");

    // Sign a throwaway ed25519 key; configure signing + signers.
    let key = repo.join("sigkey");
    let o = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(o.status.success(), "ssh-keygen: {}", String::from_utf8_lossy(&o.stderr));
    let pubkey = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    std::fs::write(repo.join("signers"), format!("t {pubkey}")).unwrap();
    std::fs::write(
        repo.join("sturn.toml"),
        format!(
            "[repo]\ndir = \".\"\n\n[actor]\nname = \"t\"\n\n[signing]\nkey = \"{}\"\nallowed_signers = \"{}\"\n",
            key.display(),
            repo.join("signers").display()
        ),
    )
    .unwrap();

    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    // Signed event.
    sturn_ok_in(&repo, &["write", "f.txt", "--ws", "a"], b"1\n");
    let out = sturn_ok(&repo, &["verify", "--ws", "a"], &[]);
    assert!(out.contains("1 ok"), "{out}");

    // Unsigned event → counted but not an error.
    wal_commit(
        &repo,
        "a",
        r#"{"id":"","ws":"a","actor":"t","ts":1,"op":{"op":"intent","text":"x"}}"#,
    );
    let out = sturn_ok(&repo, &["verify", "--ws", "a"], &[]);
    assert!(out.contains("1 unsigned"), "{out}");

    // Bad sig → verify exits non-zero.
    wal_commit(
        &repo,
        "a",
        r#"{"id":"","ws":"a","actor":"t","ts":1,"sig":"badsig","op":{"op":"intent","text":"y"}}"#,
    );
    let err = sturn_err(&repo, &["verify", "--ws", "a"]);
    assert!(err.contains("bad"), "{err}");
    // verify across all ws (no --ws).
    let _ = sturn_full(&repo, &["verify"], &[], &[]);
}

#[test]
fn push_success_and_failure() {
    let repo = bare_repo("push-src");
    let remote = scratch("push-remote");
    git(&repo, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    let out = sturn_ok(&repo, &["push"], &[]);
    assert!(out.contains("pushed"), "{out}");
    // Bogus remote → push failed.
    let err = sturn_err(&repo, &["push", "--remote", "ghost"]);
    assert!(err.contains("push failed"), "{err}");
}

#[test]
fn exec_keep_writeback_and_failures() {
    let repo = bare_repo("exec2");
    std::fs::write(repo.join("a.txt"), b"one\n").unwrap();
    git(&repo, &["add", "."]);
    commit(&repo, "add a");
    sturn_ok(&repo, &["ws", "new", "a"], &[]);

    // Failing command → sturn exits with the command's code.
    let o = sturn_full(&repo, &["exec", "--ws", "a", "--", "sh", "-c", "exit 7"], &[], &[]);
    assert_eq!(o.status.code(), Some(7));
    // Missing command → spawn error.
    let err = sturn_err(&repo, &["exec", "--ws", "a", "--", "no-such-cmd-xyz"]);
    assert!(!err.is_empty());
    // Missing ws → hydrate error.
    sturn_err(&repo, &["exec", "--ws", "ghost", "--", "true"]);

    // --writeback turns edits into ops: delete a.txt, add files,
    // a nested dir, and a symlink (skipped).
    sturn_ok(
        &repo,
        &[
            "exec", "--ws", "a", "--writeback", "--keep", "--", "sh", "-c",
            "rm a.txt; echo n > n.txt; mkdir -p deep; echo d > deep/f.txt; ln -s n.txt link",
        ],
        &[],
    );
    let log = sturn_ok(&repo, &["log", "--ws", "a"], &[]);
    assert!(log.contains("n.txt") && log.contains("deep/f.txt"), "{log}");
    assert!(log.contains("\"delete\""), "{log}");
}

#[test]
fn ws_ls_read_hydrate_paths() {
    let repo = bare_repo("wscmds");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    std::fs::write(repo.join("a.txt"), b"one\n").unwrap();
    std::fs::write(repo.join("sub/b.txt"), b"two\n").unwrap();
    git(&repo, &["add", "."]);
    commit(&repo, "files");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    sturn_ok(&repo, &["ws", "new", "a", "--base", &head], &[]);
    sturn_ok(&repo, &["ws", "new", "b"], &[]);
    // Bad base → ws new fails.
    sturn_err(&repo, &["ws", "new", "c", "--base", "bogus-ref"]);
    let ls = sturn_ok(&repo, &["ws", "ls"], &[]);
    assert!(ls.contains('a') && ls.contains('b'), "{ls}");
    let out = sturn_ok(&repo, &["ws", "read", "a", "a.txt"], &[]);
    assert_eq!(out, "one");
    sturn_err(&repo, &["ws", "read", "a", "missing.txt"]);

    // Pending ops change what hydrate sees: write new file, delete
    // a committed one, rename another (tip lookup resolves the source).
    sturn_ok_in(&repo, &["write", "pending.txt", "--ws", "a"], b"p\n");
    sturn_ok_in(&repo, &["write", "deep/new.txt", "--ws", "a"], b"n\n");
    sturn_ok(&repo, &["rename", "sub/b.txt", "sub/c.txt", "--ws", "a"], &[]);
    sturn_ok(&repo, &["delete", "a.txt", "--ws", "a"], &[]);
    let d1 = scratch("hydra1");
    sturn_ok(&repo, &["ws", "hydrate", "a", d1.to_str().unwrap()], &[]);
    assert!(d1.join("pending.txt").exists());
    assert!(d1.join("deep/new.txt").exists());
    assert!(d1.join("sub/c.txt").exists());
    assert!(!d1.join("a.txt").exists());
    assert!(!d1.join("sub/b.txt").exists());

    // --path filters the materialization.
    let d2 = scratch("hydra2");
    sturn_ok(
        &repo,
        &["ws", "hydrate", "a", d2.to_str().unwrap(), "--path", "sub"],
        &[],
    );
    assert!(d2.join("sub/c.txt").exists());

    // hydrate + --writeback in one command.
    let d3 = scratch("hydra3");
    sturn_ok(
        &repo,
        &["ws", "hydrate", "a", d3.to_str().unwrap(), "--writeback"],
        &[],
    );
    // Missing ws → hydrate fails.
    sturn_err(&repo, &["ws", "hydrate", "ghost", "/tmp/anywhere"]);
}

#[test]
fn sub_connect_error() {
    let repo = bare_repo("sub");
    let err = sturn_err(&repo, &["sub", "ops"]);
    assert!(err.contains("connect"), "{err}");
}

#[test]
fn bridge_requires_secret_env() {
    let repo = bare_repo("bridge");
    let o = sturn_full(
        &repo,
        &["bridge", "--to", "http://127.0.0.1:1/", "--secret-env", "NO_SUCH_SECRET"],
        &[],
        &[],
    );
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("not set"));
}

#[test]
fn service_deploy_destroy_with_stubbed_systemctl() {
    let repo = bare_repo("svc");
    let stub = scratch("stub-bin");
    let home = scratch("svc-home");
    // systemctl: succeed always. journalctl: fail only for -f.
    std::fs::write(stub.join("systemctl"), b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(
        stub.join("journalctl"),
        b"#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = \"-f\" ] && exit 3; done\necho log-line\nexit 0\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for n in ["systemctl", "journalctl"] {
            let mut p = std::fs::metadata(stub.join(n)).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(stub.join(n), p).unwrap();
        }
    }
    let path = format!("{}:{}", stub.display(), std::env::var("PATH").unwrap());
    let envs: Vec<(&str, &str)> = vec![("PATH", &path), ("HOME", home.to_str().unwrap())];

    sturn_ok(&repo, &["deploy", "service"], &envs);
    assert!(home.join(".config/systemd/user/sturn-daemon.service").exists());
    sturn_ok(&repo, &["deploy", "env"], &envs);
    assert!(home.join(".config/sturn/env").exists());
    sturn_ok(&repo, &["service", "enable"], &envs);
    sturn_ok(&repo, &["service", "disable"], &envs);
    let out = sturn_ok(&repo, &["service", "status"], &envs);
    assert!(out.contains("enabled="), "{out}");
    sturn_ok(&repo, &["service", "restart"], &envs);
    let out = sturn_ok(&repo, &["service", "logs"], &envs);
    assert!(out.contains("log-line"), "{out}");
    // -f → journalctl exits 3 → sturn propagates the code.
    let o = sturn_full(&repo, &["service", "logs", "-f"], &envs, &[]);
    assert_eq!(o.status.code(), Some(3));
    // destroy removes the unit file.
    sturn_ok(&repo, &["destroy", "service"], &envs);
    assert!(!home.join(".config/systemd/user/sturn-daemon.service").exists());
    // destroy again — no unit file, still succeeds.
    sturn_ok(&repo, &["destroy", "service"], &envs);
}

#[test]
fn mcp_protocol_edges() {
    let repo = bare_repo("mcp2");
    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    let home = std::env::temp_dir().join(format!("sturn-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_sturn"))
        .current_dir(&repo)
        .args(["mcp", "--ws", "a"])
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    use std::io::BufRead;
    // Notification (no id) → no response; empty line; garbage —
    // all skipped, server stays alive.
    writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
    writeln!(stdin).unwrap();
    writeln!(stdin, "not json").unwrap();
    // Unknown method → JSON-RPC error.
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"bogus/method"}}"#).unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["error"]["code"], -32601, "{v}");
    // initialize without params → default protocol version.
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"initialize"}}"#).unwrap();
    stdin.flush().unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["result"]["protocolVersion"], "2024-11-05", "{v}");
    // ping → {}.
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":3,"method":"ping"}}"#).unwrap();
    stdin.flush().unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["result"], serde_json::json!({}));
    // tools/call without params → unknown tool error inside result.
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call"}}"#).unwrap();
    stdin.flush().unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["result"]["isError"], true, "{v}");
    // Close stdin → presence "left" op lands, process exits.
    drop(stdin);
    drop(stdout);
    assert!(child.wait().unwrap().success());
    let log = sturn_ok(&repo, &["log", "--ws", "a"], &[]);
    assert!(log.contains("joined") && log.contains("left"), "{log}");
}

#[test]
fn mcp_broken_pipe_and_bad_utf8() {
    let repo = bare_repo("mcp3");
    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    // stdout closed → write fails → serve errors → exit 1.
    let mut child = Command::new(env!("CARGO_BIN_EXE_sturn"))
        .current_dir(&repo)
        .arg("mcp")
        .env("HOME", std::env::temp_dir().join(format!("sturn-home-{}", std::process::id())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdout.take().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let _ = writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"ping"}}"#);
    let _ = stdin.flush();
    drop(stdin);
    assert!(!child.wait().unwrap().success());

    // Invalid UTF-8 on stdin → read error → serve errors.
    let mut child = Command::new(env!("CARGO_BIN_EXE_sturn"))
        .current_dir(&repo)
        .arg("mcp")
        .env("HOME", std::env::temp_dir().join(format!("sturn-home-{}", std::process::id())))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&[0xff, 0xfe, b'\n']).unwrap();
    drop(stdin);
    assert!(!child.wait().unwrap().success());
}

#[test]
fn mount_real_and_missing_ws() {
    if !Path::new("/dev/fuse").exists() {
        eprintln!("no /dev/fuse — skipping mount e2e");
        return;
    }
    let repo = bare_repo("mount");
    std::fs::write(repo.join("a.txt"), b"one\n").unwrap();
    git(&repo, &["add", "."]);
    commit(&repo, "add a");
    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    // Missing ws → mount fails on tip lookup.
    sturn_err(&repo, &["mount", "ghost", "/tmp/mp-x"]);

    // RO mount: file contents readable through the kernel.
    let mp = scratch("mnt");
    let home = std::env::temp_dir().join(format!("sturn-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_sturn"))
        .current_dir(&repo)
        .args(["mount", "a", mp.to_str().unwrap()])
        .env("HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ok = false;
    for _ in 0..200 {
        if std::fs::read_to_string(mp.join("a.txt")).map(|s| s == "one\n").unwrap_or(false) {
            ok = true;
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = Command::new("fusermount3").arg("-u").arg(&mp).output();
    let status = child.wait().unwrap();
    if !ok {
        panic!("mount e2e: file never appeared (status {status:?})");
    }

    // RW mount: an edit flows back as a Write op.
    let mp2 = scratch("mnt2");
    let mut child = Command::new(env!("CARGO_BIN_EXE_sturn"))
        .current_dir(&repo)
        .args(["mount", "a", mp2.to_str().unwrap(), "--rw"])
        .env("HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if std::fs::read_dir(&mp2).map(|mut d| d.next().is_some()).unwrap_or(false) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // Drive the whole RW handler surface: rdbuf read, create, mkdir,
    // nested add_entry, rmdir (+ENOTEMPTY), rename, unlink, setattr
    // (both arms), fsync, flush/release → ops on the WAL.
    assert_eq!(std::fs::read_to_string(mp2.join("a.txt")).unwrap(), "one\n");
    std::fs::write(mp2.join("new.txt"), b"n\n").unwrap();
    std::fs::create_dir(mp2.join("empty-dir")).unwrap();
    // readdir on a kid-less dir — the `kids.get() == None` arm.
    assert_eq!(std::fs::read_dir(mp2.join("empty-dir")).unwrap().count(), 0);
    std::fs::remove_dir(mp2.join("empty-dir")).unwrap();
    std::fs::create_dir(mp2.join("sub")).unwrap();
    // Nested mkdir + rmdir — parent path joins take the format! arm.
    std::fs::create_dir(mp2.join("sub/deep")).unwrap();
    std::fs::remove_dir(mp2.join("sub/deep")).unwrap();
    std::fs::write(mp2.join("sub/deep.txt"), b"d\n").unwrap();
    assert!(std::fs::remove_dir(mp2.join("sub")).is_err());
    std::fs::rename(mp2.join("new.txt"), mp2.join("renamed.txt")).unwrap();
    std::fs::rename(mp2.join("renamed.txt"), mp2.join("sub/moved.txt")).unwrap();
    std::fs::remove_file(mp2.join("sub/moved.txt")).unwrap();
    // Dir rename with contents — descendant paths get rewritten,
    // including a descendant with an open write buffer.
    std::fs::create_dir(mp2.join("dir2")).unwrap();
    std::fs::write(mp2.join("dir2/inner.txt"), b"i\n").unwrap();
    let mut df = std::fs::OpenOptions::new()
        .write(true)
        .open(mp2.join("dir2/inner.txt"))
        .unwrap();
    df.write_all(b"opened\n").unwrap();
    std::fs::rename(mp2.join("dir2"), mp2.join("dir3")).unwrap();
    assert!(mp2.join("dir3/inner.txt").exists());
    df.write_all(b"tail\n").unwrap();
    drop(df);
    // Rename while the file is open — the open-buffer path follows.
    let mut of = std::fs::OpenOptions::new()
        .write(true)
        .open(mp2.join("a.txt"))
        .unwrap();
    of.write_all(b"open\n").unwrap();
    std::fs::rename(mp2.join("a.txt"), mp2.join("z.txt")).unwrap();
    of.write_all(b"more\n").unwrap();
    drop(of);
    std::fs::rename(mp2.join("z.txt"), mp2.join("a.txt")).unwrap();
    // chmod — setattr with mode only (no size).
    let mut perms = std::fs::metadata(mp2.join("a.txt")).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o600);
    }
    std::fs::set_permissions(mp2.join("a.txt"), perms).unwrap();
    // readdir overflow — enough long names to force reply.add to
    // report a full buffer mid-listing (the `break` arm).
    std::fs::create_dir(mp2.join("big")).unwrap();
    for i in 0..600 {
        let name = format!("{:04}-{}", i, "x".repeat(200));
        std::fs::write(mp2.join("big").join(name), b"v").unwrap();
    }
    assert!(std::fs::read_dir(mp2.join("big")).unwrap().count() >= 600);
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(mp2.join("a.txt"))
        .unwrap()
        .write_all(b"longer-content\n")
        .unwrap();
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(mp2.join("a.txt"))
        .unwrap();
    f.write_all(b"more\n").unwrap();
    f.set_len(4).unwrap();
    f.sync_all().unwrap();
    drop(f);
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(mp2.join("a.txt"))
        .unwrap();

    let _ = Command::new("fusermount3").arg("-u").arg(&mp2).output();
    assert!(child.wait().unwrap().success());
    let log = sturn_ok(&repo, &["log", "--ws", "a"], &[]);
    for needle in ["new.txt", "renamed.txt", "moved.txt", "a.txt", "\"delete\"", "\"rename\"", "\"write\""] {
        assert!(log.contains(needle), "{needle} missing from {log}");
    }
}

#[test]
fn asks_lifecycle_and_wait_codes() {
    let repo = bare_repo("asks");
    sturn_ok(&repo, &["ws", "new", "a"], &[]);
    sturn_ok(&repo, &["ws", "new", "b"], &[]);

    // ask → the printed event id is the ask handle.
    let out = sturn_ok(
        &repo,
        &[
            "ask", "yield src/**?", "--ws", "a", "--target", "human:t",
            "--option", "release", "--option", "deny",
        ],
        &[],
    );
    assert!(out.contains("open"), "{out}");
    let ask_id = out.split_whitespace().nth(1).unwrap().to_string();
    assert!(ask_id.len() >= 8, "{ask_id}");

    // asks lists it; --json carries the full record.
    let open = sturn_ok(&repo, &["asks"], &[]);
    assert!(open.contains(&ask_id) && open.contains("yield src/**?"), "{open}");
    let j = sturn_ok(&repo, &["asks", "--json"], &[]);
    let arr: serde_json::Value = serde_json::from_str(&j).unwrap();
    let e = &arr.as_array().unwrap()[0];
    assert_eq!(e["status"], "open");
    assert_eq!(e["target"], "human:t");
    assert_eq!(e["options"][1], "deny");

    // answer rides the answering ws's WAL — cross-workspace close.
    sturn_ok(&repo, &["answer", &ask_id, "release", "--ws", "b"], &[]);
    let open2 = sturn_ok(&repo, &["asks"], &[]);
    assert!(!open2.contains(&ask_id), "{open2}");
    let all = sturn_ok(&repo, &["asks", "--all"], &[]);
    assert!(all.contains("answered release"), "{all}");
    // --json --all keeps the verdict.
    let j = sturn_ok(&repo, &["asks", "--all", "--json"], &[]);
    let arr: serde_json::Value = serde_json::from_str(&j).unwrap();
    assert_eq!(arr[0]["status"], "answered");
    assert_eq!(arr[0]["verdict"], "release");

    // Closed asks refuse further terminal ops.
    sturn_err(&repo, &["answer", &ask_id, "deny", "--ws", "b"]);
    sturn_err(&repo, &["cancel", &ask_id, "--ws", "b"]);

    // wait on the answered ask exits 0 with verdict + answerer.
    let o = sturn_full(&repo, &["wait", &ask_id, "--timeout", "5"], &[], &[]);
    assert!(o.status.success());
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        out.contains("\"verdict\":\"release\"") && out.contains("\"by\":"),
        "{out}"
    );

    // Unknown ask errors everywhere.
    sturn_err(&repo, &["answer", "ghost", "v", "--ws", "b"]);
    sturn_err(&repo, &["cancel", "ghost", "--ws", "b"]);
    sturn_err(&repo, &["wait", "ghost", "--timeout", "1"]);
}

#[test]
fn asks_cancel_expire_and_wait_timeout() {
    let repo = bare_repo("asks2");
    sturn_ok(&repo, &["ws", "new", "a"], &[]);

    // cancel: open → cancelled; wait exits 2.
    let out = sturn_ok(&repo, &["ask", "deploy?", "--ws", "a", "--ttl", "0"], &[]);
    let ask_id = out.split_whitespace().nth(1).unwrap().to_string();
    sturn_ok(&repo, &["cancel", &ask_id, "--ws", "a"], &[]);
    let o = sturn_full(&repo, &["wait", &ask_id, "--timeout", "5"], &[], &[]);
    assert_eq!(o.status.code(), Some(2));
    let all = sturn_ok(&repo, &["asks", "--all"], &[]);
    assert!(all.contains("cancelled"), "{all}");

    // expiry: ttl in the past closes the ask without an op.
    let out = sturn_ok(&repo, &["ask", "ping?", "--ws", "a", "--ttl", "1"], &[]);
    let ask_id = out.split_whitespace().nth(1).unwrap().to_string();
    std::thread::sleep(std::time::Duration::from_secs(2));
    let o = sturn_full(&repo, &["wait", &ask_id, "--timeout", "5"], &[], &[]);
    assert_eq!(o.status.code(), Some(3));
    sturn_err(&repo, &["answer", &ask_id, "late", "--ws", "a"]);

    // --all --json surfaces cancelled + expired labels.
    let j = sturn_ok(&repo, &["asks", "--all", "--json"], &[]);
    assert!(j.contains("\"status\":\"cancelled\""), "{j}");
    assert!(j.contains("\"status\":\"expired\""), "{j}");

    // timeout: an ask that never resolves exits 4.
    let out = sturn_ok(&repo, &["ask", "nobody?", "--ws", "a", "--ttl", "0"], &[]);
    let ask_id = out.split_whitespace().nth(1).unwrap().to_string();
    let o = sturn_full(
        &repo,
        &["wait", &ask_id, "--timeout", "1", "--poll", "50"],
        &[],
        &[],
    );
    assert_eq!(o.status.code(), Some(4));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("\"status\":\"timeout\""), "{out}");
}
