//! Service port — OS-level service lifecycle for the daemon.
//! systemd impl in `sturn-systemd`; launchd/Windows = new impls.

use std::io;
use std::path::PathBuf;

/// Install/remove a managed service so `daemon` survives reboots.
pub trait Service: Send + Sync {
    /// Write the unit/config file; returns where it landed.
    fn write(&self) -> io::Result<PathBuf>;
    /// Enable + start now.
    fn enable(&self) -> io::Result<()>;
    /// Disable + stop + remove the file.
    fn disable(&self) -> io::Result<PathBuf>;
}
