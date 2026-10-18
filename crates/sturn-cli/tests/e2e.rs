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

/// One JSON-RPC exchange with `sturn mcp` — newline-delimited, per
/// the MCP stdio transport.
struct Mcp {
    stdin: std::process::ChildStdin,
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
            stdin: child.stdin.take().unwrap(),
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
        writeln!(self.stdin, "{req}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
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
        12,
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
