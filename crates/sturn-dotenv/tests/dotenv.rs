//! Tests for the dotenv Secrets impl: KEY=VAL reads and writes,
//! including the 0600 permission floor.

use sturn_dotenv::DotenvSecrets;
use sturn_secret::Secrets;

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sturn-dotenv-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn get_reads_key() {
    let dir = scratch("get");
    let path = dir.join("env");
    std::fs::write(&path, "A=1\nB= two \n\nC\n# comment\n").unwrap();
    let s = DotenvSecrets { path };
    assert_eq!(s.get("A").as_deref(), Some("1"));
    assert_eq!(s.get("B").as_deref(), Some("two"));
}

#[test]
fn get_missing_returns_none() {
    let dir = scratch("miss");
    let path = dir.join("env");
    std::fs::write(&path, "A=1\nEMPTY=\n").unwrap();
    let s = DotenvSecrets { path };
    assert_eq!(s.get("ZZZ"), None);
    // Empty values miss too — the caller falls through the chain.
    assert_eq!(s.get("EMPTY"), None);
}

#[test]
fn get_missing_file_returns_none() {
    let s = DotenvSecrets {
        path: scratch("nofile").join("env"),
    };
    assert_eq!(s.get("A"), None);
}

#[test]
fn set_creates_and_reads_back() {
    let dir = scratch("set");
    let path = dir.join("nested").join("env");
    let s = DotenvSecrets { path: path.clone() };
    s.set("TOKEN", "abc").unwrap();
    assert_eq!(s.get("TOKEN").as_deref(), Some("abc"));
    // Parent dirs were created.
    assert!(path.exists());
}

#[test]
fn set_updates_existing_key() {
    let dir = scratch("update");
    let path = dir.join("env");
    std::fs::write(&path, "K=old\nOTHER=x\n").unwrap();
    let s = DotenvSecrets { path: path.clone() };
    s.set("K", "new").unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("K=new"));
    assert!(text.contains("OTHER=x"));
    assert!(!text.contains("K=old"));
}

#[cfg(unix)]
#[test]
fn set_writes_owner_only_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("mode");
    let path = dir.join("env");
    let s = DotenvSecrets { path: path.clone() };
    s.set("K", "v").unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}
