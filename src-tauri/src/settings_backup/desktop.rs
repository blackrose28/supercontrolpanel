//! Narrow local Tauri surface. Passphrases never enter command logs or persisted previews.
use super::{
    compile_restore, crypto, gate, io, snapshot, transaction, Archive, RestoreOptions, SecretStore,
};
use crate::{
    mcp::{mcp_runtime_status, McpRuntimeState},
    services::config_service::*,
    stores::*,
};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

pub struct OsSecrets;
impl SecretStore for OsSecrets {
    fn read(&mut self, account: &str) -> Result<Option<String>, String> {
        keyring_load_from_os(account)
    }
    fn write(&mut self, account: &str, value: Option<&str>) -> Result<(), String> {
        let entry = keyring_entry(account)?;
        match value {
            Some(value) => entry
                .set_password(value)
                .map_err(|_| "OS credential store write failed".to_string()),
            None => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err("OS credential store deletion failed".into()),
            },
        }
    }
}

struct WorkRegistration {
    id: String,
    cancelled: Arc<AtomicBool>,
}
static ACTIVE_WORK: Mutex<Option<WorkRegistration>> = Mutex::new(None);
static KEYRING_READ_ACTIVE: AtomicBool = AtomicBool::new(false);

pub(super) struct BackupWork {
    id: String,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}
impl BackupWork {
    pub(super) fn begin(id: String) -> Result<Self, String> {
        if id.is_empty() || id.len() > 128 {
            return Err("Invalid backup request ID".into());
        }
        let mut active = ACTIVE_WORK
            .lock()
            .map_err(|_| "Backup worker unavailable")?;
        if active.is_some() {
            return Err("Another backup operation is still running".into());
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        *active = Some(WorkRegistration {
            id: id.clone(),
            cancelled: cancelled.clone(),
        });
        Ok(Self {
            id,
            cancelled,
            deadline: Instant::now() + Duration::from_secs(120),
        })
    }
    pub(super) fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Err("Backup operation cancelled".into());
        }
        if Instant::now() > self.deadline {
            return Err("Backup operation timed out; unlock the OS keyring and retry".into());
        }
        Ok(())
    }
}
impl Drop for BackupWork {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_WORK.lock() {
            if active.as_ref().is_some_and(|work| work.id == self.id) {
                *active = None;
            }
        }
    }
}
impl SecretStore for &BackupWork {
    fn read(&mut self, account: &str) -> Result<Option<String>, String> {
        self.check()?;
        if KEYRING_READ_ACTIVE.swap(true, Ordering::SeqCst) {
            return Err("A previous native credential-store read is still waiting; unlock the keyring before retrying".into());
        }
        let account = account.to_string();
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = keyring_load_from_os(&account);
            KEYRING_READ_ACTIVE.store(false, Ordering::SeqCst);
            if let Err(unsent) = send.send(result) {
                if let Ok(Some(mut secret)) = unsent.0 {
                    use zeroize::Zeroize;
                    secret.zeroize();
                }
            }
        });
        let read_deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.check()?;
            if Instant::now() > read_deadline {
                return Err("OS credential-store read timed out".into());
            }
            match receive.recv_timeout(Duration::from_millis(100)) {
                Ok(value) => return value,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("OS credential-store worker stopped".into()),
            }
        }
    }
    fn write(&mut self, _account: &str, _value: Option<&str>) -> Result<(), String> {
        Err("Backup snapshots cannot mutate credentials".into())
    }
}

#[tauri::command]
pub fn cancel_settings_backup_work(request_id: String) -> Result<(), String> {
    if let Some(work) = ACTIVE_WORK
        .lock()
        .map_err(|_| "Backup worker unavailable")?
        .as_ref()
    {
        if work.id == request_id {
            work.cancelled.store(true, Ordering::SeqCst);
        }
    }
    Ok(())
}

pub(super) struct Pending {
    pub(super) id: String,
    source: Archive,
    options: RestoreOptions,
    fingerprint: [u8; 32],
    revision: u64,
    pub(super) expires: Instant,
}
impl Pending {
    pub(super) fn new(
        source: Archive,
        options: RestoreOptions,
        destination: &Archive,
    ) -> Result<Self, String> {
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            source,
            options,
            fingerprint: fingerprint(destination)?,
            revision: gate::revision(),
            expires: Instant::now() + Duration::from_secs(600),
        })
    }
    pub(super) fn verify_identity(&self, id: &str) -> Result<(), String> {
        if self.id != id || Instant::now() > self.expires {
            return Err("Preview expired or replaced; preview the backup again".into());
        }
        Ok(())
    }
    pub(super) fn verify_destination(&self, destination: &Archive) -> Result<(), String> {
        if self.revision != gate::revision() || self.fingerprint != fingerprint(destination)? {
            return Err("Settings changed since preview; preview the backup again".into());
        }
        Ok(())
    }
}
static PENDING: OnceLock<Mutex<Option<Pending>>> = OnceLock::new();
fn pending() -> &'static Mutex<Option<Pending>> {
    PENDING.get_or_init(|| Mutex::new(None))
}
fn directory(app: &AppHandle) -> Result<PathBuf, String> {
    match app.path().app_config_dir() {
        Ok(directory) => Ok(directory),
        Err(error) => Err(format!(
            "Could not locate the application settings directory: {error}"
        )),
    }
}

