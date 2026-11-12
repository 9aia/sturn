//! tamias-driver-sturn — the pavo view driver for a repo's sturn
//! coordination state.
//!
//! Verbs (the tamias driver contract):
//!
//! - `manifest` — print the [`DriverManifest`] JSON (discovery
//!   probe; never touches repo state).
//! - `capabilities` — `{"capabilities": ["pavo"], "verbs": […]}`.
//! - `ops` — the engine ops manifest: every `sturn` CLI verb as a
//!   `cli.<verb>` wire query, derived from the engine's own clap
//!   tree ([`sturn::command`]).
//! - `run <argv>` — engine passthrough: `tamias-driver-sturn run
//!   claims` ≡ `sturn claims`. Also the child side of `cli.*` wire
//!   queries (`ops::run_verb` re-execs this same verb so engine
//!   output never touches the serve stream).
//! - `serve` — ndjson JSON-RPC on stdio: `{"op":"manifest"}`,
//!   `{"op":"views"}`, `{"op":"view","view":…,"params":{…}}`,
//!   `{"op":"query","query":…,"params":{…}}` →
//!   `{"ok":true,"result":…}` / `{"ok":false,"error":…}`.
//!   `query:"ops"` answers the ops manifest; `query:"cli.<verb>"`
//!   runs the engine op.
//!
//! - `cli <sturn argv>` — the tamias `bins[]` shim verb (T-012):
//!   `tamias bin exec` runs `tamias-driver-sturn cli --dir <pkg>
//!   --name sturn <argv>` for a declared `shim: cli` bin; the two
//!   driver flags are stripped and the rest goes to
//!   `sturn::run_from` with inherited stdio — full engine parity
//!   (`--repo`/`--config` globals before the verb, `--help`,
//!   stdin-bound verbs, the unbounded `daemon`/`sub`/`bridge`/
//!   `mount` surfaces).
//!
//! Optional surface: feature `mcp` embeds pavo-driver-kit's
//! surface layer — `mcp` serves the ops as MCP tools on stdio,
//! equivalent to the standalone `tamias-driver-mcp` surface
//! driver, minus the PATH/discovery hop.
//!
//! Which repo: each `view`/`query` resolves its own target —
//! `params.dir`/`params.root`/`params.repo` → `$STURN_REPO` →
//! `sturn.toml`'s `repo.dir` (same config search as the `sturn`
//! CLI) → cwd. See `data.rs` for the exact order. Engine ops are
//! the exception by design: `cli.<verb>` re-execs `run` in the
//! request's repo and lets `sturn::dispatch` own the outcome —
//! writes keep the engine's own semantics (WAL append, actor from
//! config/`$STURN_ACTOR`, claim fencing). `manifest`/`views` are
//! repo-free so pavo can enumerate drivers anywhere.
//!
//! The view side is **read-only**: it folds `refs/sturn/wal/*`
//! exactly like `sturn claims`/`sturn log` do — the WAL is the
//! state, the tree is a projection, and views only ever read.

mod data;
mod ops;
#[cfg(feature = "mcp")]
mod surface;
mod views;

use pavo_driver_kit as kit;
use pavo_protocol::wire::{self, Request, Response, ViewSummary};

fn capabilities_json() -> serde_json::Value {
    let m = views::manifest();
    serde_json::json!({
        "capabilities": m.capabilities,
        "verbs": m.verbs,
        "views": m.views.len(),
    })
}

/// Dispatch one parsed request to its response.
fn dispatch(req: &Request) -> Response {
    match req {
        Request::Manifest => Response::ok_typed(&views::manifest()),
        Request::Views => {
            let summaries: Vec<ViewSummary> =
                views::specs().iter().map(ViewSummary::from).collect();
            Response::ok_typed(&summaries)
        }
        Request::View { view, params } => match views::view_result(view, params) {
            Ok(result) => Response::ok_typed(&result),
            Err(e) => Response::err(e),
        },
        Request::Query { query, params } => {
            // `ops` and `cli.<verb>` are engine ops; everything
            // else is a view query — unknown names fail the op,
            // degraded snapshots travel as `{"error": …}` data
            // (the marker renderers understand).
            if query == "ops" {
                Response::ok_typed(&wire::QueryResult {
                    query: query.clone(),
                    data: ops::manifest(),
                })
            } else if let Some(v) = ops::op_of(query) {
                ops::run_verb(&v, params)
            } else if views::known_query(query) {
                let snap = data::load(params);
                let payload = views::query_data(query, params, &snap);
                Response::ok_typed(&wire::QueryResult {
                    query: query.clone(),
                    data: payload,
                })
            } else {
                Response::err(format!("unknown query: {query}"))
            }
        }
        // `render` — this driver ships no renderer plugins; hosts
        // only call it for kinds the manifest claims, so answering
        // an explicit error keeps the wire honest.
        Request::Render { kind, .. } => Response::err(format!(
            "tamias-driver-sturn claims no renderers (kind '{kind}')"
        )),
        // The protocol grew `submit`/`jobs`/`notifications` ops after
        // this driver was written; mutations go through `cli.<verb>`,
        // never `submit`, so they answer `ok:false` like any
        // unsupported op.
        Request::Submit { .. } | Request::Jobs | Request::Notifications => {
            Response::err("tamias-driver-sturn implements no submit/jobs/notifications ops")
        }
    }
}

