//! The agent surface — the tool vocabulary every sturn agent sees.
//! No git, no filesystem: the VCS stops being a tool the agent
//! operates and becomes the medium it lives in.
//!
//! `serve` runs a real MCP stdio server — newline-delimited JSON-RPC.
//! Reads are overlay-aware: `ws_read`/`ws_glob`/`ws_grep` see
//! uncommitted ops, not just the ws tip commit.

use serde_json::{json, Value};
use std::io::{BufRead, Write as IoWrite};
use std::path::Path;
use sturn_claim as claims;
use sturn_core::{overlay, since_last_snapshot, topic, Event, Op, Verdict};
use sturn_wal::Wal;
use sturn_ws::Ws;

/// One tool in the agent vocabulary.
pub struct Tool {
    /// MCP tool name.
    pub name: &'static str,
    /// What the agent reads before calling it.
    pub about: &'static str,
}

/// The whole AX — nine tools instead of git's thirty-something.
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "ws_read",
        about: "Blob contents at ws:path — straight from the ODB",
    },
    Tool {
        name: "ws_write",
        about: "Append a write op — visible on the ops topic in ms",
    },
    Tool {
        name: "ws_glob",
        about: "Path listing for a ws tree",
    },
    Tool {
        name: "ws_grep",
        about: "Content search across a ws tree",
    },
    Tool {
        name: "ws_rename",
        about: "Move a path — the intent survives where git loses it",
    },
    Tool {
        name: "ws_delete",
        about: "Delete a path",
    },
    Tool {
        name: "claim",
        about: "Advisory lock on a path glob — conflict answer now, not at merge",
    },
    Tool {
        name: "subs",
        about: "Live topic feed — claims, ops, review, presence",
    },
    Tool {
        name: "snap",
        about: "Squash ws ops into a real commit on the branch",
    },
    Tool {
        name: "intent",
        about: "Record *why* — provenance as data, not a commit message",
    },
    Tool {
        name: "review",
        about: "Approve/reject an event — continuous review",
    },
];

/// Runtime context the tools close over — wired by the composition
/// root (`sturn mcp`).
pub struct Ctx<'a> {
    /// Repo dir.
    pub repo: &'a Path,
    /// Topic prefix id for feeds.
    pub repo_id: &'a str,
    /// WAL impl.
    pub wal: &'a dyn Wal,
    /// ODB impl.
    pub wsi: &'a dyn Ws,
    /// Actor name on appended ops.
    pub actor: &'a str,
    /// Default snap target branch.
    pub branch: &'a str,
    /// The agent's own ws — presence ops attach here.
    pub ws: Option<&'a str>,
    /// ws, message, branch → new commit sha (also moves the ws ref
    /// and appends the Snapshot marker — cli supplies the body).
    pub snap: &'a dyn Fn(&str, &str, &str) -> std::io::Result<String>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A ws's effective view: tip commit + WAL overlay since the last
/// snapshot. This is what makes the workspace real for agents —
/// their own uncommitted writes are readable immediately.
fn view(ctx: &Ctx, ws: &str) -> std::io::Result<(String, Vec<Event>)> {
    let tip = ctx.wsi.tip(ctx.repo, ws)?;
    let events = ctx.wal.events(ctx.repo, ws)?;
    let since = since_last_snapshot(&events);
    Ok((tip, events[since..].to_vec()))
}

fn tool_text(v: impl std::fmt::Display) -> Value {
    json!({"content": [{"type": "text", "text": v.to_string()}]})
}

fn tool_err(v: impl std::fmt::Display) -> Value {
    json!({"content": [{"type": "text", "text": v.to_string()}], "isError": true})
}

fn arg<'a>(a: &'a Value, k: &str) -> Result<&'a str, String> {
    a.get(k)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing arg `{k}`"))
}

/// Dispatch one `tools/call` — returns the result value.
fn call(ctx: &Ctx, name: &str, a: &Value) -> Value {
    match name {
        "ws_read" => ws_read(ctx, a),
        "ws_write" => ws_write(ctx, a),
        "ws_glob" => ws_glob(ctx, a),
        "ws_grep" => ws_grep(ctx, a),
        "ws_rename" => ws_rename(ctx, a),
        "ws_delete" => ws_delete(ctx, a),
        "claim" => claim(ctx, a),
        "release" => release(ctx, a),
        "snap" => snap(ctx, a),
        "intent" => intent(ctx, a),
        "review" => review(ctx, a),
        "subs" => subs(ctx, a),
        _ => tool_err(format!("unknown tool `{name}`")),
    }
}

