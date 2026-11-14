//! Idempotent undo journal. Key lives in a dedicated OS account outside the backup set.
use super::{crypto, io, Changes, SecretStore, CONFIGURED, FILES};
use crate::models::KEYRING_BOUNDARY_APP_TOKEN;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};
use zeroize::Zeroizing;

pub const JOURNAL_ACCOUNT: &str = "settings-restore-recovery-v1";
pub const JOURNAL_FILE: &str = "settings-restore-journal.enc";
pub const DONE: &[u8] = b"SCP-RESTORE-DONE-1";
const AAD: &[u8] = b"SCP-RESTORE-JOURNAL-1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    committed: bool,
    files: BTreeMap<String, Option<Vec<u8>>>,
    credentials: BTreeMap<String, Option<String>>,
}

fn read_key(store: &mut impl SecretStore) -> Result<Zeroizing<[u8; 32]>, String> {
    let encoded = Zeroizing::new(store.read(JOURNAL_ACCOUNT)?.ok_or(
        "Recovery key missing. Keep the journal and restore OS-keyring access before retrying startup.",
    )?);
    if encoded.len() != 64 {
        return Err("Invalid settings recovery key".into());
    }
    let mut key = Zeroizing::new([0u8; 32]);
    for (index, part) in encoded.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(part).map_err(|_| "Invalid recovery key")?;
        key[index] = u8::from_str_radix(text, 16).map_err(|_| "Invalid recovery key")?;
    }
    Ok(key)
}

fn save_journal(
    dir: &Path,
    journal: &Journal,
    key: &[u8; 32],
    overwrite: bool,
) -> Result<(), String> {
    let plaintext = Zeroizing::new(
        serde_json::to_vec(journal).map_err(|_| "Could not encode recovery journal")?,
    );
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let mut bytes = nonce.to_vec();
    bytes.extend(crypto::seal(key, &nonce, &plaintext, AAD)?);
    if bytes.len() > crypto::MAX_BYTES {
        return Err("Recovery journal is too large".into());
    }
    io::atomic_write(&dir.join(JOURNAL_FILE), &bytes, overwrite)
}

pub fn prepare(dir: &Path, changes: &Changes, store: &mut impl SecretStore) -> Result<(), String> {
    let existing = io::read_bounded(&dir.join(JOURNAL_FILE))?;
    if existing.as_deref().is_some_and(|bytes| bytes != DONE) {
        return Err("Pending restore must be recovered before another import".into());
    }
    let mut journal = Journal {
        version: 1,
        committed: false,
        files: BTreeMap::new(),
        credentials: BTreeMap::new(),
    };
    for name in changes.files.keys() {
        if !FILES.contains(&name.as_str()) {
            return Err("Invalid restore filename".into());
        }
        journal
            .files
            .insert(name.clone(), io::read_bounded(&dir.join(name))?);
    }
    for account in changes.credentials.keys() {
        if !allowed_recovery_account(account) {
            return Err("Invalid restore credential account".into());
        }
        journal
            .credentials
            .insert(account.clone(), store.read(account)?);
    }
    let mut key = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(key.as_mut());
    let encoded = Zeroizing::new(key.iter().map(|b| format!("{b:02x}")).collect::<String>());
    store.write(JOURNAL_ACCOUNT, Some(&encoded))?;
    if *read_key(store)? != *key {
        return Err("Recovery key was not durably persisted; import aborted".into());
    }
    save_journal(dir, &journal, &key, existing.is_some())
}

fn allowed_recovery_account(account: &str) -> bool {
    (CONFIGURED.contains(&account) || account == KEYRING_BOUNDARY_APP_TOKEN)
        || account
            .strip_prefix("server_ssh_password:")
            .is_some_and(|id| {
                !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
            })
}

type LoadedJournal = Option<(Journal, Zeroizing<[u8; 32]>)>;

