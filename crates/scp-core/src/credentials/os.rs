use super::{SecretBackend, SecretError, SecretValue};
use keyring::{Entry, Error};

/// Native persistent stores matching desktop: Credential Manager, Keychain,
/// or Linux persistent kernel-keyring/Secret Service. Calls may prompt or block;
/// hosts must arrange user interaction and bounded worker lifetimes before use.
#[derive(Default)]
pub struct OsSecretBackend;

impl SecretBackend for OsSecretBackend {
    fn read(&mut self, namespace: &str, account: &str) -> Result<Option<SecretValue>, SecretError> {
        read_entry(&Entry::new(namespace, account).map_err(SecretError::from)?)
    }

    fn write(
        &mut self,
        namespace: &str,
        account: &str,
        value: Option<&str>,
    ) -> Result<(), SecretError> {
        write_entry(
            &Entry::new(namespace, account).map_err(SecretError::from)?,
            value,
        )
    }
}

fn read_entry(entry: &Entry) -> Result<Option<SecretValue>, SecretError> {
    let password = entry.get_password();
    if matches!(&password, Err(Error::NoEntry)) {
        return Ok(None);
    }
    password
        .map(|value| Some(SecretValue::new(value)))
        .map_err(Into::into)
}

fn write_entry(entry: &Entry, value: Option<&str>) -> Result<(), SecretError> {
    if let Some(password) = value {
        return entry.set_password(password).map_err(Into::into);
    }
    let deletion = entry.delete_credential();
    if matches!(&deletion, Err(Error::NoEntry)) {
        return Ok(());
    }
    deletion.map_err(Into::into)
}

impl From<Error> for SecretError {
    fn from(error: Error) -> Self {
        // Native Display/Debug may contain secret bytes or credential details.
        // Retain the classification only; never stringify the underlying error.
        match error {
            Error::BadEncoding(_) => Self::InvalidData,
            Error::Ambiguous(_) => Self::Ambiguous,
            Error::Invalid(_, _) | Error::TooLong(_, _) => Self::InvalidAccount,
            _ => Self::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_adapter_preserves_missing_empty_value_and_idempotent_delete() {
        let entry = Entry::new_with_credential(Box::new(keyring::mock::MockCredential::default()));
        assert!(read_entry(&entry).unwrap().is_none());
        write_entry(&entry, Some("")).unwrap();
        assert_eq!(read_entry(&entry).unwrap().unwrap().expose(), "");
        write_entry(&entry, Some("test-value")).unwrap();
        assert_eq!(read_entry(&entry).unwrap().unwrap().expose(), "test-value");
        write_entry(&entry, None).unwrap();
        write_entry(&entry, None).unwrap();
        assert!(read_entry(&entry).unwrap().is_none());
    }

    #[test]
    fn entry_adapter_does_not_hide_store_read_write_or_delete_failures() {
        let entry = Entry::new_with_credential(Box::new(keyring::mock::MockCredential::default()));
        let mock = entry
            .get_credential()
            .downcast_ref::<keyring::mock::MockCredential>()
            .unwrap();
        mock.set_error(Error::NoStorageAccess(Box::new(std::io::Error::other(
            "locked",
        ))));
        assert_eq!(read_entry(&entry).unwrap_err(), SecretError::Unavailable);
        mock.set_error(Error::PlatformFailure(Box::new(std::io::Error::other(
            "failed",
        ))));
        assert_eq!(
            write_entry(&entry, Some("test-value")),
            Err(SecretError::Unavailable)
        );
        mock.set_error(Error::NoStorageAccess(Box::new(std::io::Error::other(
            "locked",
        ))));
        assert_eq!(write_entry(&entry, None), Err(SecretError::Unavailable));
    }
}