fn ws_read(ctx: &Ctx, a: &Value) -> Value {
    let (ws, path) = match (arg(a, "ws"), arg(a, "path")) {
        (Ok(w), Ok(p)) => (w, p),
        (Err(e), _) | (_, Err(e)) => return tool_err(e),
    };
    let r = (|| -> std::io::Result<String> {
        let (tip, pending) = view(ctx, ws)?;
        let ov = overlay(&pending, |p| {
            ctx.wsi.entry(ctx.repo, &tip, p).ok().flatten()
        });
        if let Some(e) = ov.get(path) {
            return match e {
                Some(sha) => {
                    Ok(String::from_utf8_lossy(&ctx.wsi.blob(ctx.repo, sha)?).into_owned())
                }
                None => Err(std::io::Error::other("path deleted in ws overlay")),
            };
        }
        Ok(String::from_utf8_lossy(&ctx.wsi.read(ctx.repo, &tip, path)?).into_owned())
    })();
    r.map_or_else(|e| tool_err(e), tool_text)
}

fn ws_write(ctx: &Ctx, a: &Value) -> Value {
    let (ws, path, content) = match (arg(a, "ws"), arg(a, "path"), arg(a, "content")) {
        (Ok(w), Ok(p), Ok(c)) => (w, p, c),
        _ => return tool_err("missing ws/path/content"),
    };
    let r = (|| -> std::io::Result<String> {
        let blob = ctx.wsi.store(ctx.repo, content.as_bytes())?;
        ctx.wal.append(
            ctx.repo,
            ws,
            &Event::new(
                ws,
                ctx.actor,
                Op::Write {
                    path: path.into(),
                    blob,
                    intent: a.get("intent").and_then(|v| v.as_str()).map(String::from),
                },
            ),
        )
    })();
    r.map_or_else(|e| tool_err(e), |id| tool_text(format!("event {id}")))
}

