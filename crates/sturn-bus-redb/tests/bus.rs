//! Tests for the redb Bus impl: publish/poll cursors and topic
//! listing against a real embedded store.

use sturn_bus::Bus;
use sturn_bus_redb::RedbBus;

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sturn-bus-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn publish_assigns_monotonic_seq() {
    let bus = RedbBus::open(scratch("seq").join("b.redb")).unwrap();
    assert_eq!(bus.publish("r", "ops", b"a").unwrap(), 0);
    assert_eq!(bus.publish("r", "ops", b"b").unwrap(), 1);
    assert_eq!(bus.publish("r", "ops", b"c").unwrap(), 2);
    // A different topic has its own sequence.
    assert_eq!(bus.publish("r", "claims", b"x").unwrap(), 0);
}

#[test]
fn poll_returns_after_cursor() {
    let bus = RedbBus::open(scratch("poll").join("b.redb")).unwrap();
    bus.publish("r", "ops", b"one").unwrap();
    bus.publish("r", "ops", b"two").unwrap();
    bus.publish("r", "ops", b"three").unwrap();

    // Fresh consumer: u64::MAX wraps to seq 0 — every entry arrives.
    let mut cursor = u64::MAX;
    let first = bus.poll("r", "ops", &mut cursor).unwrap();
    assert_eq!(
        first,
        vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
    );
    assert_eq!(cursor, 2);

    // Resuming after the tail sees nothing new until a publish.
    assert!(bus.poll("r", "ops", &mut cursor).unwrap().is_empty());
    bus.publish("r", "ops", b"four").unwrap();
    let more = bus.poll("r", "ops", &mut cursor).unwrap();
    assert_eq!(more, vec![b"four".to_vec()]);
    assert_eq!(cursor, 3);

    // Cursor = a seen seq resumes *after* it (skips seq 0).
    let mut c = 0;
    let rest = bus.poll("r", "ops", &mut c).unwrap();
    assert_eq!(
        rest,
        vec![b"two".to_vec(), b"three".to_vec(), b"four".to_vec()]
    );
}

#[test]
fn poll_empty_topic_is_empty() {
    let bus = RedbBus::open(scratch("empty").join("b.redb")).unwrap();
    let mut cursor = u64::MAX;
    assert!(bus.poll("r", "nope", &mut cursor).unwrap().is_empty());
}

#[test]
fn topics_lists_seen_topics_sorted() {
    let bus = RedbBus::open(scratch("topics").join("b.redb")).unwrap();
    bus.publish("r", "ops", b"1").unwrap();
    bus.publish("r", "claims", b"2").unwrap();
    bus.publish("r", "ops", b"3").unwrap();
    bus.publish("other", "ops", b"9").unwrap();
    assert_eq!(bus.topics("r").unwrap(), vec!["claims", "ops"]);
    assert_eq!(bus.topics("other").unwrap(), vec!["ops"]);
    assert_eq!(bus.topics("none").unwrap(), Vec::<String>::new());
}

// ---------- coverage extensions ----------

#[test]
fn poll_resume_and_topic_isolation() {
    let d = std::env::temp_dir().join(format!("sturn-bus-{}-resume", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let bus = RedbBus::open(d.join("bus.redb")).unwrap();
    bus.publish("r", "a", b"1").unwrap();
    bus.publish("r", "a", b"2").unwrap();
    bus.publish("r", "b", b"x").unwrap();

    // Fresh-consumer sentinel returns everything.
    let mut c = u64::MAX;
    assert_eq!(bus.poll("r", "a", &mut c).unwrap().len(), 2);
    // Resuming after the cursor returns only new entries — and the
    // scan breaks at the next topic's key.
    bus.publish("r", "a", b"3").unwrap();
    let got = bus.poll("r", "a", &mut c).unwrap();
    assert_eq!(got, vec![b"3".to_vec()]);
    // Explicit small cursor resumes after it.
    let mut c0 = 0;
    let got = bus.poll("r", "a", &mut c0).unwrap();
    assert_eq!(got, vec![b"2".to_vec(), b"3".to_vec()]);
    // Topics list isolates per repo.
    let mut topics = bus.topics("r").unwrap();
    topics.sort();
    assert_eq!(topics, vec!["a", "b"]);
    assert!(bus.topics("other").unwrap().is_empty());
    // Poll an empty topic moves nothing.
    let mut c2 = 7;
    assert!(bus.poll("r", "zzz", &mut c2).unwrap().is_empty());
}
