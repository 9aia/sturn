//! Tests for the secret-tool Secrets impl — lookups of missing
//! names must miss cleanly (headless or not).

use sturn_keyring::KeyringSecrets;
use sturn_secret::Secrets;

#[test]
fn default_service_name() {
    assert_eq!(KeyringSecrets::default().service, "sturn");
}

#[test]
fn get_missing_name_returns_none() {
    let s = KeyringSecrets {
        service: "sturn-test".into(),
    };
    // Whether secret-tool exists or not, a name that was never
    // stored resolves to None — the chain falls through.
    assert_eq!(s.get("sturn-test-definitely-missing-name"), None);
}

// ---------- coverage extensions ----------
// PATH stub lives in ONE test to avoid env races.

#[test]
fn secret_tool_stub_paths() {
    let d = std::env::temp_dir().join(format!("sturn-keyring-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let log = d.join("seen");
    // lookup prints a value only for the name "present"; store
    // records stdin unless told to fail (name "fail").
    std::fs::write(
        d.join("secret-tool"),
        b"#!/bin/sh\n\
          if [ \"$1\" = lookup ]; then\n\
            for a in \"$@\"; do [ \"$a\" = present ] && echo the-secret && exit 0; done\n\
            for a in \"$@\"; do [ \"$a\" = empty ] && echo \"\" && exit 0; done\n\
            exit 1\n\
          fi\n\
          for a in \"$@\"; do [ \"$a\" = fail ] && exit 2; done\n\
          cat > \"$SECRET_LOG\"\nexit 0\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(d.join("secret-tool")).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(d.join("secret-tool"), p).unwrap();
    }
    let old_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var(
        "PATH",
        format!("{}:{old_path}", d.display()),
    );
    std::env::set_var("SECRET_LOG", &log);
    let s = KeyringSecrets {
        service: "svc".into(),
    };
    assert_eq!(s.get("present").as_deref(), Some("the-secret"));
    assert_eq!(s.get("empty"), None); // empty stdout → None
    assert_eq!(s.get("missing"), None); // exit 1 → None
    s.set("k", "v1").unwrap();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "v1");
    // Store failure propagates.
    assert!(s.set("fail", "x").is_err());
    // No secret-tool on PATH at all → clean miss.
    std::env::set_var("PATH", d.join("empty-bin"));
    assert_eq!(s.get("present"), None);
    std::env::set_var("PATH", old_path);
}
