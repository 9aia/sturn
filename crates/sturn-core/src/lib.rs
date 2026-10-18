//! sturn-core — domain types, `sturn.toml` config, and the shared
//! vocabulary every port and impl speaks.
//!
//! The WAL is the state: ops append to `refs/sturn/wal/<ws>` and the
//! working tree is a projection. Claims are advisory locks published
//! *before* edits — conflicts get prevented, not discovered at merge.

use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;

/// Bus topic suffixes under `repo.<id>.` — the coordination vocabulary.
pub mod topic {
    /// Every op append — the firehose.
    pub const OPS: &str = "ops";
    /// Claim/release ops only — collision warnings.
    pub const CLAIMS: &str = "claims";
    /// Approve/reject verdicts on events.
    pub const REVIEW: &str = "review";
    /// Actors joining/leaving workspaces.
    pub const PRESENCE: &str = "presence";
    /// Everything, for subscribers that want the full stream.
    pub const ALL: [&str; 4] = [OPS, CLAIMS, REVIEW, PRESENCE];
}

/// One op appended to a workspace WAL — the unit of coordination.
/// Stored as a blob in the git ODB, chained by commits on
/// `refs/sturn/wal/<ws>`; replicated by ordinary push/fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Content address — assigned by the Wal impl (event blob sha).
    #[serde(default)]
    pub id: String,
    /// Workspace that produced it.
    pub ws: String,
    /// Who produced it — agent id, human, service.
    pub actor: String,
    /// Unix seconds.
    pub ts: u64,
    /// The operation.
    pub op: Op,
}

/// The op vocabulary. Merge semantics stay git's — ops carry intent
/// and coordination metadata, not a new merge theory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Replace `path` with `blob` (a sha already in the ODB).
    Write {
        path: String,
        blob: String,
        #[serde(default)]
        intent: Option<String>,
    },
    Rename {
        from: String,
        to: String,
    },
    Delete {
        path: String,
    },
    /// Advisory lock on a path glob — publish *before* editing.
    Claim {
        glob: String,
        #[serde(default = "default_ttl")]
        ttl_secs: u64,
    },
    Release {
        glob: String,
    },
    /// Why the work exists — recorded as data, not a commit message.
    Intent { text: String },
    /// Marker — squash prior ops into a real commit.
    Snapshot { message: String },
    /// Approve/reject an earlier event — continuous review.
    Review { event: String, verdict: Verdict },
    /// Actor joined/left the swarm.
    Presence { status: String },
}

fn default_ttl() -> u64 {
    3600
}

/// Review verdict on an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Approve,
    Reject,
}

/// Index just after the last `Snapshot` marker — ops before it are
/// already inside the ws tip commit.
pub fn since_last_snapshot(events: &[Event]) -> usize {
    events
        .iter()
        .rposition(|e| matches!(e.op, Op::Snapshot { .. }))
        .map(|i| i + 1)
        .unwrap_or(0)
}

/// Fold tree-affecting ops into a path→blob overlay (None = deleted).
/// Pass only events *since the last snapshot* — earlier ones are
/// already in the tip. `resolve` maps a path to its blob sha in the
/// base tree, needed when a `Rename` source was never overlaid.
pub fn overlay(
    events: &[Event],
    resolve: impl Fn(&str) -> Option<String>,
) -> std::collections::BTreeMap<String, Option<String>> {
    let mut m = std::collections::BTreeMap::new();
    for ev in events {
        match &ev.op {
            Op::Write { path, blob, .. } => {
                m.insert(path.clone(), Some(blob.clone()));
            }
            Op::Delete { path } => {
                m.insert(path.clone(), None);
            }
            Op::Rename { from, to } => {
                let blob = m.remove(from).flatten().or_else(|| resolve(from));
                m.insert(from.clone(), None);
                if let Some(b) = blob {
                    m.insert(to.clone(), Some(b));
                }
            }
            _ => {}
        }
    }
    m
}

impl Event {
    /// Convenience constructor — `id` is assigned on append.
    pub fn new(ws: impl Into<String>, actor: impl Into<String>, op: Op) -> Self {
        Event {
            id: String::new(),
            ws: ws.into(),
            actor: actor.into(),
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            op,
        }
    }
}

/// sturn.toml — one per coordinated repo. Secrets never live here;
/// secret material resolves through the `Secrets` chain.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The coordinated repo.
    pub repo: RepoCfg,
    #[serde(default)]
    pub wal: WalCfg,
    #[serde(default)]
    pub bus: BusCfg,
    #[serde(default)]
    pub actor: ActorCfg,
}

/// `[repo]` — which git repo this config coordinates.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct RepoCfg {
    /// Repo dir (the ODB lives at `<dir>/.git`).
    pub dir: PathBuf,
    /// Topic prefix id, e.g. "9aia/sturn". Defaults to dir basename.
    #[serde(default)]
    pub id: Option<String>,
    /// Branch `snap` updates by default.
    #[serde(default = "default_branch")]
    pub branch: String,
}

impl RepoCfg {
    /// Resolved repo id — explicit `id` or the dir basename.
    pub fn id(&self) -> String {
        self.id.clone().unwrap_or_else(|| {
            self.dir
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "repo".into())
        })
    }
}

/// `[wal]` — where the op log lives inside git refs.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct WalCfg {
    /// WAL ref prefix — one ref per workspace beneath it.
    #[serde(default = "default_ref_prefix")]
    pub ref_prefix: String,
}

/// `[bus]` — the local pub/sub cache over the WAL.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct BusCfg {
    /// Embedded store path for the local bus impl.
    #[serde(default = "default_bus_store")]
    pub store: PathBuf,
    /// Daemon poll interval for WAL→topic fan-out, ms.
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u64,
}

/// `[actor]` — this process's identity on ops it appends.
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct ActorCfg {
    /// Actor name. Defaults to `$STURN_ACTOR`, then `$USER`.
    #[serde(default)]
    pub name: Option<String>,
}

impl ActorCfg {
    /// Resolved actor name for signing ops.
    pub fn name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            std::env::var("STURN_ACTOR")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_else(|_| "anon".into())
        })
    }
}

fn default_branch() -> String {
    "main".into()
}
fn default_ref_prefix() -> String {
    "refs/sturn/wal".into()
}
fn default_poll_ms() -> u64 {
    100
}
fn default_bus_store() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| ".".into())
        .join(".local/state/sturn/bus.redb")
}

impl Default for WalCfg {
    fn default() -> Self {
        WalCfg {
            ref_prefix: default_ref_prefix(),
        }
    }
}

impl Default for BusCfg {
    fn default() -> Self {
        BusCfg {
            store: default_bus_store(),
            poll_ms: default_poll_ms(),
        }
    }
}
