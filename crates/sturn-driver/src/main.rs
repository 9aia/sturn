//! tamias-driver-sturn — the pavo view driver for a repo's sturn
//! coordination state.
//!
//! Verbs (the tamias driver contract):
//!
//! - `manifest` — print the [`DriverManifest`] JSON (discovery
//!   probe; never touches repo state).
//! - `capabilities` — `{"capabilities": ["pavo"], "verbs": […]}`.
//! - `serve` — ndjson JSON-RPC on stdio: `{"op":"manifest"}`,
//!   `{"op":"views"}`, `{"op":"view","view":…,"params":{…}}`,
//!   `{"op":"query","query":…,"params":{…}}` →
//!   `{"ok":true,"result":…}` / `{"ok":false,"error":…}`.
//!
//! Which repo: each `view`/`query` resolves its own target —
//! `params.dir`/`params.root`/`params.repo` → `$STURN_REPO` →
//! `sturn.toml`'s `repo.dir` (same config search as the `sturn`
//! CLI) → cwd. See `data.rs` for the exact order. `manifest`/`views`
//! are repo-free so pavo can enumerate drivers anywhere.
//!
//! The driver is **read-only**: it folds `refs/sturn/wal/*` exactly
//! like `sturn claims`/`sturn log` do — the WAL is the state, the
//! tree is a projection, and this driver only ever reads.

mod data;
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
            // Unknown names fail the op; everything else answers
            // `ok:true` — degraded snapshots travel as
            // `{"error": …}` data (the marker renderers understand),
            // matching the reference drivers' contract.
            if !views::known_query(query) {
                return Response::err(format!("unknown query: {query}"));
            }
            let snap = data::load(params);
            let payload = views::query_data(query, params, &snap);
            Response::ok_typed(&wire::QueryResult {
                query: query.clone(),
                data: payload,
            })
        }
        // `render` — this driver ships no renderer plugins; hosts
        // only call it for kinds the manifest claims, so answering
        // an explicit error keeps the wire honest.
        Request::Render { kind, .. } => Response::err(format!(
            "tamias-driver-sturn claims no renderers (kind '{kind}')"
        )),
        // The protocol grew `submit`/`jobs`/`notifications` ops after
        // this driver was written; sturn is read-only with no feeds,
        // so they answer `ok:false` like any unsupported op.
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
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("manifest") => {
            println!("{}", kit::manifest_json(&views::manifest()));
        }
        Some("capabilities") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&capabilities_json()).unwrap_or_default()
            );
        }
        Some("serve") => std::process::exit(serve()),
        _ => {
            eprintln!("usage: tamias-driver-sturn manifest|capabilities|serve");
            std::process::exit(1);
        }
    }
}
