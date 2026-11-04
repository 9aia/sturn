//! Unit tests for sturn-claim: glob matching, overlap
//! approximation, and folding the WAL into live claims.

use sturn_claim::{active, conflicts, covers, glob_match, normalize, overlaps, split};
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
    assert_eq!(live[0].uri, "fs:y");
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
    assert_eq!(hits[0].uri, "fs:src/**");
    assert!(conflicts(&live, "readme.md").is_empty());
}

// --- resource URIs -------------------------------------------------------

#[test]
fn split_reads_scheme_prefix() {
    assert_eq!(split("src/**"), ("fs", "src/**"));
    assert_eq!(split("fs:src/**"), ("fs", "src/**"));
    assert_eq!(split("dev:display0"), ("dev", "display0"));
    assert_eq!(split("net:13000"), ("net", "13000"));
    assert_eq!(split("sess:T-012"), ("sess", "T-012"));
    assert_eq!(split("dev:"), ("dev", ""));
    // No valid scheme → the whole string is an fs path.
    assert_eq!(split(":x"), ("fs", ":x"));
    assert_eq!(split("1a:x"), ("fs", "1a:x"));
    // A colon past the first keeps working inside a path.
    assert_eq!(split("fs:a:b"), ("fs", "a:b"));
}

#[test]
fn normalize_canonicalizes_scheme() {
    assert_eq!(normalize("src/**"), "fs:src/**");
    assert_eq!(normalize("fs:src/**"), "fs:src/**");
    assert_eq!(normalize("FS:src/**"), "fs:src/**");
    assert_eq!(normalize("net:13000"), "net:13000");
    assert_eq!(normalize("Net:13000"), "net:13000");
}

#[test]
fn overlaps_fs_uses_glob_intersection() {
    // Regression — bare and `fs:`-prefixed are the same claim.
    assert!(overlaps("src/**", "src/foo.rs"));
    assert!(overlaps("fs:src/**", "src/foo.rs"));
    assert!(overlaps("src/**", "fs:src/foo.rs"));
    assert!(!overlaps("src/**", "docs/**"));
    // Different schemes never conflict.
    assert!(!overlaps("src/**", "net:13000"));
}

#[test]
fn overlaps_opaque_exact_and_boundary_prefix() {
    assert!(overlaps("dev:display0", "dev:display0"));
    // A bare `dev:` claims the whole scheme.
    assert!(overlaps("dev:", "dev:display0"));
    assert!(overlaps("dev:display0", "dev:"));
    // Hierarchy inside the path part splits on `/`.
    assert!(overlaps("dev:display0", "dev:display0/gpu0"));
    // No mid-token prefix — port 130 is not port 13000.
    assert!(!overlaps("net:130", "net:13000"));
    assert!(!overlaps("net:13000", "net:130"));
    assert!(!overlaps("sess:T-01", "sess:T-012"));
    assert!(!overlaps("cap:release", "cap:release-cutter"));
    assert!(!overlaps("dev:display0", "sess:display0"));
}

#[test]
fn claim_and_release_match_on_canonical_uri() {
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
        // `fs:`-spelled release drops the bare-glob claim.
        ev(
            "w1",
            "a",
            Op::Release {
                glob: "fs:src/**".into(),
            },
            1,
        ),
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "net:13000".into(),
                ttl_secs: 100,
            },
            2,
        ),
    ];
    let live = active(&events, 50);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].uri, "net:13000");
}

#[test]
fn overlaps_net_port_ranges() {
    assert!(overlaps("net:13000-13100", "net:13050"));
    assert!(overlaps("net:13050", "net:13000-13100"));
    // Bounds are inclusive.
    assert!(overlaps("net:13000-13100", "net:13100-13200"));
    assert!(!overlaps("net:13000-13100", "net:13150"));
    assert!(overlaps("net:13000", "net:13000-13100"));
    // Bare scheme still claims every port.
    assert!(overlaps("net:", "net:13000-13100"));
    // Non-numeric paths fall back to opaque matching.
    assert!(overlaps("net:13000/tcp", "net:13000"));
    assert!(!overlaps("net:13000/tcp", "net:13001"));
    // A reversed range isn't a range — opaque rules, no prefix.
    assert!(!overlaps("net:13100-13000", "net:13050"));
}