/// Must run before cache seeding, migrations, listeners, or CC initialization.
pub fn recover_startup(app: &AppHandle) -> Result<(), String> {
    transaction::recover(&directory(app)?, &mut OsSecrets).map_err(|error| {
        gate::block();
        format!("Settings restore recovery is required: {error}. Keep the recovery journal and OS-keyring account; unlock the keyring and restart.")
    })
}

pub(super) fn fingerprint(archive: &Archive) -> Result<[u8; 32], String> {
    // Exclude volatile snapshot metadata; hash settings + known credential presence/values.
    let mut hash = Sha256::new();
    for name in super::FILES {
        hash.update(name.as_bytes());
        hash.update(
            Zeroizing::new(
                serde_json::to_vec(&archive.files[name])
                    .map_err(|_| "Could not fingerprint settings")?,
            )
            .as_slice(),
        );
    }
    hash.update(
        Zeroizing::new(
            serde_json::to_vec(&archive.credentials)
                .map_err(|_| "Could not fingerprint credentials")?,
        )
        .as_slice(),
    );
    Ok(hash.finalize().into())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub id: String,
    pub source_platform: String,
    pub source_version: String,
    pub created_at_ms: u64,
    pub servers: usize,
    pub sections: Vec<SectionSummary>,
    pub credentials_set: usize,
    pub credentials_deleted: usize,
    pub credentials_kept: usize,
    pub credential_actions: Vec<Value>,
    pub external_files: Vec<Value>,
    pub executable_configuration: Value,
    pub warnings: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionSummary {
    name: String,
    state: String,
    changed: bool,
}

pub(super) fn describe(
    source: &Archive,
    destination: &Archive,
    options: &RestoreOptions,
) -> Result<Preview, String> {
    let changes = compile_restore(source, destination, options)?;
    let sections = super::FILES
        .iter()
        .map(|name| SectionSummary {
            name: name.to_string(),
            state: if !changes.files.contains_key(*name) {
                "unselected"
            } else if source.files[*name].is_some() {
                "included"
            } else {
                "absent"
            }
            .into(),
            changed: changes
                .files
                .get(*name)
                .is_some_and(|value| value != &destination.files[*name]),
        })
        .collect();
    let local = source.files["sync-local.json"].as_ref();
    let config = source.files["config.json"].as_ref();
    let paths: BTreeMap<_, _> = super::LOCAL_PATHS
        .iter()
        .map(|key| {
            (
                *key,
                local
                    .and_then(|v| v.get(key))
                    .cloned()
                    .unwrap_or(Value::Null),
            )
        })
        .collect();
    let mut warnings = vec![
        "Replaces selected sections. Unselected sections remain unchanged. Missing selected credentials are deleted; removed servers lose their saved SSH passwords.".into(),
        "Archived Boundary app tokens and telemetry keys are not restored. The destination telemetry key is kept. Changing Boundary controller/auth identity deletes the destination app token and requires a fresh login.".into(),
        "External files, Boundary CLI credentials, SSH-agent keys, deleted-server keyring entries, live sessions, queues, and logs are not bundled. Settings alone do not make a new machine connection-ready.".into(),
        "Review executable paths, arguments, sync root and privilege settings. MCP and CC stay disabled; activate them deliberately after import.".into(),
    ];
    if source.platform != std::env::consts::OS {
        warnings.push("Backup paths came from a different platform. Preserve local paths or remap them in Config after import.".into());
    }
    if options.local && options.recover_identity {
        warnings.push("Same-device recovery selected: original CC identity and MCP token will be restored, with both services disabled.".into());
    }
    let external_files = source.external_files.iter().map(|reference| json!({"kind":reference.kind,"path":reference.path,"existsHere":Path::new(&reference.path).exists()})).collect();
    Ok(Preview {
        id: String::new(),
        source_platform: source.platform.clone(),
        source_version: source.app_version.clone(),
        created_at_ms: source.created_at_ms,
        servers: source.server_ids()?.len(),
        sections,
        credentials_set: changes.credentials.values().filter(|v| v.is_some()).count(),
        credentials_deleted: changes.credentials.values().filter(|v| v.is_none()).count(),
        credentials_kept: destination.credentials.keys().filter(|account| !changes.credentials.contains_key(*account)).count(),
        credential_actions: destination.credentials.keys().chain(changes.credentials.keys()).collect::<std::collections::BTreeSet<_>>().into_iter().map(|account| {
            json!({"account":account,"action":match changes.credentials.get(account) { Some(Some(_)) => "set", Some(None) => "delete", None => "keep" }})
        }).collect(),
        external_files,
        executable_configuration: json!({"sourcePaths": paths, "preserveDestinationPaths":options.preserve_paths,"pathOverrides":options.path_overrides,
            "openvpnExtraArgs":config.and_then(|v| v.pointer("/openvpn/extraArgs")),
            "legacyOpenvpnCommand":config.and_then(|v| v.pointer("/openvpn/command")),
            "terminateOpenVpnWhenHelperExits":config.and_then(|v| v.pointer("/openvpn/terminateOpenVpnWhenHelperExits")),
            "sshOptions":config.and_then(|v| v.pointer("/boundary/connectSshOptions"))}),
        warnings,
    })
}

fn require_stopped(app: &AppHandle, destination: &Archive) -> Result<(), String> {
    if gate::has_async_changes() {
        return Err(
            "Wait for enrollment/revocation requests to finish before importing settings".into(),
        );
    }
    // Each lock is released before any OS-keyring call. Start paths share the maintenance gate.
    if !app
        .state::<SessionStore>()
        .sessions
        .lock()
        .map_err(|_| "PTY store unavailable")?
        .is_empty()
    {
        return Err(
            "Stop all terminals, connections, and transfers before importing settings".into(),
        );
    }
    if !app
        .state::<RemoteBrowserStore>()
        .sessions
        .lock()
        .map_err(|_| "Browser store unavailable")?
        .is_empty()
    {
        return Err("Close remote browsers before importing settings".into());
    }
    let boundary_active = app
        .state::<BoundaryAutomationStore>()
        .session
        .lock()
        .map_err(|_| "Boundary store unavailable")?
        .is_some();
    let openvpn_active = app
        .state::<OpenVpnAdminStore>()
        .session
        .lock()
        .map_err(|_| "OpenVPN store unavailable")?
        .is_some();
    let reconnect_pending = app
        .state::<OpenVpnReconnectStore>()
        .state
        .lock()
        .map_err(|_| "Reconnect store unavailable")?
        .next_retry_at_ms
        .is_some();
    if boundary_active || openvpn_active || reconnect_pending {
        return Err(
            "Stop authentication, OpenVPN and reconnect attempts before importing settings".into(),
        );
    }
    if app
        .state::<ScpTransferStore>()
        .entries
        .lock()
        .map_err(|_| "Transfer store unavailable")?
        .iter()
        .any(|entry| matches!(entry.state, crate::models::ScpTransferState::Pending))
    {
        return Err("Wait for transfers to finish before importing settings".into());
    }
    if app
        .state::<FunctionStore>()
        .statuses
        .lock()
        .map_err(|_| "Function store unavailable")?
        .values()
        .any(|status| {
            matches!(
                status.state.as_str(),
                "starting"
                    | "running"
                    | "reconnecting"
                    | "connecting"
                    | "authenticating"
                    | "stopping"
            )
        })
    {
        return Err(
            "Stop running functions and reconnect attempts before importing settings".into(),
        );
    }
    let local = destination.files["sync-local.json"].as_ref();
    if gate::cc_started()
        || local.is_some_and(|v| v["ccEnabled"] == true || v["mcpEnabled"] == true)
        || app.state::<CcRuntimeState>().is_connected()
        || mcp_runtime_status(&app.state::<McpRuntimeState>())?.active
    {
        return Err(
            "Disable MCP and CC in Config, then restart the app before importing settings".into(),
        );
    }
    Ok(())
}

fn refresh_affected_credentials(
    changes: &super::Changes,
    store: &mut impl SecretStore,
) -> Result<(), String> {
    for account in changes.credentials.keys() {
        let value = store.read(account)?;
        crate::stores::keyring_cache_set(account, value.as_deref().unwrap_or(""));
    }
    Ok(())
}

// Intentionally registered only as desktop commands; no MCP descriptors/handlers.
#[tauri::command]
pub async fn export_settings_backup(
    app: AppHandle,
    path: String,
    passphrase: String,
    confirmation: String,
    overwrite: bool,
    request_id: String,
) -> Result<Value, String> {
    let passphrase = Zeroizing::new(passphrase);
    let confirmation = Zeroizing::new(confirmation);
    let work = BackupWork::begin(request_id)?;
    if *passphrase != *confirmation || passphrase.chars().count() < 12 {
        return Err("Use a matching backup passphrase of at least 12 characters".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = gate::maintain()?;
        let archive = snapshot(&directory(&app)?, &mut &work)?;
        work.check()?;
        let output = PathBuf::from(path);
        if !output.is_absolute() { return Err("Select an absolute backup destination".into()); }
        // Do not allow a backup to replace its own source or recovery files.
        let parent = output.parent().ok_or("Invalid backup destination")?.canonicalize().map_err(|_| "Backup directory does not exist")?;
        let profile = directory(&app)?.canonicalize().map_err(|_| "Settings directory does not exist")?;
        if parent == profile && output.file_name().is_some_and(|n| {
            let name = n.to_string_lossy().to_ascii_lowercase();
            super::FILES.contains(&name.as_str()) || name == transaction::JOURNAL_FILE
        }) { return Err("Backup destination cannot replace application settings".into()); }
        let bytes = crypto::encrypt(&archive, &passphrase)?;
        work.check()?;
        io::atomic_write(&output, &bytes, overwrite)?;
        Ok(json!({"complete":true,"servers":archive.server_ids()?.len(),"credentialsPresent":archive.credentials.values().filter(|v| v.is_some()).count(),"credentialsAbsent":archive.credentials.values().filter(|v| v.is_none()).count(),"sections":super::FILES.iter().map(|name| json!({"name":name,"state":if archive.files[*name].is_some(){"included"}else{"absent"}})).collect::<Vec<_>>()}))
    }).await.map_err(|_| "Backup task failed".to_string())?
}

#[tauri::command]
pub async fn preview_settings_backup(
    app: AppHandle,
    path: String,
    passphrase: String,
    options: RestoreOptions,
    request_id: String,
) -> Result<Preview, String> {
    let passphrase = Zeroizing::new(passphrase);
    let work = BackupWork::begin(request_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        // Invalidate an older preview even if the next decryption fails.
        *pending().lock().map_err(|_| "Preview store unavailable")? = None;
        let bytes = io::read_bounded(Path::new(&path))?.ok_or("Backup file not found")?;
        let source: Archive = crypto::decrypt(&bytes, &passphrase)?;
        source.validate()?;
        let _guard = gate::maintain()?;
        let destination = snapshot(&directory(&app)?, &mut &work)?;
        work.check()?;
        let mut preview = describe(&source, &destination, &options)?;
        let stored = Pending::new(source, options, &destination)?;
        preview.id = stored.id.clone();
        work.check()?;
        *pending().lock().map_err(|_| "Preview store unavailable")? = Some(stored);
        let expiration_id = preview.id.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(600)).await;
            if let Ok(mut stored) = pending().lock() {
                if stored.as_ref().is_some_and(|item| item.id == expiration_id) {
                    *stored = None;
                }
            }
        });
        Ok(preview)
    })
    .await
    .map_err(|_| "Backup preview task failed".to_string())?
}

