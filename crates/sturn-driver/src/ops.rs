//! Engine ops — `cli.<verb>` wire queries dispatch into the `sturn`
//! engine lib through this binary's own `run` verb: each call
//! re-execs `tamias-driver-sturn run <argv>` with the request's repo
//! as cwd, so `sturn::dispatch` writes to the child's captured
//! stdout and never touches the serve loop's ndjson stream — and
//! every mutation keeps the engine's own write-path semantics (WAL
//! append, actor from config/`$STURN_ACTOR`, claim fencing).
//!
//! `ops` answers the ops manifest as JSON — *derived from the
//! engine's own clap tree* ([`sturn::command`]), so a verb can't
//! exist on the CLI and be missing here: leaf subcommands flatten
//! into dotted names (`ws new` → `cli.ws.new`), declared args
//! become typed params (`arg`/`opt`/`flag`), and doc strings ride
//! along unchanged.
//!
//! Unbounded verbs (`daemon`, `sub`, `bridge`, `mcp`, `mount`) are
//! deliberately *not* ops — they never return, so they live in
//! `surfaces` instead: surface drivers (`tamias-driver-cli`/`-mcp`,
//! the pavo-driver-kit `surface` layer) stream them via `run`.
//! Streaming *flags* (`diff --live`, `check --watch`,
//! `service logs -f`) are likewise excluded — the bounded form of
//! those verbs stays a wire op.

use clap::{Arg, ArgAction, Command};
use pavo_protocol::wire::{QueryResult, Response};
use serde_json::{json, Map, Value};
use std::path::Path;

/// Unbounded surfaces — verbs that never return, so they can
/// never be request/response ops.
const SURFACES: &[&str] = &["daemon", "sub", "bridge", "mcp", "mount"];

/// Params excluded from ops — flags that turn a bounded verb into
/// a stream. The bounded form rides the wire; the stream rides
/// `run` passthrough.
const SKIP_PARAMS: &[&str] = &["live", "watch", "follow"];

/// Read-only verbs — everything else appends WAL ops, writes
/// files, pushes refs, or touches system state. Mutators belong
/// behind a deliberate trigger, not ambient view polling.
const READ_ONLY: &[&str] = &[
    "log", "diff", "claims", "check", "asks", "wait", "verify", "ws.ls", "ws.read",
];

/// `(dotted name, clap spec)` for every leaf verb — groups (`ws`,
/// `deploy`, `destroy`, `service`) aren't invocable themselves.
fn verbs() -> Vec<(String, Command)> {
    fn walk(prefix: &str, cmd: &Command, out: &mut Vec<(String, Command)>) {
        for sub in cmd.get_subcommands() {
            let name = sub.get_name();
            if name == "help" {
                continue;
            }
            let dotted = match prefix {
                "" => name.to_string(),
                _ => format!("{prefix}.{name}"),
            };
            if sub.has_subcommands() {
                walk(&dotted, sub, out);
            } else {
                out.push((dotted, sub.clone()));
            }
        }
    }
    let mut out = Vec::new();
    walk("", &sturn::command(), &mut out);
    out
}

fn op_cmd(name: &str) -> Option<Command> {
    verbs()
        .into_iter()
        .find(|(n, _)| n == name && !SURFACES.contains(&n.as_str()))
        .map(|(_, c)| c)
}

/// Args that can't ride the wire: streaming flags (above) plus
/// clap's own help/version actions.
fn skipped(a: &Arg) -> bool {
    SKIP_PARAMS.contains(&a.get_id().as_str())
        || matches!(
            a.get_action(),
            ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong | ArgAction::Version
        )
}

/// A clap arg → the manifest's typed param kind — `arg` =
/// positional, `opt` = `--name <v>`, `flag` = `--name` — the
/// contract `pavo-driver-kit`'s surface layer renders args from.
fn kind(a: &Arg) -> &'static str {
    if a.is_positional() {
        "arg"
    } else if matches!(
        a.get_action(),
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
    ) {
        "flag"
    } else {
        "opt"
    }
}

fn param_json(a: &Arg) -> Value {
    // Wire name = the flag name (`--option`), not the field id
    // (`options`) — generated surfaces spell params `--<name>`.
    let id = a.get_id().as_str();
    let name = match a.is_positional() {
        true => id,
        false => a.get_long().unwrap_or(id),
    };
    json!({
        "name": name,
        "kind": kind(a),
        "doc": a.get_help().map(|d| d.to_string()).unwrap_or_default(),
    })
}

