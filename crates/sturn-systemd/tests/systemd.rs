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

// ---------- coverage extensions ----------
// All PATH manipulation lives in ONE test so nothing else in this
// binary races the process-wide env.

#[test]
fn systemctl_and_journalctl_stubs() {
    let stub = scratch("sbin");
    let home = scratch("home2");

    // Before stubs land on PATH, the probe degrades to "unknown".
    let empty = scratch("empty-bin");
    let old_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", &empty);
    let s = sturn_systemd::unit_status("u.service").unwrap();
    assert!(s.contains("unknown"), "{s}");

    // Stub systemctl (success) + journalctl (echo args, fail on -f).
    std::fs::write(stub.join("systemctl"), b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(
        stub.join("journalctl"),
        b"#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = \"-f\" ] && exit 3; done\necho tail\nexit 0\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for n in ["systemctl", "journalctl"] {
            let mut p = std::fs::metadata(stub.join(n)).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(stub.join(n), p).unwrap();
        }
    }
    std::env::set_var("PATH", format!("{}:{old_path}", stub.display()));

    // Every run() wrapper goes through the stub.
    sturn_systemd::enable_service("u").unwrap();
    sturn_systemd::stop_service("u").unwrap();
    sturn_systemd::restart_service("u").unwrap();
    // unit_status now returns the stub's (empty) states.
    let s = sturn_systemd::unit_status("u").unwrap();
    assert!(s.starts_with("enabled="), "{s}");
    // service_logs returns journalctl's exit code, -f or not.
    assert_eq!(sturn_systemd::service_logs("u", false).unwrap(), 0);
    assert_eq!(sturn_systemd::service_logs("u", true).unwrap(), 3);

    // disable_service with and without a unit file present.
    std::env::set_var("HOME", &home);
    let unit_dir = home.join(".config/systemd/user");
    std::fs::create_dir_all(&unit_dir).unwrap();
    std::fs::write(unit_dir.join("u.service"), b"x").unwrap();
    let removed = sturn_systemd::disable_service("u.service").unwrap();
    assert!(!removed.exists());
    let missing = sturn_systemd::disable_service("absent.service").unwrap();
    assert!(!missing.exists());

    // The Service port impl delegates to the same helpers.
    use sturn_service::Service;
    let svc = sturn_systemd::SystemdService {
        unit: "u.service".into(),
        bin: "/bin/sturn".into(),
        env: "/env".into(),
        config: None,
    };
    svc.enable().unwrap();
    svc.disable().unwrap();

    // A failing stub → run() surfaces an io error.
    std::fs::write(stub.join("systemctl"), b"#!/bin/sh\nexit 9\n").unwrap();
    let err = sturn_systemd::enable_service("u").unwrap_err();
    assert!(err.to_string().contains("systemctl"), "{err}");
    // daemon-reload is best-effort inside disable; enable path
    // covers the error.

    // The unit file exists as a *directory* → the render's write
    // fails through the `?`.
    let home3 = scratch("home3");
    std::env::set_var("HOME", &home3);
    std::fs::create_dir_all(
        home3.join(".config/systemd/user/sturn-daemon.service"),
    )
    .unwrap();
    assert!(write_service_unit(
        PathBuf::from("/bin/sturn").as_path(),
        PathBuf::from("/env").as_path(),
        None,
    )
    .is_err());
    // env dir exists but is read-only → the skeleton write fails.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let cfgdir = scratch("envdir");
        let mut p = std::fs::metadata(&cfgdir).unwrap().permissions();
        p.set_mode(0o500);
        std::fs::set_permissions(&cfgdir, p).unwrap();
        assert!(write_env_skeleton(&cfgdir).is_err());
        let mut p = std::fs::metadata(&cfgdir).unwrap().permissions();
        p.set_mode(0o700);
        std::fs::set_permissions(&cfgdir, p).unwrap();
    }
    std::env::set_var("PATH", old_path);
}
