//! Unit tests for sturn-core: overlay folding, snapshot indexing,
//! event signing bytes, and the op vocabulary's wire shape.
//! (Config TOML parsing is exercised in sturn-config-toml's tests —
//! that impl owns the `toml` dependency.)

use sturn_core::{overlay, since_last_snapshot, ActorCfg, Event, Op, Verdict};

fn ev(op: Op) -> Event {
    Event::new("ws1", "agent", op)
}

// --- since_last_snapshot ------------------------------------------------

#[test]
fn since_last_snapshot_empty() {
    assert_eq!(since_last_snapshot(&[]), 0);
}

#[test]
fn since_last_snapshot_none_present() {
    let events = vec![
        ev(Op::Intent { text: "a".into() }),
        ev(Op::Delete { path: "x".into() }),
    ];
    assert_eq!(since_last_snapshot(&events), 0);
}

#[test]
fn since_last_snapshot_points_after_marker() {
    let events = vec![
        ev(Op::Intent { text: "a".into() }),
        ev(Op::Snapshot {
            message: "m".into(),
        }),
        ev(Op::Delete { path: "x".into() }),
    ];
    assert_eq!(since_last_snapshot(&events), 2);
}

#[test]
fn since_last_snapshot_uses_last_marker() {
    let events = vec![
        ev(Op::Snapshot {
            message: "one".into(),
        }),
        ev(Op::Intent { text: "a".into() }),
        ev(Op::Snapshot {
            message: "two".into(),
        }),
    ];
    assert_eq!(since_last_snapshot(&events), 3);
}

// --- overlay ------------------------------------------------------------

#[test]
fn overlay_write_then_delete() {
    let events = vec![
        ev(Op::Write {
            path: "a".into(),
            blob: "b1".into(),
            intent: None,
        }),
        ev(Op::Write {
            path: "b".into(),
            blob: "b2".into(),
            intent: Some("why".into()),
        }),
        ev(Op::Delete { path: "a".into() }),
    ];
    let m = overlay(&events, |_| None);
    assert_eq!(m.get("a"), Some(&None));
    assert_eq!(m.get("b"), Some(&Some("b2".into())));
}

#[test]
fn overlay_rename_uses_overlay_blob() {
    let events = vec![
        ev(Op::Write {
            path: "old".into(),
            blob: "sha1".into(),
            intent: None,
        }),
        ev(Op::Rename {
            from: "old".into(),
            to: "new".into(),
        }),
    ];
    let m = overlay(&events, |_| None);
    assert_eq!(m.get("old"), Some(&None));
    assert_eq!(m.get("new"), Some(&Some("sha1".into())));
}

#[test]
fn overlay_rename_falls_back_to_resolve() {
    let events = vec![ev(Op::Rename {
        from: "in-tree".into(),
        to: "moved".into(),
    })];
    let m = overlay(&events, |p| (p == "in-tree").then(|| "base-sha".into()));
    assert_eq!(m.get("in-tree"), Some(&None));
    assert_eq!(m.get("moved"), Some(&Some("base-sha".into())));
}

#[test]
fn overlay_rename_missing_source_marks_delete_only() {
    let events = vec![ev(Op::Rename {
        from: "ghost".into(),
        to: "nowhere".into(),
    })];
    let m = overlay(&events, |_| None);
    assert_eq!(m.get("ghost"), Some(&None));
    assert!(!m.contains_key("nowhere"));
}

#[test]
fn overlay_ignores_non_tree_ops() {
    let events = vec![
        ev(Op::Claim {
            glob: "src/**".into(),
            ttl_secs: 60,
        }),
        ev(Op::Release {
            glob: "src/**".into(),
        }),
        ev(Op::Intent { text: "t".into() }),
        ev(Op::Snapshot {
            message: "s".into(),
        }),
        ev(Op::Review {
            event: "e".into(),
            verdict: Verdict::Approve,
        }),
        ev(Op::Presence {
            status: "joined".into(),
        }),
    ];
    assert!(overlay(&events, |_| None).is_empty());
}

// --- Event --------------------------------------------------------------

