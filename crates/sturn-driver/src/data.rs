//! data — one read of a repo's sturn state per wire op.
//!
//! The driver is a *reader*: every `view`/`query` resolves the
//! target repo, folds every `refs/sturn/wal/*` log through the same
//! pure functions the `sturn` CLI uses (`sturn_claim::active`,
//! `sturn_ask::all`), and drops the result. No mutations — `sturn
//! claim`/`write`/`snap` stay the write surface; this crate only
//! needs what `sturn claims`/`sturn log`/`sturn asks` already show.
//!
//! Repo resolution order (first hit wins) — mirrors `sturn --repo`
//! plus driver conveniences:
//!
//! 1. Request params — `dir`, `root`, or `repo` (all name the repo
//!    dir), e.g. `pavo view sturn:claims --param dir=/path/to/repo`.
//! 2. `$STURN_REPO` — driver-level convenience; the `sturn` CLI
//!    itself has no repo env var.
//! 3. `sturn.toml`'s `repo.dir` — config search identical to the
//!    CLI: `--config`-equivalent param `config` → `$STURN_CONFIG` →
//!    `./sturn.toml` → `~/.config/sturn/sturn.toml`. A relative
//!    `repo.dir` resolves against the process cwd, same as the CLI.
//! 4. `.` — the driver's cwd (pavo inherits the user's cwd, so
//!    `pavo view sturn:claims` inside a wired repo reads it).
//!
//! When nothing resolves to a git repo, [`Snapshot::error`] carries
//! the reason and every query degrades to an `{"error": …}` payload
//! — the wire stays `ok:true` so hosts render the failure inline.

use std::collections::BTreeMap;
use std::path::PathBuf;

use sturn_ask::Status as AskStatus;
use sturn_claim as claims;
use sturn_config::ConfigSource;
use sturn_core::{topic, Config, Event, Op};
use sturn_wal::Wal;
use sturn_wal_git::GitWal;
use sturn_ws::Ws;

/// One workspace row — union of `refs/sturn/ws/*` namespaces and
/// `refs/sturn/wal/*` logs (same union `sturn ws ls` prints).
#[derive(Debug, Clone)]
pub struct WsRow {
    /// Workspace name.
    pub name: String,
    /// `namespace` = `refs/sturn/ws/<name>` exists,
    /// `wal` = `refs/sturn/wal/<name>` exists; both when both do.
    pub kinds: Vec<&'static str>,
    /// WAL events in the log (0 for namespace-only ws).
    pub events: usize,
    /// Events since the last `Snapshot` marker — ops still in
    /// flight, not yet folded into a commit.
    pub pending: usize,
    /// WAL tip short sha, when the log exists.
    pub tip: Option<String>,
}

/// Everything the views need, loaded once per op.
pub struct Snapshot {
    /// Resolved repo dir (may not be a git repo).
    pub repo: PathBuf,
    /// Repo id — `repo.id` from config, else dir basename.
    pub repo_id: String,
    /// Where `sturn.toml` came from (`<path>`), None = flag/env only.
    pub config: Option<String>,
    /// WAL ref prefix in effect (default `refs/sturn/wal`).
    pub wal_prefix: String,
    /// Union of ws namespaces + WAL logs.
    pub workspaces: Vec<WsRow>,
    /// Every event across every WAL, fold order per ws.
    pub events: Vec<Event>,
    /// Live claims at `now` — `sturn claims` output, structured.
    pub claims: Vec<claims::Active>,
    /// Every ask and its terminal state — `sturn asks --all`.
    pub asks: Vec<(sturn_ask::Open, AskStatus)>,
    /// Last `Presence` status per `(ws, actor)` — `joined` actors
    /// are the live swarm; `left`/absent are gone.
    pub presence: BTreeMap<(String, String), String>,
    /// Unix seconds the snapshot was taken at.
    pub now: u64,
    /// Per-source load problems — surfaced, never fatal.
    pub warnings: Vec<String>,
    /// Why there is no usable repo state at all.
    pub error: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            repo: PathBuf::from("."),
            repo_id: String::new(),
            config: None,
            wal_prefix: "refs/sturn/wal".into(),
            workspaces: Vec::new(),
            events: Vec::new(),
            claims: Vec::new(),
            asks: Vec::new(),
            presence: BTreeMap::new(),
            now: 0,
            warnings: Vec::new(),
            error: None,
        }
    }
}