// --- fencing tokens ------------------------------------------------------

#[test]
fn fence_rises_when_a_usurper_claims() {
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "dev:display0".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w2",
            "b",
            Op::Claim {
                glob: "dev:display0".into(),
                ttl_secs: 100,
            },
            1,
        ),
    ];
    let live = active(&events, 50);
    assert_eq!(live[0].fence, 1);
    assert_eq!(live[1].fence, 2);
}

#[test]
fn fence_orders_by_ts_across_workspaces() {
    // `all_events` concatenates ws logs — fold order isn't claim
    // order. The later-ts claim is the usurper wherever it sits.
    let events = vec![
        ev(
            "z",
            "b",
            Op::Claim {
                glob: "net:9".into(),
                ttl_secs: 100,
            },
            10,
        ),
        ev(
            "a",
            "a",
            Op::Claim {
                glob: "net:9".into(),
                ttl_secs: 100,
            },
            5,
        ),
    ];
    let live = active(&events, 50);
    assert_eq!(live[0].fence, 2); // ws z claimed at ts 10
    assert_eq!(live[1].fence, 1); // ws a claimed at ts 5
}

#[test]
fn fence_never_repeats_after_release() {
    // Released history still counts — the epoch can't rewind.
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "dev:x".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w1",
            "a",
            Op::Release {
                glob: "dev:x".into(),
            },
            1,
        ),
        ev(
            "w2",
            "b",
            Op::Claim {
                glob: "dev:x".into(),
                ttl_secs: 100,
            },
            2,
        ),
    ];
    let live = active(&events, 50);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].fence, 2);
}

#[test]
fn fence_counts_expired_claims() {
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "dev:x".into(),
                ttl_secs: 1,
            },
            0,
        ),
        ev(
            "w2",
            "b",
            Op::Claim {
                glob: "dev:x".into(),
                ttl_secs: 100,
            },
            5,
        ),
    ];
    let live = active(&events, 50);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].fence, 2);
}

#[test]
fn fence_only_rises_over_overlap() {
    // Unrelated resources keep their own epochs — fences compare
    // only where claims contend.
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "dev:x".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w2",
            "b",
            Op::Claim {
                glob: "net:1".into(),
                ttl_secs: 100,
            },
            1,
        ),
        ev(
            "w3",
            "c",
            Op::Claim {
                glob: "dev:x/gpu0".into(),
                ttl_secs: 100,
            },
            2,
        ),
    ];
    let live = active(&events, 50);
    assert_eq!(live[0].fence, 1); // dev:x
    assert_eq!(live[1].fence, 1); // net:1 — different scheme
    assert_eq!(live[2].fence, 2); // dev:x/gpu0 prefixes under dev:x
}

#[test]
fn same_ws_reclaim_bumps_the_fence() {
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
                glob: "src/**".into(),
                ttl_secs: 100,
            },
            1,
        ),
    ];
    let live = active(&events, 50);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].fence, 2);
}

#[test]
fn conflicts_reports_opaque_holders() {
    let events = vec![
        ev(
            "w1",
            "a",
            Op::Claim {
                glob: "net:13000".into(),
                ttl_secs: 100,
            },
            0,
        ),
        ev(
            "w2",
            "b",
            Op::Claim {
                glob: "sess:T-012".into(),
                ttl_secs: 100,
            },
            0,
        ),
    ];
    let live = active(&events, 50);
    assert_eq!(conflicts(&live, "net:13000").len(), 1);
    assert_eq!(conflicts(&live, "net:").len(), 1);
    assert!(conflicts(&live, "net:130").is_empty());
    assert!(conflicts(&live, "sess:").len() == 1);
    assert!(conflicts(&live, "dev:display0").is_empty());
}
