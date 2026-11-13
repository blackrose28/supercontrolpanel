use std::collections::{BTreeMap, BTreeSet};

type Credentials = BTreeMap<String, Option<String>>;

fn expected() -> BTreeSet<String> {
    [
        "openvpn_password",
        "openvpn_otp_secret",
        "boundary_password",
        "boundary_oidc_password",
        "boundary_oidc_otp_secret",
        "boundary_app_token",
        "cc-telemetry-queue-key",
        "server_ssh_password:node:one",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn credentials() -> Credentials {
    expected()
        .into_iter()
        .map(|account| (account, None))
        .collect()
}

#[test]
fn exact_inventory_rejects_missing_extra_replaced_and_case_changed_accounts() {
    for account in expected() {
        let mut value = credentials();
        value.remove(&account);
        assert_eq!(
            super::validate_credentials(&value, &expected()),
            Err(
                "Backup credential inventory does not match known app accounts and server IDs"
                    .into()
            )
        );
        value.insert(account.to_uppercase(), None);
        assert_eq!(
            super::validate_credentials(&value, &expected()),
            Err(
                "Backup credential inventory does not match known app accounts and server IDs"
                    .into()
            )
        );
    }
    let mut value = credentials();
    value.insert("private-unknown-account".into(), None);
    assert_eq!(
        super::validate_credentials(&value, &expected()),
        Err("Backup credential inventory does not match known app accounts and server IDs".into())
    );
    assert_eq!(
        super::validate_credentials(&Credentials::new(), &expected()),
        Err("Backup credential inventory does not match known app accounts and server IDs".into())
    );
}

#[test]
fn absence_and_present_empty_values_are_valid_and_preserved() {
    let mut value = credentials();
    assert_eq!(super::validate_credentials(&value, &expected()), Ok(()));
    for account in expected() {
        value.insert(account, Some(String::new()));
        let original = value.clone();
        assert_eq!(super::validate_credentials(&value, &expected()), Ok(()));
        assert_eq!(value, original);
    }
}

#[test]
fn credential_limit_is_inclusive_utf8_bytes_for_every_account() {
    for account in expected() {
        for (text, valid) in [
            ("x".repeat(65536), true),
            ("x".repeat(65537), false),
            ("é".repeat(32768), true),
            (format!("{}x", "é".repeat(32768)), false),
            ("🦀".repeat(16384), true),
            ("🦀".repeat(16385), false),
        ] {
            let mut value = credentials();
            value.insert(account.clone(), Some(text));
            let original = value.clone();
            let result = super::validate_credentials(&value, &expected());
            assert_eq!(
                result,
                if valid {
                    Ok(())
                } else {
                    Err("Backup credential is too large".into())
                }
            );
            assert_eq!(value, original);
        }
    }
}

#[test]
fn inventory_errors_precede_oversized_values_without_echoing_inputs() {
    let mut value = credentials();
    value.insert(
        "openvpn_password".into(),
        Some("SYNTHETIC_PRIVATE".repeat(65536)),
    );
    value.remove("boundary_password");
    assert_eq!(
        super::validate_credentials(&value, &expected()),
        Err("Backup credential inventory does not match known app accounts and server IDs".into())
    );
    value.insert("boundary_password".into(), None);
    value.insert(
        "private-unknown-account".into(),
        Some("SYNTHETIC_PRIVATE".repeat(65536)),
    );
    assert_eq!(
        super::validate_credentials(&value, &expected()),
        Err("Backup credential inventory does not match known app accounts and server IDs".into())
    );
    value.remove("private-unknown-account");
    assert_eq!(
        super::validate_credentials(&value, &expected()),
        Err("Backup credential is too large".into())
    );
}
