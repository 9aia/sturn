//! views — the driver's manifest: every ViewSpec plus the query
//! resolvers that feed them.
//!
//! Read path: all data comes from [`crate::data::Snapshot`] — a
//! single fold of the resolved repo's `refs/sturn/wal/*` logs per
//! wire op, through `sturn-wal-git`/`sturn-claim`/`sturn-ask`
//! exactly like the `sturn` CLI verbs. Nothing here proxies the
//! `sturn` binary or `sturn serve`-equivalent surface: the reads
//! are in-process ODB/ref walks, and keeping the driver a pure
//! reader means it can never mutate coordination state.
//!
//! Verb → view mapping:
//!
//! | `sturn` surface                  | pavo view              |
//! |--------------------------------|-------------------------|
//! | `sturn claims`                  | `sturn:claims` table    |
//! | `sturn log` / `sturn diff`      | `sturn:ops` table       |
//! | `sturn asks [--all]`            | `sturn:asks` table      |
//! | `sturn ws ls` + WAL depth       | `sturn:workspaces` table|
//! | `sturn sub <topic>` vocabulary  | `sturn:topics` table    |
//! | repo/WAL/claim digest           | `sturn:status` kv       |

use pavo_protocol::{
    ColumnNode, Contributes, Contribution, DriverManifest, KvNode, MarkdownNode, NodeBase,
    QueryBinding, TableColumn, TableNode, ViewNode, ViewSpec,
};
use serde_json::{json, Map, Value};
use sturn_ask::Status as AskStatus;
use sturn_claim::Active;
use sturn_core::Op;

use crate::data::Snapshot;

// ---------------------------------------------------------------- helpers

fn titled(title: &str) -> NodeBase {
    NodeBase {
        title: Some(title.to_string()),
        ..NodeBase::default()
    }
}

fn bound(title: &str, query: &str) -> NodeBase {
    NodeBase {
        title: Some(title.to_string()),
        query: Some(query.to_string()),
        ..NodeBase::default()
    }
}

fn col(title: &str, children: Vec<ViewNode>) -> ViewNode {
    ViewNode::Column(ColumnNode {
        base: titled(title),
        children,
        gap: None,
    })
}

fn kv(title: &str, query: &str) -> ViewNode {
    ViewNode::Kv(KvNode {
        base: bound(title, query),
        pairs: Vec::new(),
    })
}

fn table(title: &str, query: &str, columns: &[(&str, &str)]) -> ViewNode {
    ViewNode::Table(TableNode {
        base: bound(title, query),
        columns: columns
            .iter()
            .map(|(key, t)| TableColumn {
                key: (*key).to_string(),
                title: (*t).to_string(),
                align: None,
                width: None,
            })
            .collect(),
        rows: Vec::new(),
        sortable: true,
    })
}

fn md(title: &str, text: &str) -> ViewNode {
    ViewNode::Markdown(MarkdownNode {
        base: titled(title),
        text: text.to_string(),
    })
}

fn binding(name: &str) -> QueryBinding {
    QueryBinding {
        name: name.to_string(),
        op: "query".to_string(),
        params: Map::new(),
    }
}

fn spec(
    id: &str,
    title: &str,
    route: &str,
    description: &str,
    queries: &[&str],
    root: ViewNode,
) -> ViewSpec {
    ViewSpec {
        id: id.to_string(),
        title: title.to_string(),
        route: route.to_string(),
        description: Some(description.to_string()),
        queries: queries.iter().map(|q| binding(q)).collect(),
        root,
    }
}

// ---------------------------------------------------------------- manifest

