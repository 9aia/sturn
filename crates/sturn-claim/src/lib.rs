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
    /// Fencing token — a Lamport-style epoch derived from the WAL,
    /// never stored: `1 + max` fence over earlier claims on
    /// overlapping URIs. Any later overlapping claim outranks it,
    /// so a holder compares on `check` to detect a usurper.
    pub fence: u64,
}

/// Fold events into live claims at `now`: Claim upserts on
/// (ws, uri), Release removes, expired TTLs drop. Callers fold
/// *all* workspaces' WALs for swarm-wide claims.
pub fn active(events: &[Event], now: u64) -> Vec<Active> {
    let fences = fences(events);
    let mut out: Vec<Active> = vec![];
    for (i, ev) in events.iter().enumerate() {
        match &ev.op {
            Op::Claim { glob, ttl_secs } => {
                let uri = normalize(glob);
                out.retain(|c| !(c.ws == ev.ws && c.uri == uri));
                out.push(Active {
                    ws: ev.ws.clone(),
                    actor: ev.actor.clone(),
                    uri,
                    expires: ev.ts + ttl_secs,
                    fence: fences[i],
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

/// Fencing tokens per Claim event index (0 for non-claims).
/// Claim order: `ts` first — claims race in wall-clock time —
/// then fold position. Same-ws ties resolve by WAL order, which
/// *is* append order; cross-ws ties land on the concatenation
/// order — deterministic, if arbitrary. Expired and released
/// claims still count as predecessors: the epoch never repeats
/// for a resource. Cross-actor clock skew blurs ordering at the
/// edges — the token is advisory, like the claim itself.
fn claim_glob(e: &Event) -> &str {
    match &e.op {
        Op::Claim { glob, .. } => glob,
        _ => "",
    }
}

fn fences(events: &[Event]) -> Vec<u64> {
    let mut order: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e.op, Op::Claim { .. }))
        .map(|(i, _)| i)
        .collect();
    order.sort_by(|&a, &b| (events[a].ts, a).cmp(&(events[b].ts, b)));
    let mut out = vec![0u64; events.len()];
    for (n, &i) in order.iter().enumerate() {
        let uri = normalize(claim_glob(&events[i]));
        let mut f = 1;
        for &j in &order[..n] {
            if overlaps(&uri, &normalize(claim_glob(&events[j]))) {
                f = f.max(out[j] + 1);
            }
        }
        out[i] = f;
    }
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

/// `net:` path as an inclusive port range — `13000` or
/// `13000-13100`; a bare port is a range of one. Anything that
/// isn't numeric (subpaths like `13000/tcp`) isn't a range and
/// falls back to opaque matching.
fn port_range(p: &str) -> Option<(u16, u16)> {
    match p.split_once('-') {
        Some((a, b)) => {
            let (a, b) = (a.parse().ok()?, b.parse().ok()?);
            (a <= b).then_some((a, b))
        }
        None if !p.is_empty() => p.parse().map(|p| (p, p)).ok(),
        None => None,
    }
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
/// `fs` uses glob intersection; `net` uses port-range
/// intersection when both paths parse, else falls back with every
/// other (opaque) scheme to exact or boundary-prefix match.
pub fn overlaps(a: &str, b: &str) -> bool {
    let (sa, pa) = split(a);
    let (sb, pb) = split(b);
    if !sa.eq_ignore_ascii_case(sb) {
        return false;
    }
    if sa.eq_ignore_ascii_case("fs") {
        covers(pa, pb)
    } else if sa.eq_ignore_ascii_case("net") {
        match (port_range(pa), port_range(pb)) {
            (Some((alo, ahi)), Some((blo, bhi))) => alo <= bhi && blo <= ahi,
            _ => opaque_covers(&normalize(a), &normalize(b)),
        }
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
