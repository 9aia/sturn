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
            // An actor leaving a ws drops all its claims there —
            // dead agents can't hold the swarm hostage.
            Op::Presence { status } if status == "left" => {
                out.retain(|c| !(c.ws == ev.ws && c.actor == ev.actor));
            }
            _ => {}
        }
    }
    out.retain(|c| c.expires > now);
    out
}

/// Glob match: `*` = any run within a segment, `**` = any run
/// across segments, else literal. Recursive, no deps.
pub fn glob_match(pat: &str, path: &str) -> bool {
    fn m(p: &[u8], s: &[u8]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some(b'*') if p.get(1) == Some(&b'*') => {
                (0..=s.len()).any(|i| m(&p[2..], &s[i..]))
            }
            Some(b'*') => {
                let end = s.iter().position(|&c| c == b'/').unwrap_or(s.len());
                (0..=end).any(|i| m(&p[1..], &s[i..]))
            }
            Some(c) => !s.is_empty() && s[0] == *c && m(&p[1..], &s[1..]),
        }
    }
    m(pat.as_bytes(), path.as_bytes())
}

/// Literal stem — the part before the first wildcard.
fn stem(p: &str) -> &str {
    match p.find(['*', '?', '[']) {
        Some(i) => p[..i].trim_end_matches('/'),
        None => p,
    }
}

/// Do two patterns/paths overlap? Both may be globs — approximate
/// intersection: match each against the other and against the
/// other's literal stem. Conservative — it may over-report on
/// exotic globs, and that's the right failure direction for a lock.
pub fn covers(a: &str, b: &str) -> bool {
    glob_match(a, b)
        || glob_match(b, a)
        || glob_match(a, stem(b))
        || glob_match(b, stem(a))
        || stem(a) == stem(b)
}

/// Live claims that overlap `glob` — what `sturn claim` warns about.
/// Overlap is symmetric: an existing `src/**` claim conflicts with a
/// request for `src/foo.rs`, and vice versa.
pub fn conflicts<'a>(claims: &'a [Active], glob: &str) -> Vec<&'a Active> {
    claims.iter().filter(|c| covers(&c.glob, glob)).collect()
}