/// Every view the driver declares — static (no repo state needed;
/// `manifest` must answer anywhere).
pub fn specs() -> Vec<ViewSpec> {
    vec![
        spec(
            "status",
            "Status",
            "/sturn/status",
            "Repo digest — workspaces, WAL depth, live claims, open asks, last op (`sturn claims`/`sturn log` at a glance).",
            &["summary"],
            col("Sturn status", vec![kv("Repo", "summary")]),
        ),
        spec(
            "claims",
            "Claims",
            "/sturn/claims",
            "Live advisory claims across every workspace — the coordination picture. Filter with --param ws= / actor= / uri=.",
            &["claims"],
            table(
                "Claims",
                "claims",
                &[
                    ("uri", "URI"),
                    ("ws", "WS"),
                    ("actor", "Actor"),
                    ("expires", "Expires"),
                    ("ttl", "TTL left"),
                    ("fence", "Fence"),
                ],
            ),
        ),
        spec(
            "ops",
            "Ops",
            "/sturn/ops",
            "Recent WAL ops across every workspace, newest first — `sturn log` as a table. Filter with --param ws= / actor= / op= / limit=.",
            &["ops"],
            table(
                "Ops",
                "ops",
                &[
                    ("time", "Time"),
                    ("ws", "WS"),
                    ("actor", "Actor"),
                    ("op", "Op"),
                    ("summary", "Summary"),
                    ("id", "ID"),
                ],
            ),
        ),
        spec(
            "asks",
            "Asks",
            "/sturn/asks",
            "Pending-response asks folded from the WAL — open by default, --param all=1 includes resolved. (`sturn asks`)",
            &["asks"],
            table(
                "Asks",
                "asks",
                &[
                    ("id", "ID"),
                    ("subject", "Subject"),
                    ("ws", "WS"),
                    ("actor", "Actor"),
                    ("target", "Target"),
                    ("status", "Status"),
                    ("asked", "Asked"),
                    ("expires", "Expires"),
                ],
            ),
        ),
        spec(
            "workspaces",
            "Workspaces",
            "/sturn/workspaces",
            "Workspace namespaces + WAL logs with event counts and in-flight (un-snapped) ops — `sturn ws ls` with depth.",
            &["workspaces"],
            table(
                "Workspaces",
                "workspaces",
                &[
                    ("name", "Name"),
                    ("kind", "Kind"),
                    ("events", "Events"),
                    ("pending", "Pending"),
                    ("tip", "WAL tip"),
                ],
            ),
        ),
        spec(
            "topics",
            "Topics",
            "/sturn/topics",
            "The bus topic vocabulary — repo.<id>.{ops,claims,review,presence,asks}. Derived names, not stored state (`sturn sub`).",
            &["topics"],
            col(
                "Sturn topics",
                vec![
                    kv("Repo", "summary"),
                    table(
                        "Topics",
                        "topics",
                        &[("topic", "Topic"), ("suffix", "Suffix"), ("description", "Carries")],
                    ),
                    md(
                        "Note",
                        "Topics are fan-out names the `sturn daemon` publishes to the local bus — a cache over the WAL, never the authority. State syncs through `refs/sturn/*` push/fetch.",
                    ),
                ],
            ),
        ),
    ]
}

/// The `manifest` op result.
pub fn manifest() -> DriverManifest {
    DriverManifest {
        name: "sturn".to_string(),
        title: "Sturn".to_string(),
        icon: Some("🐦".to_string()),
        version: env!("CARGO_PKG_VERSION").to_string(),
        verbs: vec!["manifest".to_string(), "serve".to_string()],
        capabilities: vec!["pavo".to_string()],
        contributes: Contributes {
            navbar: vec![Contribution {
                id: "claims".to_string(),
                title: "Sturn".to_string(),
                route: "/sturn/claims".to_string(),
                priority: 25,
                icon: None,
            }],
            dashboard: vec![Contribution {
                id: "status".to_string(),
                title: "Sturn status".to_string(),
                route: "/sturn/status".to_string(),
                priority: 35,
                icon: None,
            }],
            statusbar: vec![Contribution {
                id: "claims".to_string(),
                title: "Sturn claims".to_string(),
                route: "/sturn/claims".to_string(),
                priority: 45,
                icon: None,
            }],
            routes: Vec::new(),
            extra: Default::default(),
        },
        views: specs(),
        renderers: Vec::new(),
    }
}

// ---------------------------------------------------------------- queries

/// Every query name the driver answers — the `query` op rejects
/// anything outside this set with `ok:false`.
pub fn known_query(name: &str) -> bool {
    matches!(
        name,
        "summary" | "claims" | "ops" | "asks" | "workspaces" | "topics"
    )
}

/// `{"op":"query","query":name,"params":{…}}` → the data payload.
/// Unknown names return an `{"error": …}` object (defensive — the
/// wire layer rejects them before this runs).
pub fn query_data(name: &str, params: &Map<String, Value>, snap: &Snapshot) -> Value {
    if !snap.ok() {
        return snap.error_payload();
    }
    match name {
        "summary" => summary(snap),
        "claims" => claims_rows(snap, params),
        "ops" => ops_rows(snap, params),
        "asks" => asks_rows(snap, params),
        "workspaces" => workspaces_rows(snap),
        "topics" => Value::Array(crate::data::topic_rows(&snap.repo_id)),
        other => json!({"error": format!("unknown query '{other}'")}),
    }
}

