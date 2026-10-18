//! TOML impl of `ConfigSource` — explicit path → $STURN_CONFIG →
//! ./sturn.toml → ~/.config/sturn/sturn.toml.

use std::path::PathBuf;
use sturn_config::ConfigSource;

pub struct TomlSource {
    /// Explicit `--config` path, or None for the default search.
    pub path: Option<String>,
}

impl ConfigSource for TomlSource {
    fn load(&self) -> Result<sturn_core::Config, String> {
        let candidates: Vec<PathBuf> = match self
            .path
            .clone()
            .or_else(|| std::env::var("STURN_CONFIG").ok())
        {
            Some(p) => vec![PathBuf::from(p)],
            None => vec![
                PathBuf::from("sturn.toml"),
                std::env::var("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| ".".into())
                    .join(".config/sturn/sturn.toml"),
            ],
        };
        for p in candidates {
            if p.exists() {
                let text = std::fs::read_to_string(&p)
                    .map_err(|e| format!("read {}: {e}", p.display()))?;
                return toml::from_str(&text).map_err(|e| format!("parse {}: {e}", p.display()));
            }
        }
        Err("no sturn.toml found (search: $STURN_CONFIG, ., ~/.config/sturn/)".into())
    }
}