/// The `ops` query/`ops` argv verb payload — self-describing so
/// hosts and agents discover the engine surface without argv help.
/// `dir`/`root`/`repo`/`config`/`actor` are the driver's globals —
/// accepted by every op, listed once on the manifest itself.
pub fn manifest() -> Value {
    json!({
        "engine": "sturn",
        "invoke": "query `cli.<verb>` with the listed params, or argv `tamias-driver-sturn run <sturn argv>`",
        "globals": [
            {"name": "dir|root|repo", "kind": "opt", "doc": "Target repo dir — same resolution the view queries use (param → $STURN_REPO → sturn.toml repo.dir → cwd)."},
            {"name": "config", "kind": "opt", "doc": "Config path — the driver's own chain resolves it and hands the child an absolute --config."},
            {"name": "actor", "kind": "env", "doc": "Claim/verb actor — sets $STURN_ACTOR in the child (config actor.name still wins, same as the CLI)."},
        ],
        "ops": verbs().iter()
            .filter(|(n, _)| !SURFACES.contains(&n.as_str()))
            .map(|(n, c)| json!({
                "query": format!("cli.{n}"),
                "doc": c.get_about().map(|d| d.to_string()).unwrap_or_default(),
                "params": c.get_arguments()
                    .filter(|a| !skipped(a))
                    .map(param_json)
                    .collect::<Vec<_>>(),
                "mutates": !READ_ONLY.contains(&n.as_str()),
            }))
            .collect::<Vec<_>>(),
        "surfaces": SURFACES,
    })
}

/// Is `query` an engine op (`cli.<verb>`)? Returns the verb name.
pub fn op_of(query: &str) -> Option<String> {
    query
        .strip_prefix("cli.")
        .filter(|n| op_cmd(n).is_some())
        .map(str::to_string)
}

/// A wire scalar → its argv token (numbers/bools stringify).
fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// All values of a param — arrays flatten, scalars wrap (repeatable
/// opts like `--option`, multi-positionals like `snap <ws>…`).
fn values(v: &Value) -> Vec<String> {
    match v {
        Value::Array(xs) => xs.iter().filter_map(scalar).collect(),
        other => scalar(other).into_iter().collect(),
    }
}

/// `cli.<verb>` params → sturn argv (no argv[0] — `run` prepends
/// it). Globals are resolved by [`run_verb`] (`--repo`/`--config`
/// prepend here); the dotted verb name splits into the group path
/// (`ws.new` → `ws new`). Declared params map by kind — positionals
/// land last, in schema order.
fn argv(
    dir: &Path,
    config: Option<&str>,
    name: &str,
    cmd: &Command,
    params: &Map<String, Value>,
) -> Vec<String> {
    let mut out: Vec<String> = vec!["--repo".into(), dir.display().to_string()];
    if let Some(c) = config {
        out.extend(["--config".into(), c.into()]);
    }
    out.extend(name.split('.').map(String::from));
    for a in cmd.get_arguments() {
        if skipped(a) || a.is_positional() {
            continue;
        }
        let id = a.get_id().as_str();
        let long = a.get_long().unwrap_or(id);
        let raw = params
            .get(long)
            .or_else(|| params.get(id))
            .or_else(|| params.get(id.replace('_', "-").as_str()));
        let Some(raw) = raw else { continue };
        match kind(a) {
            "flag" => {
                if raw.as_bool().unwrap_or(false) {
                    out.push(format!("--{long}"));
                }
            }
            _ => {
                for s in values(raw) {
                    out.push(format!("--{long}"));
                    out.push(s);
                }
            }
        }
    }
    let mut positionals: Vec<String> = Vec::new();
    for a in cmd.get_positionals() {
        if skipped(a) {
            continue;
        }
        let id = a.get_id().as_str();
        if let Some(raw) = params
            .get(id)
            .or_else(|| params.get(id.replace('_', "-").as_str()))
        {
            positionals.extend(values(raw));
        }
    }
    out.extend(positionals);
    out
}

/// The bare-name entry the embedded surfaces use (`OpSpec.name` is
/// `cli.`-stripped); the wire path goes `op_of` → `run_verb`.
#[cfg(any(feature = "cli", feature = "mcp"))]
pub fn run_op(op: &str, params: &Map<String, Value>) -> Response {
    match op_of(&format!("cli.{}", op.strip_prefix("cli.").unwrap_or(op))) {
        Some(name) => run_verb(&name, params),
        None => Response::err(format!("unknown op: {op}")),
    }
}