// ---------------------------------------------------------------- shared

/// Unix seconds → `YYYY-MM-DD HH:MM:SS` UTC — no chrono dep.
fn fmt_ts(ts: u64) -> String {
    let days = (ts / 86_400) as i64;
    let secs = ts % 86_400;
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    // civil-from-days (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    if mo <= 2 {
        y += 1;
    }
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

/// Optional string param, lowercased for case-insensitive match.
fn param(params: &Map<String, Value>, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

fn short(id: &str) -> String {
    if id.len() > 10 {
        id[..10].to_string()
    } else {
        id.to_string()
    }
}

/// `op` kind name — same spellings the WAL JSON uses (`snake_case`
/// serde tag), kept as a match so a new `Op` variant fails to
/// compile here rather than silently stringifying.
fn op_kind(op: &Op) -> &'static str {
    match op {
        Op::Write { .. } => "write",
        Op::Rename { .. } => "rename",
        Op::Delete { .. } => "delete",
        Op::Claim { .. } => "claim",
        Op::Release { .. } => "release",
        Op::Intent { .. } => "intent",
        Op::Snapshot { .. } => "snapshot",
        Op::Review { .. } => "review",
        Op::Presence { .. } => "presence",
        Op::Ask { .. } => "ask",
        Op::Answer { .. } => "answer",
        Op::Cancel { .. } => "cancel",
    }
}

/// One-line summary of an op — what `sturn diff`'s `op_summary`
/// renders, minus the blob resolution (read-only metadata).
fn op_summary(op: &Op) -> String {
    match op {
        Op::Write { path, intent, .. } => match intent {
            Some(i) => format!("{path} — {i}"),
            None => path.clone(),
        },
        Op::Rename { from, to } => format!("{from} → {to}"),
        Op::Delete { path } => format!("delete {path}"),
        Op::Claim { glob, ttl_secs } => format!("{glob} (ttl {ttl_secs}s)"),
        Op::Release { glob } => format!("release {glob}"),
        Op::Intent { text } => {
            let t: String = text.chars().take(80).collect();
            if text.chars().count() > 80 {
                format!("{t}…")
            } else {
                t
            }
        }
        Op::Snapshot { message } => format!("snap: {message}"),
        Op::Review { event, verdict } => {
            let v = match verdict {
                sturn_core::Verdict::Approve => "approve",
                sturn_core::Verdict::Reject => "reject",
            };
            format!("{v} {}", short(event))
        }
        Op::Presence { status } => status.clone(),
        Op::Ask {
            subject, target, ..
        } => format!("{subject} → {target}"),
        Op::Answer { ask, verdict } => format!("{verdict} → ask {}", short(ask)),
        Op::Cancel { ask } => format!("cancel ask {}", short(ask)),
    }
}

// ---------------------------------------------------------------- queries

/// `summary` query — the kv object: repo identity + counts + last op.
fn summary(snap: &Snapshot) -> Value {
    let wal_ws = snap
        .workspaces
        .iter()
        .filter(|w| w.kinds.contains(&"wal"))
        .count();
    let ns_ws = snap
        .workspaces
        .iter()
        .filter(|w| w.kinds.contains(&"namespace"))
        .count();
    let open_asks = snap
        .asks
        .iter()
        .filter(|(_, s)| matches!(s, AskStatus::Open))
        .count();
    let joined = snap
        .presence
        .values()
        .filter(|s| s.as_str() == "joined")
        .count();
    let last = snap.events.iter().max_by_key(|e| e.ts);
    let mut out = Map::new();
    out.insert("repo".into(), json!(snap.repo_id));
    out.insert(
        "dir".into(),
        json!(snap
            .repo
            .canonicalize()
            .unwrap_or_else(|_| snap.repo.clone())
            .display()
            .to_string()),
    );
    out.insert("config".into(), json!(snap.config));
    out.insert("wal_prefix".into(), json!(snap.wal_prefix));
    out.insert("workspaces".into(), json!(snap.workspaces.len()));
    out.insert("workspace_namespaces".into(), json!(ns_ws));
    out.insert("wal_logs".into(), json!(wal_ws));
    out.insert("events".into(), json!(snap.events.len()));
    out.insert("claims".into(), json!(snap.claims.len()));
    out.insert("asks_open".into(), json!(open_asks));
    out.insert("asks_total".into(), json!(snap.asks.len()));
    out.insert("actors_joined".into(), json!(joined));
    out.insert(
        "last_op".into(),
        json!(last.map(|e| {
            format!(
                "{} {} {} — {} (ws {}, actor {})",
                fmt_ts(e.ts),
                op_kind(&e.op),
                short(&e.id),
                op_summary(&e.op),
                e.ws,
                e.actor
            )
        })),
    );
    out.insert("warnings".into(), json!(snap.warnings));
    Value::Object(out)
}

