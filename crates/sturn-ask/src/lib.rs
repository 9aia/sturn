//! Asks — pending *responses* folded out of the WAL. Claims are
//! pending ownership; asks are pending answers. Pure functions over
//! the event log: same log, same state, deterministic everywhere.
//!
//! An `Ask` event opens a question under its own event id (the blob
//! sha). `Answer` and `Cancel` close it — first terminal op wins.
//! `ttl_secs == 0` means the ask never expires on its own.

use std::collections::HashMap;
use sturn_core::{Event, Op};

/// An ask recorded in the WAL.
#[derive(Debug, Clone)]
pub struct Open {
    /// The Ask event's id — what answers reference.
    pub id: String,
    /// Workspace that asked.
    pub ws: String,
    pub actor: String,
    pub subject: String,
    pub target: String,
    pub options: Vec<String>,
    /// Unix seconds the ask was made.
    pub asked: u64,
    /// Unix seconds past which the ask is gone — None = no expiry.
    pub expires: Option<u64>,
}

/// Terminal state of an ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// No terminal op yet and ttl hasn't run out.
    Open,
    /// An `Answer` landed — carries the verdict + who gave it.
    Answered { verdict: String, by: String },
    /// A `Cancel` withdrew it.
    Cancelled,
    /// Its ttl ran out unanswered — expiry is derived, not an op.
    Expired,
}

enum Fold {
    Live,
    Done(Status),
}

/// Fold `events` into `(ask, status)` pairs in append order at
/// `now`. Cross-workspace by nature — answers can land on any ws's
/// WAL, so callers fold *all* workspaces' events. `Answer`/`Cancel`
/// for an unknown or already-closed ask is ignored.
fn fold(events: &[Event], now: u64) -> Vec<(Open, Status)> {
    let mut asks: HashMap<String, (Open, Fold)> = HashMap::new();
    let mut order: Vec<String> = vec![];
    for ev in events {
        match &ev.op {
            Op::Ask {
                subject,
                target,
                options,
                ttl_secs,
            } => {
                asks.entry(ev.id.clone()).or_insert_with(|| {
                    order.push(ev.id.clone());
                    (
                        Open {
                            id: ev.id.clone(),
                            ws: ev.ws.clone(),
                            actor: ev.actor.clone(),
                            subject: subject.clone(),
                            target: target.clone(),
                            options: options.clone(),
                            asked: ev.ts,
                            expires: if *ttl_secs == 0 {
                                None
                            } else {
                                Some(ev.ts + ttl_secs)
                            },
                        },
                        Fold::Live,
                    )
                });
            }
            Op::Answer { ask, verdict } => {
                if let Some((_, f)) = asks.get_mut(ask) {
                    if matches!(f, Fold::Live) {
                        *f = Fold::Done(Status::Answered {
                            verdict: verdict.clone(),
                            by: ev.actor.clone(),
                        });
                    }
                }
            }
            Op::Cancel { ask } => {
                if let Some((_, f)) = asks.get_mut(ask) {
                    if matches!(f, Fold::Live) {
                        *f = Fold::Done(Status::Cancelled);
                    }
                }
            }
            _ => {}
        }
    }
    order
        .into_iter()
        .filter_map(|id| {
            let (o, f) = asks.remove(&id)?;
            let st = match f {
                Fold::Done(s) => s,
                Fold::Live => match o.expires {
                    Some(e) if e <= now => Status::Expired,
                    _ => Status::Open,
                },
            };
            Some((o, st))
        })
        .collect()
}

/// Every ask and its state, oldest first.
pub fn all(events: &[Event], now: u64) -> Vec<(Open, Status)> {
    fold(events, now)
}

/// Asks still open at `now` — everything a responder could pick up.
pub fn open(events: &[Event], now: u64) -> Vec<Open> {
    fold(events, now)
        .into_iter()
        .filter(|(_, s)| matches!(s, Status::Open))
        .map(|(o, _)| o)
        .collect()
}

/// One ask's state — None if the id never was an Ask.
pub fn status(events: &[Event], now: u64, id: &str) -> Option<Status> {
    fold(events, now)
        .into_iter()
        .find(|(o, _)| o.id == id)
        .map(|(_, s)| s)
}
