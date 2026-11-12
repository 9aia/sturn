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
    /// Pending-response ops — asks, answers, cancels.
    pub const ASKS: &str = "asks";
    /// Everything, for subscribers that want the full stream.
    pub const ALL: [&str; 5] = [OPS, CLAIMS, REVIEW, PRESENCE, ASKS];
}

/// One op appended to a workspace WAL — the unit of coordination.
/// Storage is Wal-impl defined (git impl: blob in the ODB, chained
/// by commits on `refs/sturn/wal/<ws>`); replicated by ordinary
/// push/fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Content address — assigned by the Wal impl (git: event blob sha).
    #[serde(default)]
    pub id: String,
    /// Workspace that produced it.
    pub ws: String,
    /// Who produced it — agent id, human, service.
    pub actor: String,
    /// Unix seconds.
    pub ts: u64,
    /// SSH signature (PEM) over the event with `sig` empty —
    /// `ssh-keygen -Y sign`, namespace `sturn-event`. Attribution
    /// becomes authentication when `[signing] key` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig: Option<String>,
    /// The operation.
    pub op: Op,
}

/// The op vocabulary. Merge semantics stay git's — ops carry intent
/// and coordination metadata, not a new merge theory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Replace `path` with `blob` (an object id already resolvable
    /// via `Ws::blob`).
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
    /// Advisory lock on a resource URI — publish *before* acting.
    /// `glob` carries `scheme:path`: `fs:` globs (`src/**`, and a
    /// bare glob means `fs:`) or opaque resources (`dev:display0`,
    /// `net:13000`, `sess:T-012`, `cap:role`).
    Claim {
        glob: String,
        #[serde(default = "default_ttl")]
        ttl_secs: u64,
    },
    Release {
        glob: String,
    },
    /// Why the work exists — recorded as data, not a commit message.
    Intent {
        text: String,
    },
    /// Marker — squash prior ops into a real commit.
    Snapshot {
        message: String,
    },
    /// Approve/reject an earlier event — continuous review.
    Review {
        event: String,
        verdict: Verdict,
    },
    /// Actor joined/left the swarm.
    Presence {
        status: String,
    },
    /// A question awaiting a verdict — pending *response* in the
    /// WAL. The ask id is this event's own id (the blob sha), the
    /// same handle `review` uses.
    Ask {
        /// What is being asked.
        subject: String,
        /// Who may answer — free-form spec: `actor:<name>`,
        /// `human:<name>`, `node:<host>`, `claim-holder:<glob>`,
        /// `any`. Routers and responders filter on it.
        target: String,
        /// Offered verdicts — advisory, answers are free-form.
        #[serde(default)]
        options: Vec<String>,
        /// Seconds until the ask expires. 0 = never.
        #[serde(default = "default_ttl")]
        ttl_secs: u64,
    },
    /// A verdict closing an ask — first terminal op wins.
    Answer {
        /// The Ask event id being answered.
        ask: String,
        verdict: String,
    },
    /// Withdraw an open ask.
    Cancel {
        /// The Ask event id being withdrawn.
        ask: String,
    },
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

/// What `merge` returns — the applied stream plus everything the
/// fold noticed that a path-blind concat would hide.
#[derive(Debug, Default)]
pub struct Merge {
    /// Ops in apply order — writes retargeted by a rename re-emit
    /// last, so they land on the destination however the
    /// workspaces were ordered.
    pub events: Vec<Event>,
    /// `conflict:`/`note:` lines — advisory; the ops themselves
    /// stay git's merge.
    pub warnings: Vec<String>,
}