/// `claims` query — live claims as row objects (`sturn claims`).
/// Filters: `ws`, `actor` (exact), `uri` (substring).
fn claims_rows(snap: &Snapshot, params: &Map<String, Value>) -> Value {
    let ws = param(params, "ws");
    let actor = param(params, "actor");
    let uri = param(params, "uri").map(|u| u.to_lowercase());
    let rows: Vec<Value> = snap
        .claims
        .iter()
        .filter(|c| ws.as_deref().is_none_or(|w| c.ws == w))
        .filter(|c| actor.as_deref().is_none_or(|a| c.actor == a))
        .filter(|c| {
            uri.as_deref()
                .is_none_or(|u| c.uri.to_lowercase().contains(u))
        })
        .map(|c| claim_row(c, snap.now))
        .collect();
    Value::Array(rows)
}

fn claim_row(c: &Active, now: u64) -> Value {
    let ttl_left = c.expires.saturating_sub(now);
    json!({
        "uri": c.uri,
        "ws": c.ws,
        "actor": c.actor,
        "expires": fmt_ts(c.expires),
        "expires_ts": c.expires,
        "ttl": ttl_left,
        "fence": c.fence,
    })
}

/// `ops` query — every WAL event as a row, newest first
/// (`sturn log` flattened across workspaces).
/// Filters: `ws`, `actor` (exact), `op` (kind), `limit` (default
/// 100, 0 = all).
fn ops_rows(snap: &Snapshot, params: &Map<String, Value>) -> Value {
    let ws = param(params, "ws");
    let actor = param(params, "actor");
    let op = param(params, "op").map(|o| o.to_lowercase());
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;

    let mut evs: Vec<&sturn_core::Event> = snap
        .events
        .iter()
        .filter(|e| ws.as_deref().is_none_or(|w| e.ws == w))
        .filter(|e| actor.as_deref().is_none_or(|a| e.actor == a))
        .filter(|e| op.as_deref().is_none_or(|k| op_kind(&e.op) == k))
        .collect();
    // Newest first; per-ws WAL order breaks ts ties stably.
    evs.sort_by_key(|e| std::cmp::Reverse(e.ts));
    if limit > 0 {
        evs.truncate(limit);
    }
    Value::Array(
        evs.into_iter()
            .map(|e| {
                json!({
                    "id": short(&e.id),
                    "event": e.id,
                    "time": fmt_ts(e.ts),
                    "ts": e.ts,
                    "ws": e.ws,
                    "actor": e.actor,
                    "op": op_kind(&e.op),
                    "summary": op_summary(&e.op),
                })
            })
            .collect(),
    )
}

/// `asks` query — every ask + status (`sturn asks --all`), open
/// first is *not* guaranteed — order is WAL append order. Filter:
/// `all` (truthy shows resolved too), `ws`, `actor`.
fn asks_rows(snap: &Snapshot, params: &Map<String, Value>) -> Value {
    let all = params
        .get("all")
        .and_then(|v| {
            v.as_bool()
                .or_else(|| v.as_str().map(|s| matches!(s, "1" | "true" | "yes")))
        })
        .unwrap_or(false);
    let ws = param(params, "ws");
    let actor = param(params, "actor");
    let rows: Vec<Value> = snap
        .asks
        .iter()
        .filter(|(_, s)| all || matches!(s, AskStatus::Open))
        .filter(|(o, _)| ws.as_deref().is_none_or(|w| o.ws == w))
        .filter(|(o, _)| actor.as_deref().is_none_or(|a| o.actor == a))
        .map(|(o, s)| ask_row(o, s))
        .collect();
    Value::Array(rows)
}

fn ask_status_label(s: &AskStatus) -> String {
    match s {
        AskStatus::Open => "open".into(),
        AskStatus::Answered { verdict, by } => format!("answered {verdict} by {by}"),
        AskStatus::Cancelled => "cancelled".into(),
        AskStatus::Expired => "expired".into(),
    }
}

