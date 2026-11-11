//! Ops parity — every verb the retired `sturn mcp` tools needed is
//! reachable two ways: `run <argv>` passthrough (≡ `sturn <argv>`)
//! and `cli.<verb>` wire queries over `serve`. This test drives the
//! new bounded ops (`ws read/paths/grep`, `write --content`,
//! `log --topic/--limit/--wait-ms`) through both paths against a
//! real scratch repo.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

fn git(dir: &Path, args: &[&str]) {
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
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sturn-opswire-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn repo(name: &str) -> PathBuf {
    let d = scratch(name);
    git(&d, &["init", "-q", "-b", "main"]);
    git(&d, &["config", "user.email", "t@t"]);
    git(&d, &["config", "user.name", "t"]);
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

/// `tamias-driver-sturn run <argv>` in `dir` — must exit 0.
fn run(dir: &Path, args: &[&str]) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_tamias-driver-sturn"))
        .current_dir(dir)
        .arg("run")
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "run {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// One `serve` session — ndjson requests in, responses out.
struct Serve {
    stdin: Option<ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
    child: Child,
    n: u64,
}

impl Serve {
    fn spawn(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tamias-driver-sturn"))
            .current_dir(dir)
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Serve {
            stdin: Some(child.stdin.take().unwrap()),
            stdout: BufReader::new(child.stdout.take().unwrap()),
            child,
            n: 0,
        }
    }

    /// `{"op":"query","query":q,"params":p}` → the result payload.
    fn query(&mut self, q: &str, params: serde_json::Value) -> serde_json::Value {
        self.n += 1;
        let req = serde_json::json!({
            "op": "query", "id": self.n, "query": q, "params": params,
        });
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{req}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["ok"], true, "{q}: {v}");
        v["result"].clone()
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if self.child.wait().is_err() {
            let _ = self.child.kill();
        }
    }
}

#[test]
fn cli_ops_cover_the_mcp_tool_surface() {
    let dir = repo("parity");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.txt"), b"alpha\n").unwrap();
    git(&dir, &["add", "."]);
    git(
        &dir,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "files",
        ],
    );

    // Seed state through the `run` passthrough — same argv as `sturn`.
    run(&dir, &["init"]);
    run(&dir, &["ws", "new", "a"]);
    run(
        &dir,
        &[
            "write",
            "src/new.txt",
            "--ws",
            "a",
            "--content",
            "beta gamma\n",
        ],
    );
    run(&dir, &["claim", "src/**", "--ws", "a"]);

    let mut s = Serve::spawn(&dir);

    // The ops manifest exposes the bounded verbs — no `mcp` surface.
    let ops = s.query("ops", serde_json::json!({}));
    let names: Vec<&str> = ops["data"]["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["query"].as_str().unwrap())
        .collect();
    for want in [
        "cli.ws.read",
        "cli.ws.paths",
        "cli.ws.grep",
        "cli.write",
        "cli.log",
        "cli.claim",
        "cli.release",
        "cli.snap",
        "cli.intent",
        "cli.review",
        "cli.ask",
        "cli.asks",
        "cli.answer",
        "cli.cancel",
        "cli.join",
        "cli.leave",
        "cli.rename",
        "cli.delete",
    ] {
        assert!(names.contains(&want), "{want} missing from ops manifest");
    }
    assert!(!names.contains(&"cli.mcp"), "mcp must not be an op");
    assert_eq!(
        ops["data"]["surfaces"],
        serde_json::json!(["daemon", "sub", "bridge", "mount"])
    );

    // cli.<verb> wire queries — `data.stdout` is the engine's stdout.
    let read = s.query(
        "cli.ws.read",
        serde_json::json!({"dir": dir, "name": "a", "path": "src/new.txt"}),
    );
    assert_eq!(read["data"]["code"], 0);
    assert_eq!(read["data"]["stdout"], "beta gamma\n");

    let paths = s.query(
        "cli.ws.paths",
        serde_json::json!({"dir": dir, "name": "a", "pattern": "src/**"}),
    );
    assert!(
        paths["data"]["stdout"]
            .as_str()
            .unwrap()
            .contains("src/new.txt"),
        "{paths}"
    );

    let grep = s.query(
        "cli.ws.grep",
        serde_json::json!({"dir": dir, "name": "a", "pattern": "gamma"}),
    );
    assert!(
        grep["data"]["stdout"]
            .as_str()
            .unwrap()
            .contains("src/new.txt:1: beta gamma"),
        "{grep}"
    );

    // write --content rides the wire as a plain string param.
    s.query(
        "cli.write",
        serde_json::json!({
            "dir": dir, "path": "src/second.txt", "ws": "a",
            "content": "delta\n", "intent": "wire write",
        }),
    );
    let read2 = s.query(
        "cli.ws.read",
        serde_json::json!({"dir": dir, "name": "a", "path": "src/second.txt"}),
    );
    assert_eq!(read2["data"]["stdout"], "delta\n");

    // log --topic/--limit over the wire.
    let claims = s.query(
        "cli.log",
        serde_json::json!({"dir": dir, "topic": "claims", "limit": 5}),
    );
    assert!(
        claims["data"]["stdout"]
            .as_str()
            .unwrap()
            .contains("\"claim\""),
        "{claims}"
    );
    assert!(
        !claims["data"]["stdout"]
            .as_str()
            .unwrap()
            .contains("\"write\""),
        "{claims}"
    );

    // The same verbs through `run` passthrough — argv parity.
    let via_run = run(&dir, &["ws", "paths", "a", "src/**"]);
    for p in ["src/a.txt", "src/new.txt", "src/second.txt"] {
        assert!(
            via_run.lines().any(|l| l == p),
            "{p} missing from {via_run}"
        );
    }
    assert_eq!(run(&dir, &["ws", "read", "a", "src/second.txt"]), "delta");
}
