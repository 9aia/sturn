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