/// Run one engine op by name: re-exec `run` in a child whose cwd is
/// the request's repo (same resolution the views use), and hand
/// back `{op, argv, code, stdout, stderr}` as the query payload.
/// `ok:false` is reserved for spawn failures — a nonzero `code` is
/// the engine's own exit status (e.g. `check` contested), honest
/// data. `params.actor` sets `STURN_ACTOR` for the child — claims
/// stay actor-scoped, no bypass around the engine's own rules.
pub fn run_verb(name: &str, params: &Map<String, Value>) -> Response {
    let Some(cmd) = op_cmd(name) else {
        return Response::err(format!("unknown op: {name}"));
    };
    let (dir, config) = crate::data::resolve(params);
    let args = argv(&dir, config.as_deref(), name, &cmd, params);
    let out = std::env::current_exe().and_then(|exe| {
        let mut c = std::process::Command::new(exe);
        c.arg("run").args(&args).current_dir(&dir);
        if let Some(actor) = params.get("actor").and_then(Value::as_str) {
            c.env("STURN_ACTOR", actor);
        }
        c.output()
    });
    let data = match out {
        Ok(o) => json!({
            "op": name,
            "argv": args,
            "code": o.status.code().unwrap_or(-1),
            "stdout": String::from_utf8_lossy(&o.stdout),
            "stderr": String::from_utf8_lossy(&o.stderr),
        }),
        Err(e) => return Response::err(format!("run {name}: {e}")),
    };
    Response::ok_typed(&QueryResult {
        query: format!("cli.{name}"),
        data,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::path::PathBuf;

    use super::*;
    use serde_json::json;

    #[test]
    fn op_of_accepts_cli_prefix_only() {
        assert_eq!(op_of("cli.claims").unwrap(), "claims");
        assert_eq!(op_of("cli.ws.new").unwrap(), "ws.new");
        assert_eq!(op_of("cli.service.logs").unwrap(), "service.logs");
        assert!(op_of("cli.daemon").is_none()); // surface, not an op
        assert!(op_of("cli.mcp").is_none());
        assert!(op_of("claims").is_none()); // view query, not an op
        assert!(op_of("cli.bogus").is_none());
        assert!(op_of("cli.ws").is_none()); // group, not a leaf
    }

    #[test]
    fn manifest_lists_every_leaf_as_cli_query() {
        let m = manifest();
        let ops = m["ops"].as_array().unwrap();
        for (n, _) in verbs()
            .iter()
            .filter(|(n, _)| !SURFACES.contains(&n.as_str()))
        {
            assert!(
                ops.iter().any(|o| o["query"] == json!(format!("cli.{n}"))),
                "{n} missing from ops manifest"
            );
        }
        assert_eq!(m["surfaces"], json!(SURFACES));
        for o in ops {
            for p in o["params"].as_array().unwrap() {
                assert!(["arg", "opt", "flag"].contains(&p["kind"].as_str().unwrap()));
                // streaming flags never ride the wire
                assert!(!SKIP_PARAMS.contains(&p["name"].as_str().unwrap()));
            }
        }
    }

    #[test]
    fn argv_maps_params_to_sturn_flags() {
        let dir = PathBuf::from("/repo");
        let cmd = op_cmd("claim").unwrap();
        let mut p = Map::new();
        p.insert("uri".into(), json!("fs:src/**"));
        p.insert("ws".into(), json!("t"));
        p.insert("ttl".into(), json!(600));
        p.insert("force".into(), json!(true));
        assert_eq!(
            argv(&dir, None, "claim", &cmd, &p),
            vec![
                "--repo",
                "/repo",
                "claim",
                "--ws",
                "t",
                "--ttl",
                "600",
                "--force",
                "fs:src/**",
            ]
        );
    }

    #[test]
    fn argv_splits_dotted_names_and_repeatables() {
        let dir = PathBuf::from("/repo");
        let cmd = op_cmd("ask").unwrap();
        let mut p = Map::new();
        p.insert("subject".into(), json!("ship it?"));
        p.insert("ws".into(), json!("t"));
        p.insert("option".into(), json!(["yes", "no"]));
        assert_eq!(
            argv(&dir, Some("/cfg/sturn.toml"), "ask", &cmd, &p),
            vec![
                "--repo",
                "/repo",
                "--config",
                "/cfg/sturn.toml",
                "ask",
                "--ws",
                "t",
                "--option",
                "yes",
                "--option",
                "no",
                "ship it?",
            ]
        );
        let cmd = op_cmd("ws.new").unwrap();
        let mut p = Map::new();
        p.insert("name".into(), json!("w1"));
        p.insert("base".into(), json!("HEAD~1"));
        assert_eq!(
            argv(&dir, None, "ws.new", &cmd, &p),
            vec!["--repo", "/repo", "ws", "new", "--base", "HEAD~1", "w1"]
        );
    }
}
