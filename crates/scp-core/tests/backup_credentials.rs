use scp_core::settings_backup::validate_credentials;

#[path = "contracts/backup_credentials.rs"]
mod contracts;

#[test]
fn caller_supplies_inventory_without_account_mapping_or_keyring_policy() {
    use std::collections::{BTreeMap, BTreeSet};
    let credentials = BTreeMap::from([("opaque:caller-account".into(), None)]);
    let expected = BTreeSet::from(["opaque:caller-account".into()]);
    assert_eq!(validate_credentials(&credentials, &expected), Ok(()));
    assert_eq!(
        validate_credentials(&BTreeMap::new(), &BTreeSet::new()),
        Ok(())
    );
}