fn ws_paths(ctx: &Ctx, ws: &str) -> std::io::Result<Vec<String>> {
    let (tip, pending) = view(ctx, ws)?;
    let mut paths = ctx.wsi.list(ctx.repo, &tip)?;
    for (p, v) in overlay(&pending, |p| {
        ctx.wsi.entry(ctx.repo, &tip, p).ok().flatten()
    }) {
        match v {
            Some(_) => paths.push(p),
            None => paths.retain(|x| *x != p),
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn ws_glob(ctx: &Ctx, a: &Value) -> Value {
    let ws = match arg(a, "ws") {
        Ok(w) => w,
        Err(e) => return tool_err(e),
    };
    let pat = a.get("pattern").and_then(|v| v.as_str()).unwrap_or("**");
    ws_paths(ctx, ws)
        .map(|ps| {
            let hits: Vec<_> = ps
                .into_iter()
                .filter(|p| claims::glob_match(pat, p))
                .collect();
            tool_text(hits.join("\n"))
        })
        .unwrap_or_else(tool_err)
}

fn ws_grep(ctx: &Ctx, a: &Value) -> Value {
    let (ws, pat) = match (arg(a, "ws"), arg(a, "pattern")) {
        (Ok(w), Ok(p)) => (w, p),
        _ => return tool_err("missing ws/pattern"),
    };
    let r = (|| -> std::io::Result<String> {
        let (tip, pending) = view(ctx, ws)?;
        let ov = overlay(&pending, |p| {
            ctx.wsi.entry(ctx.repo, &tip, p).ok().flatten()
        });
        let mut hits = Vec::new();
        for p in ws_paths(ctx, ws)? {
            let bytes = match ov.get(&p) {
                Some(Some(sha)) => ctx.wsi.blob(ctx.repo, sha)?,
                Some(None) => continue,
                None => ctx.wsi.read(ctx.repo, &tip, &p)?,
            };
            for (n, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
                if line.contains(pat) {
                    hits.push(format!("{p}:{}: {line}", n + 1));
                }
            }
        }
        Ok(hits.join("\n"))
    })();
    r.map_or_else(|e| tool_err(e), tool_text)
}

fn ws_rename(ctx: &Ctx, a: &Value) -> Value {
    let (ws, from, to) = match (arg(a, "ws"), arg(a, "from"), arg(a, "to")) {
        (Ok(w), Ok(f), Ok(t)) => (w, f, t),
        _ => return tool_err("missing ws/from/to"),
    };
    ctx.wal
        .append(
            ctx.repo,
            ws,
            &Event::new(
                ws,
                ctx.actor,
                Op::Rename {
                    from: from.into(),
                    to: to.into(),
                },
            ),
        )
        .map(|id| tool_text(format!("rename {from} → {to} (event {id})")))
        .unwrap_or_else(|e| tool_err(e))
}

fn ws_delete(ctx: &Ctx, a: &Value) -> Value {
    let (ws, path) = match (arg(a, "ws"), arg(a, "path")) {
        (Ok(w), Ok(p)) => (w, p),
        (Err(e), _) | (_, Err(e)) => return tool_err(e),
    };
    ctx.wal
        .append(
            ctx.repo,
            ws,
            &Event::new(ws, ctx.actor, Op::Delete { path: path.into() }),
        )
        .map(|id| tool_text(format!("delete {path} (event {id})")))
        .unwrap_or_else(|e| tool_err(e))
}

fn claim(ctx: &Ctx, a: &Value) -> Value {
    let (ws, glob) = match (arg(a, "ws"), arg(a, "glob")) {
        (Ok(w), Ok(g)) => (w, g),
        _ => return tool_err("missing ws/glob"),
    };
    let r = (|| -> Result<String, String> {
        let mut all = Vec::new();
        for w in ctx.wal.workspaces(ctx.repo).map_err(|e| e.to_string())? {
            all.extend(ctx.wal.events(ctx.repo, &w).map_err(|e| e.to_string())?);
        }
        let live = claims::active(&all, now());
        let hits: Vec<_> = claims::conflicts(&live, glob)
            .into_iter()
            .filter(|c| c.ws != ws)
            .collect();
        if !hits.is_empty() {
            let detail = hits
                .iter()
                .map(|c| format!("{} holds {} (ws {})", c.actor, c.glob, c.ws))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(format!("conflict: {detail}"));
        }
        let ttl = a.get("ttl_secs").and_then(|v| v.as_u64()).unwrap_or(3600);
        ctx.wal
            .append(
                ctx.repo,
                ws,
                &Event::new(
                    ws,
                    ctx.actor,
                    Op::Claim {
                        glob: glob.into(),
                        ttl_secs: ttl,
                    },
                ),
            )
            .map_err(|e| e.to_string())?;
        Ok(format!("claimed {glob}"))
    })();
    r.map_or_else(|e| tool_err(e), tool_text)
}

fn release(ctx: &Ctx, a: &Value) -> Value {
    let (ws, glob) = match (arg(a, "ws"), arg(a, "glob")) {
        (Ok(w), Ok(g)) => (w, g),
        _ => return tool_err("missing ws/glob"),
    };
    ctx.wal
        .append(
            ctx.repo,
            ws,
            &Event::new(ws, ctx.actor, Op::Release { glob: glob.into() }),
        )
        .map(|_| tool_text(format!("released {glob}")))
        .unwrap_or_else(|e| tool_err(e))
}

fn snap(ctx: &Ctx, a: &Value) -> Value {
    let (ws, msg) = match (arg(a, "ws"), arg(a, "message")) {
        (Ok(w), Ok(m)) => (w, m),
        _ => return tool_err("missing ws/message"),
    };
    let branch = a
        .get("branch")
        .and_then(|v| v.as_str())
        .unwrap_or(ctx.branch);
    (ctx.snap)(ws, msg, branch)
        .map(|c| tool_text(format!("snap {c} → {branch}")))
        .unwrap_or_else(|e| tool_err(e))
}

fn intent(ctx: &Ctx, a: &Value) -> Value {
    let (ws, text) = match (arg(a, "ws"), arg(a, "text")) {
        (Ok(w), Ok(t)) => (w, t),
        _ => return tool_err("missing ws/text"),
    };
    ctx.wal
        .append(
            ctx.repo,
            ws,
            &Event::new(ws, ctx.actor, Op::Intent { text: text.into() }),
        )
        .map(|id| tool_text(format!("intent {id}")))
        .unwrap_or_else(|e| tool_err(e))
}

fn review(ctx: &Ctx, a: &Value) -> Value {
    let (ws, event, verdict) = match (arg(a, "ws"), arg(a, "event"), arg(a, "verdict")) {
        (Ok(w), Ok(e), Ok(v)) => (w, e, v),
        _ => return tool_err("missing ws/event/verdict"),
    };
    let v = match verdict {
        "approve" => Verdict::Approve,
        "reject" => Verdict::Reject,
        _ => return tool_err("verdict: approve|reject"),
    };
    ctx.wal
        .append(
            ctx.repo,
            ws,
            &Event::new(
                ws,
                ctx.actor,
                Op::Review {
                    event: event.into(),
                    verdict: v,
                },
            ),
        )
        .map(|id| tool_text(format!("{verdict} {event} (event {id})")))
        .unwrap_or_else(|e| tool_err(e))
}

/// Pull-replay feed — or long-poll: `wait_ms` blocks until *new*
/// events land (vs. the entry snapshot) or the deadline passes.
/// That's the agent's live feed inside request/response MCP.
fn subs(ctx: &Ctx, a: &Value) -> Value {
    let topic_name = a
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or(topic::OPS);
    let limit = a.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
    let wait_ms = a.get("wait_ms").and_then(|v| v.as_u64()).unwrap_or(0);
    let baseline: Option<std::collections::HashSet<String>> = if wait_ms > 0 {
        (|| -> std::io::Result<_> {
            let mut ids = std::collections::HashSet::new();
            for w in ctx.wal.workspaces(ctx.repo)? {
                ids.extend(ctx.wal.events(ctx.repo, &w)?.into_iter().map(|e| e.id));
            }
            Ok(ids)
        })()
        .ok()
    } else {
        None
    };
    if let Some(base) = &baseline {
        let deadline = now() * 1000 + wait_ms;
        loop {
            let fresh = (|| -> std::io::Result<usize> {
                let mut n = 0;
                for w in ctx.wal.workspaces(ctx.repo)? {
                    for e in ctx.wal.events(ctx.repo, &w)? {
                        if !base.contains(&e.id) {
                            n += 1;
                        }
                    }
                }
                Ok(n)
            })()
            .unwrap_or(0);
            if fresh > 0 || now() * 1000 >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    let r = (|| -> std::io::Result<String> {
        let mut all = Vec::new();
        for w in ctx.wal.workspaces(ctx.repo)? {
            all.extend(ctx.wal.events(ctx.repo, &w)?);
        }
        let keep = |ev: &Event| match topic_name {
            t if t == topic::OPS => true,
            t if t == topic::CLAIMS => {
                matches!(ev.op, Op::Claim { .. } | Op::Release { .. })
            }
            t if t == topic::REVIEW => matches!(ev.op, Op::Review { .. }),
            t if t == topic::PRESENCE => matches!(ev.op, Op::Presence { .. }),
            _ => false,
        };
        let mut lines: Vec<String> = all
            .iter()
            .filter(|e| keep(e))
            .map(|e| serde_json::to_string(e).unwrap_or_default())
            .collect();
        let start = lines.len().saturating_sub(limit);
        Ok(lines.drain(start..).collect::<Vec<_>>().join("\n"))
    })();
    r.map_or_else(|e| tool_err(e), tool_text)
}

fn tool_schema(name: &str, about: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": about,
        "inputSchema": {
            "type": "object",
            "properties": props,
            "required": required,
        }
    })
}

fn tool_list() -> Value {
    let s = |k: &str| (k.to_string(), json!({"type": "string"}));
    let ws = s("ws");
    json!({"tools": [
        tool_schema("ws_read", "Read a file from a ws (sees uncommitted ops)", json!({"ws": ws.1, "path": s("path").1}), &["ws","path"]),
        tool_schema("ws_write", "Write file content as an op", json!({"ws": ws.1, "path": s("path").1, "content": s("content").1, "intent": s("intent").1}), &["ws","path","content"]),
        tool_schema("ws_glob", "List paths matching a glob in a ws", json!({"ws": ws.1, "pattern": s("pattern").1}), &["ws"]),
        tool_schema("ws_grep", "Substring search across a ws", json!({"ws": ws.1, "pattern": s("pattern").1}), &["ws","pattern"]),
        tool_schema("ws_rename", "Move a path (intent survives)", json!({"ws": ws.1, "from": s("from").1, "to": s("to").1}), &["ws","from","to"]),
        tool_schema("ws_delete", "Delete a path", json!({"ws": ws.1, "path": s("path").1}), &["ws","path"]),
        tool_schema("claim", "Advisory lock on a path glob — conflicts before work", json!({"ws": ws.1, "glob": s("glob").1, "ttl_secs": {"type":"integer"}}), &["ws","glob"]),
        tool_schema("release", "Drop a claim", json!({"ws": ws.1, "glob": s("glob").1}), &["ws","glob"]),
        tool_schema("snap", "Fold ws ops into a real commit", json!({"ws": ws.1, "message": s("message").1, "branch": s("branch").1}), &["ws","message"]),
        tool_schema("intent", "Record why work exists", json!({"ws": ws.1, "text": s("text").1}), &["ws","text"]),
        tool_schema("review", "Approve/reject an event id", json!({"ws": ws.1, "event": s("event").1, "verdict": s("verdict").1}), &["ws","event","verdict"]),
        tool_schema("subs", "Recent events on a topic; wait_ms long-polls for new ones", json!({"topic": s("topic").1, "limit": {"type":"integer"}, "wait_ms": {"type":"integer"}}), &[]),
    ]})
}

/// Serve newline-delimited JSON-RPC (MCP stdio transport) until
/// stdin closes. Requests and notifications interleave; only
/// requests get responses.
pub fn serve(ctx: &Ctx) -> std::io::Result<()> {
    // Presence is lifecycle, not a tool — connect/disconnect emit ops.
    if let Some(ws) = ctx.ws {
        let _ = ctx.wal.append(
            ctx.repo,
            ws,
            &Event::new(
                ws,
                ctx.actor,
                Op::Presence {
                    status: "joined".into(),
                },
            ),
        );
    }
    let stdin = std::io::stdin().lock();
    let mut out = std::io::stdout().lock();
    for line in stdin.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Notifications have no id — never respond to them.
        let Some(id) = req.get("id").cloned() else {
            continue;
        };
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let result = match method {
            "initialize" => json!({
                "protocolVersion": req.get("params")
                    .and_then(|p| p.get("protocolVersion"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("2024-11-05"),
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "sturn", "version": env!("CARGO_PKG_VERSION")},
            }),
            "ping" => json!({}),
            "tools/list" => tool_list(),
            "tools/call" => {
                let p = req.get("params").cloned().unwrap_or(json!({}));
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = p.get("arguments").cloned().unwrap_or(json!({}));
                call(ctx, name, &args)
            }
            other => {
                let resp = json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": format!("method not found: {other}")},
                });
                out.write_all(resp.to_string().as_bytes())?;
                out.write_all(b"\n")?;
                out.flush()?;
                continue;
            }
        };
        let resp = json!({"jsonrpc": "2.0", "id": id, "result": result});
        out.write_all(resp.to_string().as_bytes())?;
        out.write_all(b"\n")?;
        out.flush()?;
    }
    if let Some(ws) = ctx.ws {
        let _ = ctx.wal.append(
            ctx.repo,
            ws,
            &Event::new(
                ws,
                ctx.actor,
                Op::Presence {
                    status: "left".into(),
                },
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Tool-level coverage: every `tools/call` verb against in-memory
    //! Wal/Ws fakes, plus the tool schema list. `serve` itself (the
    //! stdio loop) is exercised by sturn-cli's e2e.

    use super::*;
    use std::collections::BTreeMap;
    use std::io;
    use std::sync::Mutex;

    struct FakeWal {
        log: Mutex<BTreeMap<String, Vec<Event>>>,
        n: Mutex<u64>,
    }

    impl FakeWal {
        fn new() -> Self {
            Self {
                log: Mutex::new(BTreeMap::new()),
                n: Mutex::new(0),
            }
        }
    }

    impl Wal for FakeWal {
        fn append(&self, _r: &Path, ws: &str, e: &Event) -> io::Result<String> {
            let mut ev = e.clone();
            let mut n = self.n.lock().unwrap();
            *n += 1;
            ev.id = format!("e{n}");
            let id = ev.id.clone();
            self.log
                .lock()
                .unwrap()
                .entry(ws.to_string())
                .or_default()
                .push(ev);
            Ok(id)
        }
        fn events_since(&self, _r: &Path, ws: &str, from: Option<&str>) -> io::Result<Vec<Event>> {
            let log = self.log.lock().unwrap();
            let all = log.get(ws).cloned().unwrap_or_default();
            Ok(match from {
                None => all,
                Some(f) => {
                    let pos = all.iter().position(|e| e.id == f).map_or(0, |i| i + 1);
                    all[pos..].to_vec()
                }
            })
        }
        fn head(&self, _r: &Path, ws: &str) -> io::Result<Option<String>> {
            Ok(self
                .log
                .lock()
                .unwrap()
                .get(ws)
                .and_then(|v| v.last())
                .map(|e| e.id.clone()))
        }
        fn workspaces(&self, _r: &Path) -> io::Result<Vec<String>> {
            Ok(self.log.lock().unwrap().keys().cloned().collect())
        }
        fn compact(&self, _r: &Path, _ws: &str) -> io::Result<usize> {
            Ok(0)
        }
    }

    struct FakeWs {
        /// Tip-tree contents: path → bytes.
        files: BTreeMap<String, Vec<u8>>,
        /// ODB: sha → bytes.
        blobs: Mutex<BTreeMap<String, Vec<u8>>>,
        tips: Mutex<BTreeMap<String, String>>,
        n: Mutex<u64>,
    }

    impl FakeWs {
        fn new() -> Self {
            let mut files = BTreeMap::new();
            files.insert("a.txt".to_string(), b"alpha\nline two\n".to_vec());
            files.insert("src/b.rs".to_string(), b"fn b() {}\n".to_vec());
            Self {
                files,
                blobs: Mutex::new(BTreeMap::new()),
                tips: Mutex::new(BTreeMap::new()),
                n: Mutex::new(0),
            }
        }
    }

    impl Ws for FakeWs {
        fn read(&self, _r: &Path, _rev: &str, path: &str) -> io::Result<Vec<u8>> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::other("not found"))
        }
        fn list(&self, _r: &Path, _rev: &str) -> io::Result<Vec<String>> {
            Ok(self.files.keys().cloned().collect())
        }
        fn materialize(
            &self,
            _r: &Path,
            _rev: &str,
            _dir: &Path,
            _paths: &[String],
        ) -> io::Result<()> {
            Ok(())
        }
        fn create(&self, _r: &Path, name: &str, base: &str) -> io::Result<()> {
            self.tips
                .lock()
                .unwrap()
                .insert(name.to_string(), base.to_string());
            Ok(())
        }
        fn entry(&self, _r: &Path, _rev: &str, path: &str) -> io::Result<Option<String>> {
            Ok(self.files.get(path).map(|c| {
                // Register the tip content as a blob so `blob()` on
                // the returned sha resolves, like a real ODB.
                let sha = format!("tip-{path}");
                self.blobs.lock().unwrap().insert(sha.clone(), c.clone());
                sha
            }))
        }
        fn blob(&self, _r: &Path, sha: &str) -> io::Result<Vec<u8>> {
            self.blobs
                .lock()
                .unwrap()
                .get(sha)
                .cloned()
                .ok_or_else(|| io::Error::other("blob not found"))
        }
        fn store(&self, _r: &Path, content: &[u8]) -> io::Result<String> {
            let mut n = self.n.lock().unwrap();
            *n += 1;
            let sha = format!("blob{n}");
            self.blobs
                .lock()
                .unwrap()
                .insert(sha.clone(), content.to_vec());
            Ok(sha)
        }
        fn tip(&self, _r: &Path, name: &str) -> io::Result<String> {
            Ok(self
                .tips
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .unwrap_or_else(|| "tip".into()))
        }
        fn workspaces(&self, _r: &Path) -> io::Result<Vec<String>> {
            Ok(self.tips.lock().unwrap().keys().cloned().collect())
        }
    }

    fn ctx<'a>(
        wal: &'a FakeWal,
        wsi: &'a FakeWs,
        snap: &'a dyn Fn(&str, &str, &str) -> io::Result<String>,
    ) -> Ctx<'a> {
        Ctx {
            repo: Path::new("/repo"),
            repo_id: "test/repo",
            wal,
            wsi,
            actor: "agent-a",
            branch: "main",
            ws: Some("w1"),
            snap,
        }
    }

    fn text(v: &Value) -> &str {
        v["content"][0]["text"].as_str().unwrap()
    }

    fn is_err(v: &Value) -> bool {
        v.get("isError").and_then(|b| b.as_bool()).unwrap_or(false)
    }

    fn ok_snap(_w: &str, _m: &str, _b: &str) -> io::Result<String> {
        Ok("cafe123".into())
    }

    #[test]
    fn tool_vocabulary_and_schemas() {
        assert_eq!(TOOLS.len(), 11);
        let list = tool_list();
        let tools = list["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 12); // list includes `release`
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
    }

    #[test]
    fn ws_read_from_tip_and_overlay() {
        let wal = FakeWal::new();
        let wsi = FakeWs::new();
        let c = ctx(&wal, &wsi, &ok_snap);

        // Tip read.
        let v = call(&c, "ws_read", &json!({"ws": "w1", "path": "a.txt"}));
        assert!(!is_err(&v));
        assert!(text(&v).contains("alpha"));

        // Overlay read: a pending Write wins over the tip.
        let sha = wsi.store(Path::new("/repo"), b"pending\n").unwrap();
        wal.append(
            Path::new("/repo"),
            "w1",
            &Event::new(
                "w1",
                "agent-a",
                Op::Write {
                    path: "new.rs".into(),
                    blob: sha,
                    intent: None,
                },
            ),
        )
        .unwrap();
        let v = call(&c, "ws_read", &json!({"ws": "w1", "path": "new.rs"}));
        assert_eq!(text(&v), "pending\n");

        // Deleted in overlay → error.
        wal.append(
            Path::new("/repo"),
            "w1",
            &Event::new(
                "w1",
                "agent-a",
                Op::Delete {
                    path: "a.txt".into(),
                },
            ),
        )
        .unwrap();
        let v = call(&c, "ws_read", &json!({"ws": "w1", "path": "a.txt"}));
        assert!(is_err(&v));

        // Missing arg → error.
        assert!(is_err(&call(&c, "ws_read", &json!({"ws": "w1"}))));
    }

    #[test]
    fn ws_write_appends_op() {
        let wal = FakeWal::new();
        let wsi = FakeWs::new();
        let c = ctx(&wal, &wsi, &ok_snap);
        let v = call(
            &c,
            "ws_write",
            &json!({"ws": "w1", "path": "p.rs", "content": "x", "intent": "why"}),
        );
        assert!(!is_err(&v));
        assert!(text(&v).starts_with("event e"));
        let events = wal.events(Path::new("/repo"), "w1").unwrap();
        match &events.last().unwrap().op {
            Op::Write { path, intent, .. } => {
                assert_eq!(path, "p.rs");
                assert_eq!(intent.as_deref(), Some("why"));
            }
            other => panic!("expected Write, got {other:?}"),
        }
        assert!(is_err(&call(&c, "ws_write", &json!({"ws": "w1"}))));
    }

    #[test]
    fn ws_glob_filters_paths() {
        let wal = FakeWal::new();
        let wsi = FakeWs::new();
        let c = ctx(&wal, &wsi, &ok_snap);
        let v = call(&c, "ws_glob", &json!({"ws": "w1", "pattern": "src/**"}));
        assert_eq!(text(&v), "src/b.rs");
        // Default pattern lists everything.
        let v = call(&c, "ws_glob", &json!({"ws": "w1"}));
        assert!(text(&v).contains("a.txt"));
        assert!(text(&v).contains("src/b.rs"));
    }

    #[test]
    fn ws_grep_finds_lines() {
        let wal = FakeWal::new();
        let wsi = FakeWs::new();
        let c = ctx(&wal, &wsi, &ok_snap);
        let v = call(&c, "ws_grep", &json!({"ws": "w1", "pattern": "line"}));
        assert!(text(&v).contains("a.txt:2: line two"));
        assert!(!text(&v).contains("b.rs"));
        assert!(is_err(&call(&c, "ws_grep", &json!({"ws": "w1"}))));
    }

    #[test]
    fn ws_rename_and_delete_append_ops() {
        let wal = FakeWal::new();
        let wsi = FakeWs::new();
        let c = ctx(&wal, &wsi, &ok_snap);
        let v = call(
            &c,
            "ws_rename",
            &json!({"ws": "w1", "from": "a.txt", "to": "b.txt"}),
        );
        assert!(text(&v).contains("rename a.txt → b.txt"));
        let v = call(&c, "ws_delete", &json!({"ws": "w1", "path": "a.txt"}));
        assert!(text(&v).contains("delete a.txt"));
        assert!(is_err(&call(&c, "ws_rename", &json!({"ws": "w1"}))));
        assert!(is_err(&call(&c, "ws_delete", &json!({"ws": "w1"}))));
    }

    #[test]
    fn claim_conflict_and_release() {
        let wal = FakeWal::new();
        let wsi = FakeWs::new();
        let c = ctx(&wal, &wsi, &ok_snap);

        // Another ws holds the glob → conflict.
        wal.append(
            Path::new("/repo"),
            "w2",
            &Event::new(
                "w2",
                "agent-b",
                Op::Claim {
                    glob: "src/**".into(),
                    ttl_secs: 3600,
                },
            ),
        )
        .unwrap();
        let v = call(&c, "claim", &json!({"ws": "w1", "glob": "src/x.rs"}));
        assert!(is_err(&v));
        assert!(text(&v).contains("conflict"));

        // Disjoint glob → claimed, event appended.
        let v = call(
            &c,
            "claim",
            &json!({"ws": "w1", "glob": "docs/**", "ttl_secs": 60}),
        );
        assert!(text(&v).contains("claimed docs/**"));
        let events = wal.events(Path::new("/repo"), "w1").unwrap();
        assert!(matches!(
            events.last().unwrap().op,
            Op::Claim { ttl_secs: 60, .. }
        ));

        let v = call(&c, "release", &json!({"ws": "w1", "glob": "docs/**"}));
        assert!(text(&v).contains("released docs/**"));
        assert!(is_err(&call(&c, "claim", &json!({"ws": "w1"}))));
    }

    #[test]
    fn snap_intent_review() {
        let wal = FakeWal::new();
        let wsi = FakeWs::new();
        let c = ctx(&wal, &wsi, &ok_snap);

        let v = call(&c, "snap", &json!({"ws": "w1", "message": "ship"}));
        assert!(text(&v).contains("snap cafe123 → main"));
        let v = call(
            &c,
            "snap",
            &json!({"ws": "w1", "message": "m", "branch": "dev"}),
        );
        assert!(text(&v).contains("→ dev"));
        assert!(is_err(&call(&c, "snap", &json!({"ws": "w1"}))));

        let v = call(&c, "intent", &json!({"ws": "w1", "text": "why"}));
        assert!(text(&v).starts_with("intent e"));
        assert!(is_err(&call(&c, "intent", &json!({"ws": "w1"}))));

        let v = call(
            &c,
            "review",
            &json!({"ws": "w1", "event": "e1", "verdict": "approve"}),
        );
        assert!(text(&v).contains("approve e1"));
        let v = call(
            &c,
            "review",
            &json!({"ws": "w1", "event": "e1", "verdict": "nope"}),
        );
        assert!(is_err(&v));
    }

    #[test]
    fn subs_filters_topics() {
        let wal = FakeWal::new();
        let wsi = FakeWs::new();
        let c = ctx(&wal, &wsi, &ok_snap);
        wal.append(
            Path::new("/repo"),
            "w1",
            &Event::new("w1", "a", Op::Intent { text: "t".into() }),
        )
        .unwrap();
        wal.append(
            Path::new("/repo"),
            "w1",
            &Event::new(
                "w1",
                "a",
                Op::Claim {
                    glob: "x".into(),
                    ttl_secs: 10,
                },
            ),
        )
        .unwrap();
        wal.append(
            Path::new("/repo"),
            "w1",
            &Event::new(
                "w1",
                "a",
                Op::Review {
                    event: "e1".into(),
                    verdict: Verdict::Approve,
                },
            ),
        )
        .unwrap();
        wal.append(
            Path::new("/repo"),
            "w1",
            &Event::new(
                "w1",
                "a",
                Op::Presence {
                    status: "joined".into(),
                },
            ),
        )
        .unwrap();

        // ops = firehose.
        let v = call(&c, "subs", &json!({"ws": "w1"}));
        assert_eq!(text(&v).lines().count(), 4);
        // claims only.
        let v = call(&c, "subs", &json!({"topic": "claims"}));
        assert_eq!(text(&v).lines().count(), 1);
        assert!(text(&v).contains("\"claim\""));
        // review / presence / unknown.
        let v = call(&c, "subs", &json!({"topic": "review"}));
        assert!(text(&v).contains("\"review\""));
        let v = call(&c, "subs", &json!({"topic": "presence"}));
        assert!(text(&v).contains("\"presence\""));
        let v = call(&c, "subs", &json!({"topic": "bogus"}));
        assert_eq!(text(&v), "");
        // limit clips the tail.
        let v = call(&c, "subs", &json!({"limit": 2}));
        assert_eq!(text(&v).lines().count(), 2);
        // wait_ms=1 polls briefly then returns — deadline path.
        let v = call(&c, "subs", &json!({"wait_ms": 1}));
        assert!(!is_err(&v));
    }

    #[test]
    fn unknown_tool_is_error() {
        let wal = FakeWal::new();
        let wsi = FakeWs::new();
        let c = ctx(&wal, &wsi, &ok_snap);
        let v = call(&c, "bogus", &json!({}));
        assert!(is_err(&v));
        assert!(text(&v).contains("unknown tool"));
    }
}
