//! Tests for the TOML ConfigSource: explicit-path load, the search
//! error, and the `sturn.toml` shape itself.

use sturn_config::ConfigSource;
use sturn_config_toml::TomlSource;

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sturn-toml-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn explicit_path_loads() {
    let dir = scratch("load");
    let path = dir.join("sturn.toml");
    std::fs::write(&path, "[repo]\ndir = \"/tmp/repo\"\n").unwrap();
    let cfg = TomlSource {
        path: Some(path.to_string_lossy().into_owned()),
    }
    .load()
    .unwrap();
    assert_eq!(cfg.repo.dir, std::path::PathBuf::from("/tmp/repo"));
    assert_eq!(cfg.repo.branch, "main");
    assert_eq!(cfg.repo.id(), "repo");
    assert_eq!(cfg.wal.ref_prefix, "refs/sturn/wal");
    assert_eq!(cfg.bus.poll_ms, 100);
}

#[test]
fn full_config_parses() {
    let dir = scratch("full");
    let path = dir.join("sturn.toml");
    std::fs::write(
        &path,
        r#"
[repo]
dir = "/x"
id = "9aia/sturn"
branch = "trunk"

[wal]
ref_prefix = "refs/custom"

[bus]
store = "/tmp/b.redb"
poll_ms = 42
repos = ["/a", "/b"]

[actor]
name = "bot"

[signing]
key = "~/.ssh/k"
allowed_signers = "/etc/signers"

[review]
require = true
"#,
    )
    .unwrap();
    let cfg = TomlSource {
        path: Some(path.to_string_lossy().into_owned()),
    }
    .load()
    .unwrap();
    assert_eq!(cfg.repo.id(), "9aia/sturn");
    assert_eq!(cfg.repo.branch, "trunk");
    assert_eq!(cfg.wal.ref_prefix, "refs/custom");
    assert_eq!(cfg.bus.poll_ms, 42);
    assert_eq!(cfg.bus.repos.len(), 2);
    assert_eq!(cfg.actor.name(), "bot");
    assert!(cfg.review.require);
    assert_eq!(cfg.signing.key.as_deref(), Some("~/.ssh/k"));
}

#[test]
fn unknown_field_rejected() {
    let dir = scratch("unknown");
    let path = dir.join("sturn.toml");
    std::fs::write(&path, "[repo]\ndir = \"/x\"\nbogus = 1\n").unwrap();
    let err = TomlSource {
        path: Some(path.to_string_lossy().into_owned()),
    }
    .load();
    assert!(err.unwrap_err().contains("parse"));
}

#[test]
fn missing_everywhere_errors() {
    let dir = scratch("missing");
    let path = dir.join("nope.toml");
    let err = TomlSource {
        path: Some(path.to_string_lossy().into_owned()),
    }
    .load()
    .unwrap_err();
    assert!(err.contains("no sturn.toml found"));
}

#[test]
fn malformed_toml_errors() {
    let dir = scratch("bad");
    let path = dir.join("sturn.toml");
    std::fs::write(&path, "not [valid toml\n").unwrap();
    let err = TomlSource {
        path: Some(path.to_string_lossy().into_owned()),
    }
    .load()
    .unwrap_err();
    assert!(err.contains("parse"));
}