impl Snapshot {
    /// The empty/degraded snapshot — `error` says why.
    fn failed(error: String) -> Self {
        Snapshot {
            error: Some(error),
            ..Snapshot::default()
        }
    }

    /// The `{"error": …}` payload every query emits when the
    /// snapshot couldn't load — the degradation marker pavo
    /// renderers already understand.
    pub fn error_payload(&self) -> serde_json::Value {
        serde_json::json!({"error": self.error.clone().unwrap_or_else(|| "no sturn repo".into())})
    }

    /// Is there usable repo state?
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `params` → `(repo_dir, config_path)` overrides. `dir`/`root`/
/// `repo` all name the repo; `dir` wins when several are given
/// (mirrors the CLI globals' precedence).
fn overrides(
    params: &serde_json::Map<String, serde_json::Value>,
) -> (Option<PathBuf>, Option<String>) {
    let repo = ["dir", "root", "repo"]
        .iter()
        .find_map(|k| params.get(*k).and_then(|v| v.as_str()))
        .map(PathBuf::from);
    let config = params
        .get("config")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    (repo, config)
}

/// Which config file the source chain would read — same candidate
/// order as `TomlSource`, reported for the status view.
fn config_path_hint(explicit: Option<&str>) -> Option<String> {
    let candidates: Vec<PathBuf> = match explicit
        .map(str::to_string)
        .or_else(|| std::env::var("STURN_CONFIG").ok().filter(|s| !s.is_empty()))
    {
        Some(p) => vec![PathBuf::from(p)],
        None => vec![
            PathBuf::from("sturn.toml"),
            std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| ".".into())
                .join(".config/sturn/sturn.toml"),
        ],
    };
    candidates
        .into_iter()
        .find(|p| p.exists())
        .map(|p| p.display().to_string())
}

/// Load config via the same source chain the CLI uses. Failure is
/// a warning, not fatal — sturn works flag-driven with no toml.
fn load_config(path: Option<String>) -> (Option<Config>, Vec<String>) {
    let mut warnings = Vec::new();
    match (sturn_config_toml::TomlSource { path }).load() {
        Ok(cfg) => (Some(cfg), warnings),
        Err(e) => {
            // "no sturn.toml found" is the normal case outside a
            // wired repo — only surface real parse/read errors.
            if !e.contains("no sturn.toml found") {
                warnings.push(format!("config: {e}"));
            }
            (None, warnings)
        }
    }
}

