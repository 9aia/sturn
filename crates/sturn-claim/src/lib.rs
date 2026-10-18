//! Claims — advisory locks folded out of the WAL. Publish before
//! editing; contenders learn in milliseconds instead of at merge.
//! Pure functions over the event log: same log, same claims,
//! deterministic everywhere it's computed.

use sturn_core::{Event, Op};

/// A live claim after folding the WAL.
#[derive(Debug, Clone)]
pub struct Active {
    pub ws: String,
    pub actor: String,
    pub glob: String,
    /// Unix seconds — past this, the claim is gone.
    pub expires: u64,
}

/// Fold events into live claims at `now`: Claim upserts on
/// (ws, glob), Release removes, expired TTLs drop. Callers fold
/// *all* workspaces' WALs for swarm-wide claims.
pub fn active(events: &[Event], now: u64) -> Vec<Active> {
    let mut out: Vec<Active> = vec![];
    for ev in events {
        match &ev.op {
            Op::Claim { glob, ttl_secs } => {
                out.retain(|c| !(c.ws == ev.ws && c.glob == *glob));
                out.push(Active {
                    ws: ev.ws.clone(),
                    actor: ev.actor.clone(),
                    glob: glob.clone(),
                    expires: ev.ts + ttl_secs,
                });
            }
            Op::Release { glob } => {
                out.retain(|c| !(c.ws == ev.ws && c.glob == *glob));
            }
            _ => {}
        }
    }
    out.retain(|c| c.expires > now);
    out
}

/// Does pattern `a` cover path/glob `b`? `dir/**` covers the whole
/// subtree, `dir/*` covers one level, otherwise exact match.
pub fn covers(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if let Some(dir) = a.strip_suffix("/**") {
        return b == dir || b.starts_with(&format!("{dir}/"));
    }
    if let Some(stem) = a.strip_suffix("/*") {
        return b
            .strip_prefix(&format!("{stem}/"))
            .map(|r| !r.is_empty() && !r.contains('/'))
            .unwrap_or(false);
    }
    false
}

/// Live claims that overlap `glob` — what `sturn claim` warns about.
/// Overlap is symmetric: an existing `src/**` claim conflicts with a
/// request for `src/foo.rs`, and vice versa.
pub fn conflicts<'a>(claims: &'a [Active], glob: &str) -> Vec<&'a Active> {
    claims
        .iter()
        .filter(|c| covers(&c.glob, glob) || covers(glob, &c.glob))
        .collect()
}