#[test]
fn event_new_assigns_fields() {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let e = Event::new("w", "me", Op::Intent { text: "x".into() });
    assert_eq!(e.ws, "w");
    assert_eq!(e.actor, "me");
    assert!(e.id.is_empty());
    assert!(e.sig.is_none());
    assert!(e.ts >= before);
}

#[test]
fn signing_bytes_clear_sig_and_roundtrip() {
    let mut e = ev(Op::Intent { text: "x".into() });
    e.sig = Some("signature".into());
    e.id = "id".into();
    let bytes = e.signing_bytes();
    let back: Event = serde_json::from_slice(&bytes).unwrap();
    assert!(back.sig.is_none());
    // id is derived (assigned by the wal *after* signing) — the
    // signed payload can't include it.
    assert!(back.id.is_empty());
    // Deterministic across calls.
    assert_eq!(bytes, e.signing_bytes());
}

// --- Op serde -----------------------------------------------------------

#[test]
fn op_serde_roundtrips_all_variants() {
    let ops = vec![
        Op::Write {
            path: "p".into(),
            blob: "b".into(),
            intent: Some("i".into()),
        },
        Op::Rename {
            from: "a".into(),
            to: "b".into(),
        },
        Op::Delete { path: "p".into() },
        Op::Claim {
            glob: "g".into(),
            ttl_secs: 5,
        },
        Op::Release { glob: "g".into() },
        Op::Intent { text: "t".into() },
        Op::Snapshot {
            message: "m".into(),
        },
        Op::Review {
            event: "e".into(),
            verdict: Verdict::Reject,
        },
        Op::Presence {
            status: "left".into(),
        },
    ];
    for op in ops {
        let json = serde_json::to_string(&op).unwrap();
        let back: Op = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }
}

#[test]
fn op_tag_is_snake_case() {
    let json = serde_json::to_value(Op::Snapshot {
        message: "m".into(),
    })
    .unwrap();
    assert_eq!(json["op"], "snapshot");
    let json = serde_json::to_value(Op::Write {
        path: "p".into(),
        blob: "b".into(),
        intent: None,
    })
    .unwrap();
    assert_eq!(json["op"], "write");
}