fn serve() -> i32 {
    // The kit owns the ndjson loop — stdin EOF and a dead client
    // both end it quietly, and either way the driver exits 0.
    let _ = kit::serve_stdio(dispatch);
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("manifest") => {
            println!("{}", kit::manifest_json(&views::manifest()));
            0
        }
        Some("capabilities") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&capabilities_json()).unwrap_or_default()
            );
            0
        }
        Some("ops") => {
            println!("{}", ops::manifest());
            0
        }
        // Engine passthrough — `tamias-driver-sturn run claims` is
        // `sturn claims`. Also the child side of `cli.*` wire
        // queries (ops::run_verb re-execs this same verb).
        Some("run") => {
            let argv = std::iter::once("sturn".to_string()).chain(args.iter().skip(1).cloned());
            std::process::exit(sturn::run_from(argv));
        }
        // The tamias `bins[]` shim verb (T-012): `tamias bin exec`
        // runs `<driver> cli --dir <pkg> --name <pkg> <sturn argv>`
        // for a declared `shim: cli` bin. sturn's engine IS the
        // CLI, so this is a passthrough — strip the two standard
        // driver flags and hand the rest to `sturn::run_from` with
        // inherited stdio. That keeps full engine parity a
        // generated ops→CLI surface can't express: globals before
        // the verb (`--repo`/`--config`), `--help`, stdin-bound
        // verbs, and the unbounded `daemon`/`sub`/`bridge`/`mount`
        // surfaces all behave exactly like the `sturn` binary.
        Some("cli") => {
            let argv = engine_argv(&args[1..]);
            std::process::exit(sturn::run_from(argv));
        }
        // Optional embedded surface (feature-gated): the
        // pavo-driver-kit surface layer serves this driver's own
        // ops manifest as MCP tools over stdio — the same ops the
        // `cli.*` wire queries expose.
        #[cfg(feature = "mcp")]
        Some("mcp") => {
            let m = kit::surface::OpsManifest::from_value(&ops::manifest()).unwrap_or_default();
            match kit::surface::mcp::serve_stdio(&m, &mut surface::Engine) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("tamias-driver-sturn mcp: {e}");
                    1
                }
            }
        }
        Some("serve") | None => serve(),
        _ => {
            eprintln!(
                "usage: tamias-driver-sturn manifest|capabilities|ops|run <argv>|cli <sturn argv>|serve\n       (feature `mcp` adds `mcp`)"
            );
            1
        }
    };
    std::process::exit(code);
}

/// `cli` verb argv (after the `cli` token) → engine argv (argv[0]
/// included). `tamias bin exec`/`driver exec` prepend the standard
/// driver flags — `--dir <payload>` `--name <pkg>` — which the
/// engine doesn't know; consume the leading run of those two
/// (`--flag v` and `--flag=v` forms) and pass everything else
/// through untouched. `--repo`/`--config` are deliberately NOT
/// stripped: they're sturn globals, so a user-passed one must
/// reach the engine's own parse.
fn engine_argv(args: &[String]) -> Vec<String> {
    let mut out = vec!["sturn".to_string()];
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--dir" || a == "--name" {
            // flag + separate value — skip both (a trailing flag
            // with no value just ends the scan).
            i = (i + 2).min(args.len());
        } else if a.starts_with("--dir=") || a.starts_with("--name=") {
            i += 1;
        } else {
            break;
        }
    }
    out.extend(args[i..].iter().cloned());
    out
}
