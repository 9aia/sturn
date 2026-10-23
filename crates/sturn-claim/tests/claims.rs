//! Unit tests for sturn-claim: glob matching, overlap
//! approximation, and folding the WAL into live claims.

use sturn_claim::{active, conflicts, covers, glob_match};
use sturn_core::{Event, Op};

fn ev(ws: &str, actor: &str, op: Op, ts: u64) -> Event {
    Event {
        id: String::new(),
        ws: ws.into(),
        actor: actor.into(),
        ts,
        sig: None,
        op,
    }
}

// --- glob_match ---------------------------------------------------------

#[test]
fn glob_literal() {
    assert!(glob_match("a/b.rs", "a/b.rs"));
    assert!(!glob_match("a/b.rs", "a/c.rs"));
    assert!(!glob_match("a", "ab"));
}

#[test]
fn glob_star_within_segment() {
    assert!(glob_match("src/*.rs", "src/main.rs"));
    assert!(!glob_match("src/*.rs", "src/deep/main.rs"));
    assert!(glob_match("*.rs", "x.rs"));
}

#[test]
fn glob_double_star_crosses_segments() {
    assert!(glob_match("src/**", "src/a/b/c.rs"));
    assert!(glob_match("**", "any/path/at/all"));
    assert!(glob_match("**/x.rs", "a/b/x.rs"));
    assert!(!glob_match("src/**", "other/x.rs"));
}

#[test]
fn glob_edges() {
    assert!(glob_match("", ""));
    assert!(!glob_match("", "x"));
    assert!(!glob_match("a/", "a"));
    assert!(glob_match("a*", "abc"));
}

// --- covers -------------------------------------------------------------

#[test]
fn covers_is_symmetric_overlap() {
    assert!(covers("src/**", "src/foo.rs"));
    assert!(covers("src/foo.rs", "src/**"));
    assert!(covers("src/*.rs", "src/main.rs"));
    assert!(!covers("src/**", "docs/**"));
    // Identical literal stems overlap.
    assert!(covers("src/*.rs", "src/*.toml"));
}

// --- active / conflicts -------------------------------------------------

#[test]
fn claim_upserts_per_ws_and_glob() {
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "src/**".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w1",
            "b",
            Op::Claim {
                glob: "src/**".into(),
                ttl_secs: 200,
            },
            0,
        ),
    ];
    let live = active(&events, 50);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].actor, "b");
    assert_eq!(live[0].expires, 200);
}

#[test]
fn release_drops_claim() {
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "src/**".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w1",
            "a",
            Op::Release {
                glob: "src/**".into(),
            },
            1,
        ),
    ];
    assert!(active(&events, 50).is_empty());
}

#[test]
fn release_in_other_ws_keeps_claim() {
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "src/**".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w2",
            "a",
            Op::Release {
                glob: "src/**".into(),
            },
            1,
        ),
    ];
    assert_eq!(active(&events, 50).len(), 1);
}

#[test]
fn presence_left_drops_actors_claims() {
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "src/**".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "docs/**".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w1",
            "b",
            Op::Claim {
                glob: "tests/**".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w1",
            "a",
            Op::Presence {
                status: "left".into(),
            },
            1,
        ),
    ];
    let live = active(&events, 50);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].actor, "b");
}

#[test]
fn presence_joined_keeps_claims() {
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "src/**".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w1",
            "a",
            Op::Presence {
                status: "joined".into(),
            },
            1,
        ),
    ];
    assert_eq!(active(&events, 50).len(), 1);
}

#[test]
fn expired_claims_drop() {
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "x".into(),
                ttl_secs: 10,
            },
            0,
        ),
        ev(
            "w1",
            "b",
            Op::Claim {
                glob: "y".into(),
                ttl_secs: 100,
            },
            0,
        ),
    ];
    let live = active(&events, 50);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].glob, "y");
}

#[test]
fn conflicts_reports_overlapping_holders() {
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "src/**".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w2",
            "b",
            Op::Claim {
                glob: "docs/**".into(),
                ttl_secs: 100,
            },
            0,
        ),
    ];
    let live = active(&events, 50);
    let hits = conflicts(&live, "src/main.rs");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].glob, "src/**");
    assert!(conflicts(&live, "readme.md").is_empty());
}