#[tauri::command]
pub fn discard_settings_backup_preview() -> Result<(), String> {
    *pending().lock().map_err(|_| "Preview store unavailable")? = None;
    Ok(())
}

#[tauri::command]
pub async fn import_settings_backup(
    app: AppHandle,
    preview_id: String,
    confirmed: bool,
) -> Result<(), String> {
    if !confirmed {
        return Err("Confirm replacement of the previewed settings".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = gate::maintain()?;
        let pending = pending().lock().map_err(|_| "Preview store unavailable")?.take().ok_or("Preview expired; preview the backup again")?;
        pending.verify_identity(&preview_id)?;
        let dir = directory(&app)?;
        let mut store = OsSecrets;
        let destination = snapshot(&dir, &mut store)?;
        pending.verify_destination(&destination)?;
        require_stopped(&app, &destination)?;
        let changes = compile_restore(&pending.source, &destination, &pending.options)?;
        let result = transaction::commit(&dir, &changes, &mut store);
        if result.is_err() && dir.join(transaction::JOURNAL_FILE).exists() {
            // Do not let any ordinary command observe a failed/uncertain transaction.
            gate::block();
            return Err("Restore needs recovery. Keep settings-restore-journal.enc, unlock the OS keyring, and restart the app to recover before using settings.".into());
        }
        if let Err(error) = refresh_affected_credentials(&changes, &mut store) { gate::block(); return Err(format!("Settings were committed or rolled back, but cache refresh failed. Restart after unlocking the keyring: {error}")); }
        gate::changed();
        result
    }).await.map_err(|_| "Backup import task failed".to_string())?
}
