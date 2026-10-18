//! Config port — where the `Config` value comes from.
//! Impl: `sturn-config-toml` (file); env/remote = new impls.

/// A config source — returns the loaded `Config`.
pub trait ConfigSource: Send + Sync {
    /// Load + validate the config; Err carries a readable message.
    fn load(&self) -> Result<sturn_core::Config, String>;
}