fn ask_row(o: &sturn_ask::Open, s: &AskStatus) -> Value {
    let mut row = json!({
        "id": short(&o.id),
        "ask": o.id,
        "subject": o.subject,
        "ws": o.ws,
        "actor": o.actor,
        "target": o.target,
        "status": ask_status_label(s),
        "asked": fmt_ts(o.asked),
        "expires": o.expires.map(fmt_ts),
    });
    if let AskStatus::Answered { verdict, by } = s {
        row["verdict"] = json!(verdict);
        row["answered_by"] = json!(by);
    }
    row
}

/// `workspaces` query — the ws union with WAL depth.
fn workspaces_rows(snap: &Snapshot) -> Value {
    Value::Array(
        snap.workspaces
            .iter()
            .map(|w| {
                json!({
                    "name": w.name,
                    "kind": w.kinds.join("+"),
                    "events": w.events,
                    "pending": w.pending,
                    "tip": w.tip,
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------- view op

/// `{"op":"view","view":id,"params":{…}}` → [`pavo_protocol::wire::ViewResult`].
pub fn view_result(
    view_id: &str,
    params: &Map<String, Value>,
) -> Result<pavo_protocol::wire::ViewResult, String> {
    let spec = specs()
        .into_iter()
        .find(|v| v.id == view_id)
        .ok_or_else(|| format!("unknown view: {view_id}"))?;

    let snap = crate::data::load(params);
    let mut data = Map::new();
    for b in &spec.queries {
        data.insert(b.name.clone(), query_data(&b.name, params, &snap));
    }

    Ok(pavo_protocol::wire::ViewResult { view: spec, data })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::Map;

    #[test]
    fn manifest_advertises_pavo_and_lists_every_view() {
        let m = manifest();
        assert!(m.verbs.iter().any(|v| v == "serve"));
        assert!(m.capabilities.iter().any(|c| c == "pavo"));
        let ids: std::collections::BTreeSet<&str> = m.views.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids.len(), m.views.len(), "duplicate view ids");
        for want in ["status", "claims", "ops", "asks", "workspaces", "topics"] {
            assert!(ids.contains(want), "missing view {want}");
        }
        assert!(m
            .contributes
            .navbar
            .iter()
            .any(|c| c.route == "/sturn/claims"));
        assert!(m
            .contributes
            .statusbar
            .iter()
            .any(|c| c.route == "/sturn/claims"));
        assert!(m
            .contributes
            .dashboard
            .iter()
            .any(|c| c.route == "/sturn/status"));
    }

    #[test]
    fn every_binding_is_a_known_query() {
        for v in specs() {
            for b in &v.queries {
                assert!(known_query(&b.name), "{}: unknown query {}", v.id, b.name);
            }
        }
    }

    #[test]
    fn degraded_snapshot_yields_error_payloads() {
        let snap = Snapshot {
            error: Some("not a git repo".into()),
            ..Snapshot::default()
        };
        let params = Map::new();
        for name in ["summary", "claims", "ops", "asks", "workspaces", "topics"] {
            let v = query_data(name, &params, &snap);
            assert_eq!(v["error"], "not a git repo", "{name}");
        }
    }

    #[test]
    fn fmt_ts_renders_unix_utc() {
        assert_eq!(fmt_ts(0), "1970-01-01 00:00:00");
        assert_eq!(fmt_ts(1_700_000_000), "2023-11-14 22:13:20");
    }

    #[test]
    fn topics_cover_the_bus_vocabulary() {
        let rows = crate::data::topic_rows("sturn");
        let names: Vec<&str> = rows.iter().filter_map(|r| r["topic"].as_str()).collect();
        for suffix in sturn_core::topic::ALL {
            assert!(
                names.iter().any(|t| t.ends_with(suffix)),
                "missing {suffix}"
            );
        }
    }

    #[test]
    fn op_kind_and_summary_cover_every_variant() {
        // Compile-time coverage: match arms above are exhaustive;
        // here just sanity-check the renderings.
        let ev = sturn_core::Event::new(
            "ws",
            "actor",
            Op::Claim {
                glob: "fs:src/**".into(),
                ttl_secs: 60,
            },
        );
        assert_eq!(op_kind(&ev.op), "claim");
        assert!(op_summary(&ev.op).contains("fs:src/**"));
    }
}