#[test]
fn claim_ttl_defaults_3600() {
    let op: Op = serde_json::from_str(r#"{"op":"claim","glob":"g"}"#).unwrap();
    match op {
        Op::Claim { ttl_secs, .. } => assert_eq!(ttl_secs, 3600),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn verdict_lowercase() {
    assert_eq!(
        serde_json::to_string(&Verdict::Approve).unwrap(),
        "\"approve\""
    );
    assert_eq!(
        serde_json::to_string(&Verdict::Reject).unwrap(),
        "\"reject\""
    );
}

// --- Config defaults (the parse itself lives in sturn-config-toml) ------

#[test]
fn config_defaults() {
    let cfg = sturn_core::WalCfg::default();
    assert_eq!(cfg.ref_prefix, "refs/sturn/wal");
    let bus = sturn_core::BusCfg::default();
    assert_eq!(bus.poll_ms, 100);
    assert!(bus.repos.is_empty());
    assert!(bus.store.ends_with("bus.redb"));
    assert!(!sturn_core::ReviewCfg::default().require);
    assert!(sturn_core::SigningCfg::default().key.is_none());
    assert!(sturn_core::SigningCfg::default().allowed_signers.is_none());
}

#[test]
fn repo_id_explicit_or_basename() {
    let explicit = sturn_core::RepoCfg {
        dir: "/x".into(),
        id: Some("9aia/sturn".into()),
        branch: "main".into(),
    };
    assert_eq!(explicit.id(), "9aia/sturn");
    let basename = sturn_core::RepoCfg {
        dir: "/a/b/c".into(),
        id: None,
        branch: "main".into(),
    };
    assert_eq!(basename.id(), "c");
    // Root-ish dirs still yield a name.
    let root = sturn_core::RepoCfg {
        dir: "/".into(),
        id: None,
        branch: "main".into(),
    };
    assert!(!root.id().is_empty());
}

#[test]
fn actor_name_prefers_explicit_then_env() {
    let explicit = ActorCfg {
        name: Some("set".into()),
    };
    assert_eq!(explicit.name(), "set");
    // Fallback resolves *something* non-empty in any environment.
    assert!(!ActorCfg::default().name().is_empty());
}

#[test]
fn topic_vocabulary() {
    use sturn_core::topic;
    assert_eq!(topic::OPS, "ops");
    assert_eq!(topic::CLAIMS, "claims");
    assert_eq!(topic::REVIEW, "review");
    assert_eq!(topic::PRESENCE, "presence");
    assert_eq!(topic::ASKS, "asks");
    assert_eq!(topic::ALL.len(), 5);
}

// ---------- coverage extensions ----------

#[test]
fn config_defaults_fill_every_section() {
    // Minimal config (sturn-core's serde defaults fill the rest;
    // the TOML text path is covered in sturn-config-toml's tests).
    let cfg: sturn_core::Config = serde_json::from_str(r#"{"repo":{"dir":"/x"}}"#).unwrap();
    assert_eq!(cfg.wal.ref_prefix, "refs/sturn/wal");
    assert_eq!(cfg.bus.poll_ms, 100);
    assert!(cfg.bus.store.ends_with(".local/state/sturn/bus.redb"));
    assert!(cfg.bus.repos.is_empty());
    assert!(cfg.actor.name.is_none());
    assert!(cfg.signing.key.is_none());
    assert!(cfg.signing.allowed_signers.is_none());
    assert!(!cfg.review.require);
    assert_eq!(cfg.repo.branch, "main");
}

#[test]
fn op_serde_edges() {
    // Claim without ttl_secs → default_ttl().
    let ev: Event = serde_json::from_str(
        r#"{"ws":"w","actor":"a","ts":1,"op":{"op":"claim","glob":"x"}}"#,
    )
    .unwrap();
    match ev.op {
        Op::Claim { ttl_secs, .. } => assert_eq!(ttl_secs, 3600),
        _ => panic!("expected claim"),
    }
    // Verdict reject round-trips lowercase.
    let v = serde_json::to_string(&Verdict::Reject).unwrap();
    assert_eq!(v, "\"reject\"");
    // Event with a sig serializes it; signing_bytes strips it.
    let mut e = Event::new("w", "a", Op::Intent { text: "t".into() });
    e.sig = Some("SIG".into());
    let j = serde_json::to_string(&e).unwrap();
    assert!(j.contains("\"sig\":\"SIG\""));
    let unsigned = serde_json::from_slice::<Event>(&e.signing_bytes()).unwrap();
    assert!(unsigned.sig.is_none());
    // No sig → field omitted entirely.
    let e2 = Event::new("w", "a", Op::Intent { text: "t".into() });
    assert!(!serde_json::to_string(&e2).unwrap().contains("sig"));
}

#[test]
fn overlay_rename_resolution() {
    // Rename whose `from` was overlaid earlier in the batch.
    let mut evs = vec![
        Event::new(
            "w",
            "a",
            Op::Write { path: "x".into(), blob: "b1".into(), intent: None },
        ),
        Event::new(
            "w",
            "a",
            Op::Rename { from: "x".into(), to: "y".into() },
        ),
    ];
    let ov = sturn_core::overlay(&evs, |_| None);
    assert_eq!(ov.get("y"), Some(&Some("b1".to_string())));
    assert_eq!(ov.get("x"), Some(&None));
    // Rename of a base-tree path → resolve() supplies the blob.
    let evs2 = vec![Event::new(
        "w",
        "a",
        Op::Rename { from: "old".into(), to: "new".into() },
    )];
    let ov = sturn_core::overlay(&evs2, |p| (p == "old").then(|| "b9".to_string()));
    assert_eq!(ov.get("new"), Some(&Some("b9".to_string())));
    // Rename with no resolvable blob → `to` gets nothing.
    let evs3 = vec![Event::new(
        "w",
        "a",
        Op::Rename { from: "ghost".into(), to: "t".into() },
    )];
    let ov = sturn_core::overlay(&evs3, |_| None);
    assert_eq!(ov.get("t"), None);
    evs.clear();
}
