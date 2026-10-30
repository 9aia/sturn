//! Unit tests for sturn-ask: folding the WAL into open/closed asks.
//! Event ids stand in for the blob shas the real wal assigns.

use sturn_ask::{all, open, status, Status};
use sturn_core::{Event, Op};

fn ev(id: &str, ws: &str, actor: &str, op: Op, ts: u64) -> Event {
    Event {
        id: id.into(),
        ws: ws.into(),
        actor: actor.into(),
        ts,
        sig: None,
        op,
    }
}

fn ask(id: &str, ws: &str, ts: u64, ttl: u64) -> Event {
    ev(
        id,
        ws,
        "agent-7",
        Op::Ask {
            subject: "yield src/**?".into(),
            target: "any".into(),
            options: vec!["release".into(), "deny".into()],
            ttl_secs: ttl,
        },
        ts,
    )
}

// --- status ---------------------------------------------------------------

#[test]
fn unknown_ask_is_none() {
    let events = vec![];
    assert_eq!(status(&events, 100, "nope"), None);
    // A non-ask event id is also not an ask.
    let events = vec![ev(
        "e1",
        "w",
        "a",
        Op::Intent { text: "i".into() },
        10,
    )];
    assert_eq!(status(&events, 100, "e1"), None);
}

#[test]
fn fresh_ask_is_open() {
    let events = vec![ask("a1", "w1", 10, 100)];
    assert_eq!(status(&events, 50, "a1"), Some(Status::Open));
}

#[test]
fn answer_closes_and_first_wins() {
    let events = vec![
        ask("a1", "w1", 10, 100),
        ev(
            "r1",
            "w2",
            "human:luis",
            Op::Answer {
                ask: "a1".into(),
                verdict: "release".into(),
            },
            20,
        ),
        // Second answer is ignored — first terminal op wins.
        ev(
            "r2",
            "w2",
            "agent-x",
            Op::Answer {
                ask: "a1".into(),
                verdict: "deny".into(),
            },
            21,
        ),
    ];
    assert_eq!(
        status(&events, 30, "a1"),
        Some(Status::Answered {
            verdict: "release".into(),
            by: "human:luis".into()
        })
    );
}

#[test]
fn cancel_closes() {
    let events = vec![
        ask("a1", "w1", 10, 100),
        ev("c1", "w1", "agent-7", Op::Cancel { ask: "a1".into() }, 20),
    ];
    assert_eq!(status(&events, 30, "a1"), Some(Status::Cancelled));
}

#[test]
fn expiry_is_derived() {
    let events = vec![ask("a1", "w1", 10, 100)];
    assert_eq!(status(&events, 109, "a1"), Some(Status::Open));
    assert_eq!(status(&events, 110, "a1"), Some(Status::Expired));
}

#[test]
fn zero_ttl_never_expires() {
    let events = vec![ask("a1", "w1", 10, 0)];
    assert_eq!(status(&events, u64::MAX - 1, "a1"), Some(Status::Open));
}

#[test]
fn answered_ask_does_not_expire() {
    let events = vec![
        ask("a1", "w1", 10, 100),
        ev(
            "r1",
            "w2",
            "x",
            Op::Answer {
                ask: "a1".into(),
                verdict: "v".into(),
            },
            20,
        ),
    ];
    assert_eq!(
        status(&events, 9999, "a1"),
        Some(Status::Answered {
            verdict: "v".into(),
            by: "x".into()
        })
    );
}

#[test]
fn terminal_ops_for_unknown_asks_are_ignored() {
    let events = vec![
        ev(
            "r1",
            "w",
            "x",
            Op::Answer {
                ask: "ghost".into(),
                verdict: "v".into(),
            },
            10,
        ),
        ev("c1", "w", "x", Op::Cancel { ask: "ghost".into() }, 11),
    ];
    assert_eq!(status(&events, 20, "ghost"), None);
    assert!(open(&events, 20).is_empty());
}

#[test]
fn answers_from_other_workspaces_close_the_ask() {
    // The ask lives on w1's WAL, the answer on w2's — the fold is
    // swarm-wide by construction.
    let events = vec![
        ask("a1", "w1", 10, 100),
        ev(
            "r1",
            "w2",
            "node:box2",
            Op::Answer {
                ask: "a1".into(),
                verdict: "accept".into(),
            },
            20,
        ),
    ];
    assert_eq!(
        status(&events, 30, "a1"),
        Some(Status::Answered {
            verdict: "accept".into(),
            by: "node:box2".into()
        })
    );
}

// --- open / all -----------------------------------------------------------

#[test]
fn open_lists_only_live_asks() {
    let events = vec![
        ask("a1", "w1", 10, 100),
        ask("a2", "w2", 11, 100),
        ask("a3", "w1", 12, 5),
        ask("a4", "w1", 13, 100),
        ev(
            "r1",
            "w2",
            "x",
            Op::Answer {
                ask: "a2".into(),
                verdict: "v".into(),
            },
            20,
        ),
        ev("c1", "w1", "x", Op::Cancel { ask: "a4".into() }, 21),
    ];
    let ids: Vec<String> = open(&events, 30)
        .iter()
        .map(|o| o.id.clone())
        .collect();
    assert_eq!(ids, vec!["a1"]); // a2 answered, a3 expired, a4 cancelled
}

#[test]
fn open_carries_metadata() {
    let events = vec![ev(
        "a1",
        "w1",
        "agent-7",
        Op::Ask {
            subject: "deploy anyway?".into(),
            target: "human:luis".into(),
            options: vec!["yes".into(), "no".into()],
            ttl_secs: 60,
        },
        10,
    )];
    let o = &open(&events, 20)[0];
    assert_eq!(o.subject, "deploy anyway?");
    assert_eq!(o.target, "human:luis");
    assert_eq!(o.options, vec!["yes", "no"]);
    assert_eq!(o.expires, Some(70));
    assert_eq!(o.asked, 10);
    assert_eq!(o.ws, "w1");
    assert_eq!(o.actor, "agent-7");
}

#[test]
fn all_reports_every_state_in_order() {
    let events = vec![
        ask("a1", "w1", 10, 100),
        ask("a2", "w1", 11, 5),
        ask("a3", "w1", 12, 100),
        ev(
            "r1",
            "w1",
            "x",
            Op::Answer {
                ask: "a1".into(),
                verdict: "ok".into(),
            },
            20,
        ),
    ];
    let states: Vec<Status> = all(&events, 30)
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    assert_eq!(
        states,
        vec![
            Status::Answered {
                verdict: "ok".into(),
                by: "x".into()
            },
            Status::Expired,
            Status::Open
        ]
    );
}

#[test]
fn reask_same_log_order_stable() {
    // Two asks interleaved with unrelated ops fold independently.
    let events = vec![
        ask("a1", "w1", 10, 100),
        ev(
            "x1",
            "w1",
            "a",
            Op::Claim {
                glob: "src/**".into(),
                ttl_secs: 1,
            },
            11,
        ),
        ask("a2", "w2", 12, 100),
    ];
    let ids: Vec<String> = open(&events, 20)
        .iter()
        .map(|o| o.id.clone())
        .collect();
    assert_eq!(ids, vec!["a1", "a2"]);
}