/// Where a `cli.<verb>` re-exec should run — the repo dir plus the
/// config file the same chain `load` uses would read, so engine ops
/// land exactly where the views see state. Both come back absolute:
/// the child's cwd is the repo dir, so a relative candidate like
/// `sturn.toml` would re-resolve against the wrong root.
pub fn resolve(params: &serde_json::Map<String, serde_json::Value>) -> (PathBuf, Option<String>) {
    let (param_repo, param_config) = overrides(params);
    let (cfg, _) = load_config(param_config.clone());
    let repo = param_repo
        .or_else(|| {
            std::env::var("STURN_REPO")
                .ok()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| cfg.as_ref().map(|c| c.repo.dir.clone()))
        .unwrap_or_else(|| PathBuf::from("."));
    let repo = std::fs::canonicalize(&repo).unwrap_or(repo);
    let config = config_path_hint(param_config.as_deref())
        .map(|p| {
            std::fs::canonicalize(&p)
                .map(|a| a.to_string_lossy().into_owned())
                .unwrap_or(p)
        })
        .or(param_config);
    (repo, config)
}

/// Resolve + load. `params` are the wire op's caller params —
/// `dir`/`root`/`repo`/`config` steer resolution, the rest pass
/// through to the query resolvers untouched.
pub fn load(params: &serde_json::Map<String, serde_json::Value>) -> Snapshot {
    let (param_repo, param_config) = overrides(params);
    let config = config_path_hint(param_config.as_deref());
    let (cfg, warnings) = load_config(param_config);

    // Repo resolution — params → $STURN_REPO → cfg.repo.dir → cwd.
    let repo = param_repo
        .or_else(|| {
            std::env::var("STURN_REPO")
                .ok()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| cfg.as_ref().map(|c| c.repo.dir.clone()))
        .unwrap_or_else(|| PathBuf::from("."));

    // Not a git repo → honest degradation, no WAL reads attempted.
    if sturn_odb::git_dir(&repo).is_err() {
        return Snapshot::failed(format!(
            "{}: not a git repo — sturn state lives in refs/sturn/* under a repo",
            repo.display()
        ));
    }

    let repo_id = cfg
        .as_ref()
        .map(|c| c.repo.id())
        .or_else(|| {
            repo.canonicalize()
                .ok()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        })
        .unwrap_or_else(|| "repo".into());
    let wal_prefix = cfg
        .as_ref()
        .map(|c| c.wal.ref_prefix.clone())
        .unwrap_or_else(|| "refs/sturn/wal".into());

    let wal = GitWal {
        ref_prefix: wal_prefix.clone(),
        // Read-only driver — never signs.
        sign_key: None,
    };
    let ws_impl = sturn_ws_git::GitWs;
    let now = now();
    let mut snap = Snapshot {
        repo,
        repo_id,
        config,
        wal_prefix,
        now,
        warnings,
        ..Snapshot::default()
    };

    // Workspace union — namespaces (`refs/sturn/ws/*`) plus WAL
    // logs (`refs/sturn/wal/*`), same as `sturn ws ls`.
    let mut names: Vec<String> = ws_impl
        .workspaces(&snap.repo)
        .map_err(|e| e.to_string())
        .unwrap_or_else(|e| {
            snap.warnings.push(format!("ws namespaces: {e}"));
            Vec::new()
        });
    names.extend(wal.workspaces(&snap.repo).unwrap_or_else(|e| {
        snap.warnings.push(format!("wal workspaces: {e}"));
        Vec::new()
    }));
    names.sort();
    names.dedup();

    let mut events_by_ws: BTreeMap<String, Vec<Event>> = BTreeMap::new();
    for name in &names {
        let mut row = WsRow {
            name: name.clone(),
            kinds: Vec::new(),
            events: 0,
            pending: 0,
            tip: None,
        };
        if ws_impl.tip(&snap.repo, name).is_ok() {
            row.kinds.push("namespace");
        }
        // `wal.events` answers `Ok(vec![])` for a missing log — the
        // head check is what distinguishes "wal-only/absent".
        let head = wal.head(&snap.repo, name).unwrap_or(None);
        if head.is_some() {
            row.kinds.push("wal");
            row.tip = head.map(|h| if h.len() > 8 { h[..8].to_string() } else { h });
            match wal.events(&snap.repo, name) {
                Ok(evs) => {
                    row.events = evs.len();
                    row.pending = evs
                        .len()
                        .saturating_sub(sturn_core::since_last_snapshot(&evs));
                    events_by_ws.insert(name.clone(), evs);
                }
                Err(e) => snap.warnings.push(format!("wal {name}: {e}")),
            }
        }
        snap.workspaces.push(row);
    }

    // Fold order: per-ws WAL order, concatenated — the same order
    // `all_events` in the CLI feeds `claims::active`.
    for evs in events_by_ws.values() {
        snap.events.extend(evs.iter().cloned());
    }

    snap.claims = claims::active(&snap.events, now);
    snap.asks = sturn_ask::all(&snap.events, now);
    for ev in &snap.events {
        if let Op::Presence { status } = &ev.op {
            snap.presence
                .insert((ev.ws.clone(), ev.actor.clone()), status.clone());
        }
    }
    snap
}

/// The five bus topic suffixes under `repo.<id>.` — derived names,
/// not stored state (the bus is a cache; topics are vocabulary).
pub fn topic_rows(repo_id: &str) -> Vec<serde_json::Value> {
    let descs = [
        (topic::OPS, "every op append — the firehose"),
        (topic::CLAIMS, "claim/release ops only — collision warnings"),
        (topic::REVIEW, "approve/reject verdicts on events"),
        (topic::PRESENCE, "actors joining/leaving workspaces"),
        (topic::ASKS, "pending-response ops — asks, answers, cancels"),
    ];
    descs
        .iter()
        .map(|(suffix, desc)| {
            serde_json::json!({
                "topic": format!("repo.{repo_id}.{suffix}"),
                "suffix": suffix,
                "description": desc,
            })
        })
        .collect()
}