fn load_journal(dir: &Path, store: &mut impl SecretStore) -> Result<LoadedJournal, String> {
    let Some(bytes) = io::read_bounded(&dir.join(JOURNAL_FILE))? else {
        return Ok(None);
    };
    if bytes == DONE {
        return Ok(None);
    }
    let key = read_key(store)?;
    if bytes.len() < 28 {
        return Err("Damaged settings recovery journal; keep the journal and recovery key".into());
    }
    let nonce: &[u8; 12] = bytes[..12].try_into().unwrap();
    let plain = crypto::open(&key, nonce, &bytes[12..], AAD)?;
    let journal: Journal =
        serde_json::from_slice(&plain).map_err(|_| "Invalid settings recovery journal")?;
    if journal.version != 1
        || journal
            .files
            .keys()
            .any(|name| !FILES.contains(&name.as_str()))
        || journal
            .credentials
            .keys()
            .any(|a| !allowed_recovery_account(a))
    {
        return Err("Unsupported settings recovery journal".into());
    }
    Ok(Some((journal, key)))
}

pub fn commit(dir: &Path, changes: &Changes, store: &mut impl SecretStore) -> Result<(), String> {
    let staged: BTreeMap<_, _> = changes
        .files
        .iter()
        .map(|(name, value)| {
            value
                .as_ref()
                .map(serde_json::to_vec_pretty)
                .transpose()
                .map(|bytes| (name.clone(), bytes.map(Zeroizing::new)))
                .map_err(|_| "Could not stage settings".to_string())
        })
        .collect::<Result<_, _>>()?;
    prepare(dir, changes, store)?;
    let apply = (|| {
        for (account, value) in &changes.credentials {
            store.write(account, value.as_deref())?;
        }
        for (name, value) in &staged {
            restore_file(dir, name, value.as_ref().map(|bytes| bytes.as_slice()))?;
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = apply {
        return match recover(dir, store) {
            Ok(()) => Err(format!("Import failed and was rolled back: {error}")),
            Err(_) => Err("Import and rollback failed. Settings are in maintenance state. Keep settings-restore-journal.enc and the OS recovery key, unlock the keyring, then restart to retry recovery.".into()),
        };
    }
    mark_committed(dir, store)?;
    cleanup(dir, store)
}

pub fn mark_committed(dir: &Path, store: &mut impl SecretStore) -> Result<(), String> {
    let (mut journal, key) =
        load_journal(dir, store)?.ok_or("Missing recovery journal after import")?;
    journal.committed = true;
    save_journal(dir, &journal, &key, true)
}

fn restore_file(dir: &Path, name: &str, bytes: Option<&[u8]>) -> Result<(), String> {
    let path = dir.join(name);
    match bytes {
        Some(bytes) => io::atomic_write(&path, bytes, true),
        None => match fs::remove_file(&path) {
            Ok(()) => io::sync_directory(dir),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Could not remove settings file during restore".into()),
        },
    }
}

pub fn recover(dir: &Path, store: &mut impl SecretStore) -> Result<(), String> {
    let Some((journal, _key)) = load_journal(dir, store)? else {
        return Ok(());
    };
    if !journal.committed {
        for (account, value) in &journal.credentials {
            store.write(account, value.as_deref())?;
        }
        for (name, value) in &journal.files {
            restore_file(dir, name, value.as_deref())?;
        }
    }
    cleanup(dir, store)
}

fn cleanup(dir: &Path, store: &mut impl SecretStore) -> Result<(), String> {
    io::atomic_write(&dir.join(JOURNAL_FILE), DONE, true)?;
    fs::remove_file(dir.join(JOURNAL_FILE)).map_err(|_| {
        "Settings committed/recovered, but journal cleanup failed; retry startup recovery"
    })?;
    io::sync_directory(dir)?;
    store
        .write(JOURNAL_ACCOUNT, None)
        .map_err(|_| "Settings committed/recovered; unused recovery key cleanup failed".to_string())
}

impl Drop for Journal {
    fn drop(&mut self) {
        for buf in self.files.values_mut().flatten() {
            zeroize::Zeroize::zeroize(buf);
        }
        for val in self.credentials.values_mut().flatten() {
            zeroize::Zeroize::zeroize(val);
        }
    }
}
