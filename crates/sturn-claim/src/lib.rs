//! Claims — advisory locks folded out of the WAL. Publish before
//! editing; contenders learn in milliseconds instead of at merge.
//! Pure functions over the event log: same log, same claims,
//! deterministic everywhere it's computed.
//!
//! Claim records carry a typed resource URI `scheme:path` —
//! `fs:src/**` globs, `dev:display0`, `net:13000`, `sess:T-012`,
//! `cap:role`. A bare glob with no scheme is `fs:` — old WALs and
//! scripts keep working unchanged.

use sturn_core::{Event, Op};

/// A live claim after folding the WAL.
#[derive(Debug, Clone)]
pub struct Active {
    pub ws: String,
    pub actor: String,
    /// Canonical resource URI — bare globs carry `fs:` here.
    pub uri: String,
    /// Unix seconds — past this, the claim is gone.
    pub expires: u64,
}

/// Fold events into live claims at `now`: Claim upserts on
/// (ws, uri), Release removes, expired TTLs drop. Callers fold
/// *all* workspaces' WALs for swarm-wide claims.
pub fn active(events: &[Event], now: u64) -> Vec<Active> {
    let mut out: Vec<Active> = vec![];
    for ev in events {
        match &ev.op {
            Op::Claim { glob, ttl_secs } => {
                let uri = normalize(glob);
                out.retain(|c| !(c.ws == ev.ws && c.uri == uri));
                out.push(Active {
                    ws: ev.ws.clone(),
                    actor: ev.actor.clone(),
                    uri,
                    expires: ev.ts + ttl_secs,
                });
            }
            Op::Release { glob } => {
                let uri = normalize(glob);
                out.retain(|c| !(c.ws == ev.ws && c.uri == uri));
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

/// URI scheme shape — ALPHA *(ALPHA / DIGIT / "+" / "-" / ".").
/// Anything else before the first `:` is a path char, not a scheme.
fn scheme_ok(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Split a claim URI into `(scheme, path)`. No valid `scheme:`
/// prefix → `fs` (a bare glob is a filesystem claim).
pub fn split(uri: &str) -> (&str, &str) {
    match uri.find(':') {
        Some(i) if scheme_ok(&uri[..i]) => (&uri[..i], &uri[i + 1..]),
        _ => ("fs", uri),
    }
}

/// Canonical URI — bare globs gain `fs:`, schemes lowercase so
/// `fs:` and `FS:` name the same resource.
pub fn normalize(uri: &str) -> String {
    let (s, p) = split(uri);
    format!("{}:{p}", s.to_ascii_lowercase())
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

/// Do two fs paths/patterns overlap? Both may be globs —
/// approximate intersection: match each against the other and
/// against the other's literal stem. Conservative — it may
/// over-report on exotic globs, and that's the right failure
/// direction for a lock.
pub fn covers(a: &str, b: &str) -> bool {
    glob_match(a, b)
        || glob_match(b, a)
        || glob_match(a, stem(b))
        || glob_match(b, stem(a))
        || stem(a) == stem(b)
}

/// Opaque-scheme overlap: equality, or the shorter URI is a prefix
/// of the longer ending at a `:`/`/` boundary — a bare `dev:`
/// claims the whole scheme while `net:130` stays clear of
/// `net:13000`.
fn opaque_covers(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (s, l) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    l.starts_with(s)
        && (s.ends_with(':')
            || s.ends_with('/')
            || l[s.len()..].starts_with(':')
            || l[s.len()..].starts_with('/'))
}

/// Do two claim URIs overlap? Different schemes never conflict;
/// `fs` uses glob intersection; every other (opaque) scheme uses
/// exact or boundary-prefix match.
pub fn overlaps(a: &str, b: &str) -> bool {
    let (sa, pa) = split(a);
    let (sb, pb) = split(b);
    if !sa.eq_ignore_ascii_case(sb) {
        return false;
    }
    if sa.eq_ignore_ascii_case("fs") {
        covers(pa, pb)
    } else {
        opaque_covers(&normalize(a), &normalize(b))
    }
}

/// Live claims that overlap `uri` — what `sturn claim` warns about.
/// Overlap is symmetric: an existing `src/**` claim conflicts with
/// a request for `src/foo.rs`, and vice versa.
pub fn conflicts<'a>(claims: &'a [Active], uri: &str) -> Vec<&'a Active> {
    claims.iter().filter(|c| overlaps(&c.uri, uri)).collect()
}