/// Fold several workspaces' pending ops into ONE rename-aware
/// stream for a commit. Concatenation order still decides true
/// same-path collisions — but a Write that hits a path another ws
/// renamed away follows the rename (the modify-vs-rename case a
/// 3-way merge resolves the same way). Genuinely ambiguous
/// shapes — a source renamed two ways, deletes meeting renames,
/// colliding destinations — stay put and report in `warnings`
/// instead of picking a silent winner.
pub fn merge(events: &[Event]) -> Merge {
    use std::collections::{BTreeMap, BTreeSet};

    // Per-path op census — the signal a path-overlay concat loses.
    let mut renames: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    let mut dests: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    let mut ws_renames: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    let mut writes: BTreeMap<&str, Vec<&Event>> = BTreeMap::new();
    let mut deletes: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for ev in events {
        match &ev.op {
            Op::Write { path, .. } => writes.entry(path).or_default().push(ev),
            Op::Delete { path } => deletes.entry(path).or_default().push(&ev.ws),
            Op::Rename { from, to } => {
                renames.entry(from).or_default().push((&ev.ws, to));
                dests.entry(to).or_default().push((&ev.ws, from));
                ws_renames.entry(&ev.ws).or_default().push((from, to));
            }
            _ => {}
        }
    }

    /// Final resting place of `path` under `ws`'s own ops — replay
    /// its renames in order tracking the file's current name, so
    /// f→g→h lands h and a cycle landing back on f means f.
    fn final_to(ws_renames: &BTreeMap<&str, Vec<(&str, &str)>>, ws: &str, path: &str) -> String {
        let mut cur = path.to_string();
        for (f, t) in ws_renames.get(ws).into_iter().flatten() {
            if *f == cur {
                cur = t.to_string();
            }
        }
        cur
    }

    let mut m = Merge::default();

    // A source renamed to different final destinations by
    // different ws — no write can safely follow that.
    for (from, rs) in &renames {
        let by_ws: BTreeMap<&str, String> = rs
            .iter()
            .map(|(w, _)| (*w, final_to(&ws_renames, w, from)))
            .collect();
        if by_ws.values().collect::<BTreeSet<_>>().len() > 1 {
            m.warnings.push(format!(
                "conflict: rename {from} diverged — {}",
                by_ws
                    .iter()
                    .map(|(w, t)| format!("{t} (ws {w})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        // A foreign delete of the source: the deleter wants the
        // file gone, the renamer moved it. The delete still lands
        // (on the old path, now a no-op) — the content survives.
        let rws: BTreeSet<&str> = rs.iter().map(|(w, _)| *w).collect();
        for w in deletes
            .get(from)
            .into_iter()
            .flatten()
            .filter(|w| !rws.contains(**w))
        {
            m.warnings.push(format!(
                "conflict: delete {from} (ws {w}) vs rename {}",
                rs.iter()
                    .map(|(rw, t)| format!("{from} → {t} (ws {rw})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    // Something else landing where a rename lands.
    for (to, rs) in &dests {
        let rws: BTreeSet<&str> = rs.iter().map(|(w, _)| *w).collect();
        // Two ws renaming different sources onto one path.
        if rws.len() > 1 && rs.iter().map(|(_, f)| f).collect::<BTreeSet<_>>().len() > 1 {
            m.warnings.push(format!(
                "conflict: {to} claimed by renames {}",
                rs.iter()
                    .map(|(w, f)| format!("{f} (ws {w})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        // A foreign write at the destination — order decides it.
        for ev in writes
            .get(to)
            .into_iter()
            .flatten()
            .filter(|e| !rws.contains(e.ws.as_str()))
        {
            m.warnings.push(format!(
                "conflict: {to} written (ws {}) vs rename {} (ws {})",
                ev.ws,
                rs.iter()
                    .map(|(_, f)| format!("{f} → {to}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                rws.iter().copied().collect::<Vec<_>>().join("/")
            ));
        }
        // A foreign delete at the destination kills the moved file.
        for w in deletes
            .get(to)
            .into_iter()
            .flatten()
            .filter(|w| !rws.contains(**w))
        {
            m.warnings.push(format!(
                "conflict: delete {to} (ws {w}) vs rename {} (ws {})",
                rs.iter()
                    .map(|(_, f)| format!("{f} → {to}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                rws.iter().copied().collect::<Vec<_>>().join("/")
            ));
        }
    }

    // Same-path collisions a concat would silently order past.
    for (path, evs) in &writes {
        let mut by_ws: BTreeMap<&str, &Event> = BTreeMap::new();
        for e in evs {
            by_ws.insert(&e.ws, e);
        }
        if by_ws.len() > 1 {
            m.warnings.push(format!(
                "conflict: {path} written by {} — last op wins",
                by_ws
                    .iter()
                    .map(|(w, e)| {
                        let i = match &e.op {
                            Op::Write {
                                intent: Some(i), ..
                            } => format!(" \"{i}\""),
                            _ => String::new(),
                        };
                        format!("{w}{i}")
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    for (path, ds) in &deletes {
        let writers: BTreeSet<&str> = writes
            .get(path)
            .into_iter()
            .flatten()
            .map(|e| e.ws.as_str())
            .collect();
        let deleters: BTreeSet<&str> = ds.iter().copied().collect();
        if !writers.is_empty()
            && (writers.iter().any(|w| !deleters.contains(w))
                || deleters.iter().any(|w| !writers.contains(w)))
        {
            m.warnings.push(format!(
                "conflict: {path} written (ws {}) vs deleted (ws {}) — order decides",
                writers.iter().copied().collect::<Vec<_>>().join("/"),
                deleters.iter().copied().collect::<Vec<_>>().join("/")
            ));
        }
    }

    // The retarget pass — writes onto rename sources follow the
    // rename when the destination is unambiguous.
    let mut deferred: Vec<Event> = vec![];
    for ev in events {
        let Op::Write { path, .. } = &ev.op else {
            m.events.push(ev.clone());
            continue;
        };
        let rs = renames.get(path.as_str());
        // A ws that renamed the path itself manages its own intent.
        let own = rs.is_some_and(|rs| rs.iter().any(|(w, _)| *w == ev.ws));
        let dsts: BTreeSet<String> = rs
            .into_iter()
            .flatten()
            .filter(|(w, _)| *w != ev.ws.as_str())
            .map(|(w, _)| final_to(&ws_renames, w, path))
            .collect();
        if own || dsts.len() != 1 || dsts.contains(path.as_str()) {
            m.events.push(ev.clone());
            continue;
        }
        let to = dsts.into_iter().next().unwrap();
        let i = match &ev.op {
            Op::Write {
                intent: Some(i), ..
            } => format!(" \"{i}\""),
            _ => String::new(),
        };
        m.warnings.push(format!(
            "note: write {path} (ws {}{i}) → {to} — follows rename",
            ev.ws
        ));
        // What the retargeted write lands on:
        for (w, _) in rs
            .into_iter()
            .flatten()
            .filter(|(w, _)| *w != ev.ws.as_str())
        {
            // The renaming ws rewrote the source first — its
            // version at `to` loses to the retarget.
            if writes
                .get(path.as_str())
                .is_some_and(|evs| evs.iter().any(|e| e.ws == *w))
            {
                m.warnings.push(format!(
                    "conflict: ws {w} rewrote {path} before renaming — ws {} write wins {to}",
                    ev.ws
                ));
            }
        }
        for e in writes
            .get(to.as_str())
            .into_iter()
            .flatten()
            .filter(|e| e.ws != ev.ws)
        {
            m.warnings.push(format!(
                "conflict: {to} written (ws {}) — retargeted write (ws {}) lands last",
                e.ws, ev.ws
            ));
        }
        for w in deletes
            .get(to.as_str())
            .into_iter()
            .flatten()
            .filter(|w| **w != ev.ws)
        {
            m.warnings.push(format!(
                "conflict: retargeted write {to} (ws {}) vs delete (ws {w})",
                ev.ws
            ));
        }
        let mut ev2 = ev.clone();
        if let Op::Write { path: p, .. } = &mut ev2.op {
            *p = to;
        }
        deferred.push(ev2);
    }
    m.events.extend(deferred);
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
            sig: None,
            op,
        }
    }

    /// Canonical bytes for signing — the event with `sig` and `id`
    /// cleared. `id` is derived (the wal assigns the blob sha *after*
    /// signing), so it can't be part of the signed payload; serde
    /// field order is deterministic, so this round-trips.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut e = self.clone();
        e.sig = None;
        e.id = String::new();
        serde_json::to_vec(&e).unwrap_or_default()
    }
}

/// sturn.toml — one per coordinated repo. Secrets never live here;
/// secret material resolves through the `Secrets` chain.
#[derive(Debug, Serialize, Deserialize, Clone)]
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
    #[serde(default)]
    pub signing: SigningCfg,
    #[serde(default)]
    pub review: ReviewCfg,
    #[serde(default)]
    pub hooks: HooksCfg,
}

/// `[hooks]` — external commands the CLI runs around ops. Off by
/// default; every hook is best-effort and detached — a failing
/// hook can never fail the op that triggered it.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct HooksCfg {
    /// Shell commands spawned (`sh -c`, detached, cwd = repo dir)
    /// after a successful ref-moving verb — claim, leave, write,
    /// snap, push, … The event-driven way to re-index: e.g.
    /// `post_op = "kuda sync"` replaces `kuda watch --poll-refs`
    /// when sturn drives the writes. One string or a list;
    /// `$STURN_POST_OP` (single command) overrides.
    #[serde(default, deserialize_with = "one_or_many")]
    pub post_op: Vec<String>,
}

impl HooksCfg {
    /// Resolved post-op commands — `$STURN_POST_OP` overrides the
    /// configured list when set to a non-empty value.
    pub fn post_ops(&self) -> Vec<String> {
        match std::env::var("STURN_POST_OP") {
            Ok(s) if !s.trim().is_empty() => vec![s],
            _ => self.post_op.clone(),
        }
    }
}

/// `post_op = "cmd"` and `post_op = ["a", "b"]` both parse.
fn one_or_many<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

/// `[review]` — turn continuous review into a gate: snap refuses
/// unreviewed tree ops.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct ReviewCfg {
    /// Every Write/Rename/Delete since last snap must hold an
    /// Approve review (in any ws's WAL) before `snap` lands.
    #[serde(default)]
    pub require: bool,
}

/// `[signing]` — sign ops with an SSH key (`ssh-keygen -Y sign`,
/// namespace `sturn-event`).
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct SigningCfg {
    /// Private key path (ed25519). ~ expands.
    #[serde(default)]
    pub key: Option<String>,
    /// allowed_signers for `sturn verify`.
    #[serde(default)]
    pub allowed_signers: Option<String>,
}

/// `[repo]` — which git repo this config coordinates.
#[derive(Debug, Serialize, Deserialize, Clone)]
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
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct WalCfg {
    /// WAL ref prefix — one ref per workspace beneath it.
    #[serde(default = "default_ref_prefix")]
    pub ref_prefix: String,
}

/// `[bus]` — the local pub/sub cache over the WAL.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct BusCfg {
    /// Embedded store path for the local bus impl.
    #[serde(default = "default_bus_store")]
    pub store: PathBuf,
    /// Daemon poll interval for WAL→topic fan-out, ms.
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u64,
    /// Extra repo dirs this daemon also fans out — multi-repo from
    /// one process. Topic ids derive from dir basename.
    #[serde(default)]
    pub repos: Vec<PathBuf>,
}

/// `[actor]` — this process's identity on ops it appends.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
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
            repos: vec![],
        }
    }
}
