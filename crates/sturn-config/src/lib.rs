//! Config port + file impl — `sturn.{yaml,yml,toml}` loading through
//! scarab-config's file discovery (P-017): explicit `--config` →
//! `$STURN_CONFIG` → `./sturn.*` (YAML preferred) →
//! `~/.config/sturn/`. The generic machinery is scarab's
//! (`scarab-config-file`); this crate is only the wiring plus the
//! [`ConfigSource`] port front-ends bind. The retired
//! `sturn-config-toml` crate's hand-rolled search lives here now.

use std::path::PathBuf;

use scarab_config_file::FileSource as ScarabSource;

/// A config source — returns the loaded `Config`.
pub trait ConfigSource: Send + Sync {
    /// Load + validate the config; Err carries a readable message.
    fn load(&self) -> Result<sturn_core::Config, String>;
}

/// The file-backed source — explicit `--config` path wins over
/// `$STURN_CONFIG`, then the conventional discovery.
pub struct FileSource {
    /// Explicit `--config` path, or None for the default search.
    pub path: Option<String>,
}

impl FileSource {
    /// The underlying scarab source — `for_app("sturn")`
    /// conventions (`$STURN_CONFIG`, `./`, `~/.config/sturn/`) with
    /// the standard YAML-preferred formats.
    fn source(&self) -> ScarabSource<sturn_core::Config> {
        let mut src = ScarabSource::for_app("sturn");
        src.explicit = self.path.clone();
        src
    }

    /// Candidate config paths in search order — for status/hint
    /// views that report which file the chain would read. The
    /// explicit/env path heads the list even when missing (the
    /// loader turns that into an error, not a skip).
    #[must_use]
    pub fn candidates(&self) -> Vec<PathBuf> {
        self.source().candidates_with(&|k| std::env::var(k).ok())
    }
}

impl ConfigSource for FileSource {
    fn load(&self) -> Result<sturn_core::Config, String> {
        self.source().load()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sturn-cfg-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn explicit_path_loads() {
        let dir = scratch("load");
        let path = dir.join("sturn.toml");
        std::fs::write(&path, "[repo]\ndir = \"/tmp/repo\"\n").unwrap();
        let cfg = FileSource {
            path: Some(path.to_string_lossy().into_owned()),
        }
        .load()
        .unwrap();
        assert_eq!(cfg.repo.dir, PathBuf::from("/tmp/repo"));
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
        let cfg = FileSource {
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
    fn yaml_config_loads() {
        let dir = scratch("yaml");
        let path = dir.join("sturn.yaml");
        std::fs::write(&path, "repo:\n  dir: /yaml/repo\n").unwrap();
        let cfg = FileSource {
            path: Some(path.to_string_lossy().into_owned()),
        }
        .load()
        .unwrap();
        assert_eq!(cfg.repo.dir, PathBuf::from("/yaml/repo"));
    }

    #[test]
    fn unknown_field_rejected() {
        let dir = scratch("unknown");
        let path = dir.join("sturn.toml");
        std::fs::write(&path, "[repo]\ndir = \"/x\"\nbogus = 1\n").unwrap();
        let err = FileSource {
            path: Some(path.to_string_lossy().into_owned()),
        }
        .load();
        assert!(err.unwrap_err().contains("parse"));
    }

    #[test]
    fn missing_everywhere_errors() {
        let dir = scratch("missing");
        let path = dir.join("nope.toml");
        let err = FileSource {
            path: Some(path.to_string_lossy().into_owned()),
        }
        .load()
        .unwrap_err();
        assert!(err.contains("config file not found"), "{err}");
    }

    #[test]
    fn malformed_toml_errors() {
        let dir = scratch("bad");
        let path = dir.join("sturn.toml");
        std::fs::write(&path, "not [valid toml\n").unwrap();
        let err = FileSource {
            path: Some(path.to_string_lossy().into_owned()),
        }
        .load()
        .unwrap_err();
        assert!(err.contains("parse"), "{err}");
    }

    #[test]
    fn env_var_drives_search() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch("env");
        let good = dir.join("good.toml");
        std::fs::write(&good, "[repo]\ndir = \".\"\n").unwrap();
        let saved = std::env::var("STURN_CONFIG").ok();
        // $STURN_CONFIG names the file when no explicit path.
        std::env::set_var("STURN_CONFIG", good.to_str().unwrap());
        let cfg = FileSource { path: None }.load().unwrap();
        assert_eq!(cfg.repo.dir, PathBuf::from("."));
        // A missing env path is an error, not a silent skip.
        std::env::set_var("STURN_CONFIG", dir.join("missing.toml"));
        let err = FileSource { path: None }.load().unwrap_err();
        assert!(err.contains("config file not found"), "{err}");
        match saved {
            Some(v) => std::env::set_var("STURN_CONFIG", v),
            None => std::env::remove_var("STURN_CONFIG"),
        }
    }

    #[test]
    fn cwd_dotfile_and_candidates() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch("cwd");
        std::fs::write(dir.join("sturn.toml"), "[repo]\ndir = \".\"\n").unwrap();
        let saved_cfg = std::env::var("STURN_CONFIG").ok();
        let saved_home = std::env::var("HOME").ok();
        std::env::remove_var("STURN_CONFIG");
        std::env::set_var("HOME", "/nonexistent-home");
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let cfg = FileSource { path: None }.load().unwrap();
        // candidates report the same chain the loader walks —
        // `./sturn.{yaml,yml,toml}` first, then the user dir.
        let c = FileSource { path: None }.candidates();
        std::env::set_current_dir(cwd).unwrap();
        match saved_cfg {
            Some(v) => std::env::set_var("STURN_CONFIG", v),
            None => std::env::remove_var("STURN_CONFIG"),
        }
        match saved_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        assert_eq!(cfg.repo.dir, PathBuf::from("."));
        assert!(c.iter().any(|p| p.ends_with("sturn.toml")));
        assert!(c.iter().any(|p| p.ends_with("sturn.yaml")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// env/cwd mutation is process-global — serialize the tests
    /// that touch them (plain `cargo test` shares the process;
    /// nextest already isolates per-test).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn minimal_config_uses_core_defaults() {
        let dir = scratch("defaults");
        let f = dir.join("sturn.toml");
        std::fs::write(&f, "[repo]\ndir = \"/x\"\n").unwrap();
        let cfg = FileSource {
            path: Some(f.to_str().unwrap().into()),
        }
        .load()
        .unwrap();
        assert_eq!(cfg.repo.branch, "main");
        assert_eq!(cfg.wal.ref_prefix, "refs/sturn/wal");
        assert_eq!(cfg.bus.poll_ms, 100);
        assert!(cfg.actor.name.is_none());
        assert!(!cfg.review.require);
    }

    #[test]
    fn explicit_path_is_a_file_check() {
        // a directory named like a config is not a candidate
        let dir = scratch("dirpath");
        let sub = dir.join("sturn.toml");
        std::fs::create_dir_all(&sub).unwrap();
        let err = FileSource {
            path: Some(sub.to_string_lossy().into_owned()),
        }
        .load()
        .unwrap_err();
        assert!(err.contains("config file not found"), "{err}");
    }
}
