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
                Some(sha) => Ok(String::from_utf8_lossy(
                    &ctx.wsi.blob(ctx.repo, sha)?,
                )
                .into_owned()),
                None => Err(std::io::Error::other("path deleted in ws overlay")),
            };
        }
        Ok(String::from_utf8_lossy(
            &ctx.wsi.read(ctx.repo, &tip, path)?,
        )
        .into_owned())
    })();
    r.map_or_else(|e| tool_err(e), tool_text)
}

fn ws_write(ctx: &Ctx, a: &Value) -> Value {
    let (ws, path, content) =
        match (arg(a, "ws"), arg(a, "path"), arg(a, "content")) {
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
                    intent: a
                        .get("intent")
                        .and_then(|v| v.as_str())
                        .map(String::from),
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

fn claim(ctx: &Ctx, a: &Value) -> Value {
    let (ws, glob) = match (arg(a, "ws"), arg(a, "glob")) {
        (Ok(w), Ok(g)) => (w, g),
        _ => return tool_err("missing ws/glob"),
    };
    let r = (|| -> Result<String, String> {
        let mut all = Vec::new();
        for w in ctx
            .wal
            .workspaces(ctx.repo)
            .map_err(|e| e.to_string())?
        {
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
        let ttl = a
            .get("ttl_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);
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
            &Event::new(
                ws,
                ctx.actor,
                Op::Release {
                    glob: glob.into(),
                },
            ),
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
            &Event::new(
                ws,
                ctx.actor,
                Op::Intent {
                    text: text.into(),
                },
            ),
        )
        .map(|id| tool_text(format!("intent {id}")))
        .unwrap_or_else(|e| tool_err(e))
}

fn review(ctx: &Ctx, a: &Value) -> Value {
    let (ws, event, verdict) =
        match (arg(a, "ws"), arg(a, "event"), arg(a, "verdict")) {
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

/// Pull-replay feed: recent events across all workspaces, mapped to
/// the requested topic. MCP is request/response — live push lives
/// on `sturn sub`, this is the agent's catch-up view.
fn subs(ctx: &Ctx, a: &Value) -> Value {
    let topic_name = a
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or(topic::OPS);
    let limit = a.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
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
        tool_schema("claim", "Advisory lock on a path glob — conflicts before work", json!({"ws": ws.1, "glob": s("glob").1, "ttl_secs": {"type":"integer"}}), &["ws","glob"]),
        tool_schema("release", "Drop a claim", json!({"ws": ws.1, "glob": s("glob").1}), &["ws","glob"]),
        tool_schema("snap", "Fold ws ops into a real commit", json!({"ws": ws.1, "message": s("message").1, "branch": s("branch").1}), &["ws","message"]),
        tool_schema("intent", "Record why work exists", json!({"ws": ws.1, "text": s("text").1}), &["ws","text"]),
        tool_schema("review", "Approve/reject an event id", json!({"ws": ws.1, "event": s("event").1, "verdict": s("verdict").1}), &["ws","event","verdict"]),
        tool_schema("subs", "Recent events on a topic (ops|claims|review|presence)", json!({"topic": s("topic").1, "limit": {"type":"integer"}}), &[]),
    ]})
}

/// Serve newline-delimited JSON-RPC (MCP stdio transport) until
/// stdin closes. Requests and notifications interleave; only
/// requests get responses.
pub fn serve(ctx: &Ctx) -> std::io::Result<()> {
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
    Ok(())
}
