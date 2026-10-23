//! Tests for the systemd helpers that don't need a running systemd:
//! the env skeleton, the unit-file render (under a redirected HOME),
//! and the always-succeeds status line.

use std::path::PathBuf;
use sturn_systemd::{unit_status, write_env_skeleton, write_service_unit};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sturn-sd-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn env_skeleton_writes_template_once() {
    let dir = scratch("env");
    let path = write_env_skeleton(&dir).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("STURN_ACTOR="));
    // Second call keeps existing contents (never clobbers secrets).
    std::fs::write(&path, "STURN_ACTOR=me\n").unwrap();
    write_env_skeleton(&dir).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "STURN_ACTOR=me\n");
}

#[cfg(unix)]
#[test]
fn env_skeleton_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("mode");
    let path = write_env_skeleton(&dir).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

// write_service_unit renders into $HOME/.config/systemd/user — run
// it against a scratch HOME. Single test touching the env so nothing
// else in this binary races it.
#[test]
fn service_unit_renders() {
    let home = scratch("home");
    // Test-binary-local env; no other test here reads HOME.
    std::env::set_var("HOME", &home);
    let unit = write_service_unit(
        PathBuf::from("/usr/bin/sturn").as_path(),
        PathBuf::from("/home/x/.config/sturn/env").as_path(),
        Some(PathBuf::from("/etc/sturn.toml").as_path()),
    )
    .unwrap();
    let text = std::fs::read_to_string(&unit).unwrap();
    assert!(text.contains("ExecStart=/usr/bin/sturn daemon --config /etc/sturn.toml"));
    assert!(text.contains("EnvironmentFile=/home/x/.config/sturn/env"));
    assert!(text.contains("WantedBy=default.target"));

    // Same render through the Service port impl.
    let svc = sturn_systemd::SystemdService {
        unit: "sturn-daemon.service".into(),
        bin: "/bin/sturn".into(),
        env: "/env".into(),
        config: None,
    };
    use sturn_service::Service;
    let plain = svc.write().unwrap();
    let text = std::fs::read_to_string(&plain).unwrap();
    assert!(text.contains("ExecStart=/bin/sturn daemon\n"));
}

#[test]
fn unit_status_never_throws() {
    // With or without a user systemd, the probe degrades to
    // "enabled=<state> active=<state>" — possibly "unknown".
    let s = unit_status("sturn-daemon.service").unwrap();
    assert!(s.starts_with("enabled="), "{s}");
    assert!(s.contains(" active="), "{s}");
}
