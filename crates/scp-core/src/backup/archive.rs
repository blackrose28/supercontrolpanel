//! Shared encrypted settings archive, restore compiler, preview builder, and export guards.
use super::{io, references, transaction, Changes, SecretStore, CONFIGURED, FILES, LOCAL_PATHS};
use crate::models::{
    generate_mcp_auth_token, keyring_server_ssh_password_account, now_ms, AppConfig, Server,
    SyncSettings, KEYRING_BOUNDARY_APP_TOKEN, KEYRING_BOUNDARY_OIDC_OTP_SECRET,
    KEYRING_BOUNDARY_OIDC_PASSWORD, KEYRING_BOUNDARY_PASSWORD, KEYRING_CC_TELEMETRY_QUEUE_KEY,
    KEYRING_OPENVPN_OTP_SECRET, KEYRING_OPENVPN_PASSWORD,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[derive(Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Archive {
    pub schema: u32,
    pub app_version: String,
    pub platform: String,
    pub created_at_ms: u64,
    pub files: BTreeMap<String, Option<Value>>,
    pub credentials: BTreeMap<String, Option<String>>,
    pub external_files: Vec<ExternalFile>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ExternalFile {
    pub kind: String,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct RestoreOptions {
    pub shared: bool,
    pub servers: bool,
    pub local: bool,
    pub secrets: bool,
    pub preserve_paths: bool,
    pub recover_identity: bool,
    pub path_overrides: BTreeMap<String, String>,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self {
            shared: true,
            servers: true,
            local: true,
            secrets: true,
            preserve_paths: true,
            recover_identity: false,
            path_overrides: BTreeMap::new(),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
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

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SectionSummary {
    pub name: String,
    pub state: String,
    pub changed: bool,
}

pub struct PendingRestore {
    pub id: String,
    pub source: Archive,
    pub options: RestoreOptions,
    pub fingerprint: [u8; 32],
    pub revision: u64,
    pub expires: Instant,
}

impl PendingRestore {
    pub fn new(
        source: Archive,
        options: RestoreOptions,
        destination: &Archive,
        revision: u64,
        ttl: Duration,
    ) -> Result<Self, String> {
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            source,
            options,
            fingerprint: fingerprint(destination)?,
            revision,
            expires: Instant::now() + ttl,
        })
    }

    pub fn verify_identity(&self, id: &str) -> Result<(), String> {
        Self::check_identity(&self.id, id, self.expires)
    }

    pub fn verify_destination(
        &self,
        destination: &Archive,
        current_revision: u64,
    ) -> Result<(), String> {
        Self::check_destination(
            self.fingerprint,
            self.revision,
            destination,
            current_revision,
        )
    }

    pub fn check_identity(expected_id: &str, id: &str, expires: Instant) -> Result<(), String> {
        if expected_id != id || Instant::now() > expires {
            return Err("Preview expired or replaced; preview the backup again".into());
        }
        Ok(())
    }

    pub fn check_destination(
        expected_fingerprint: [u8; 32],
        expected_revision: u64,
        destination: &Archive,
        current_revision: u64,
    ) -> Result<(), String> {
        if expected_revision != current_revision
            || expected_fingerprint != fingerprint(destination)?
        {
            return Err("Settings changed since preview; preview the backup again".into());
        }
        Ok(())
    }
}

pub fn known_accounts(ids: &[String]) -> Vec<String> {
    CONFIGURED
        .iter()
        .map(|a| a.to_string())
        .chain([
            KEYRING_BOUNDARY_APP_TOKEN.to_string(),
            KEYRING_CC_TELEMETRY_QUEUE_KEY.to_string(),
        ])
        .chain(ids.iter().map(|id| keyring_server_ssh_password_account(id)))
        .collect()
}

impl Archive {
    pub fn server_ids(&self) -> Result<Vec<String>, String> {
        let Some(value) = self.files.get("servers.json").and_then(Option::as_ref) else {
            return Ok(vec![]);
        };
        let servers = value
            .get("servers")
            .and_then(Value::as_array)
            .ok_or("Invalid backup server list")?;
        if servers.len() > 10000 {
            return Err("Too many servers in backup".into());
        }
        let mut ids = BTreeSet::new();
        for value in servers {
            let server: Server = serde_json::from_value(value.clone())
                .map_err(|_| "Invalid backup server record")?;
            if !is_valid_server_id(&server.id) || !ids.insert(server.id) {
                return Err("Invalid or duplicate backup server ID".into());
            }
        }
        Ok(ids.into_iter().collect())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.app_version.len() > 64
            || self.platform.len() > 64
            || self.external_files.len() > 10000
            || self
                .external_files
                .iter()
                .any(|file| file.kind.len() > 128 || file.path.len() > 4096)
        {
            return Err("Backup manifest metadata exceeds size limits".into());
        }
        if self.schema != 1 {
            return Err("Unsupported settings schema; update the application".into());
        }
        if self.files.len() != FILES.len()
            || FILES.iter().any(|name| !self.files.contains_key(*name))
        {
            return Err("Backup must declare all settings sections".into());
        }
        for name in FILES {
            if let Some(value) = &self.files[name] {
                if !value.is_object() {
                    return Err("Invalid settings section".into());
                }
                if name == "config.json" {
                    serde_json::from_value::<AppConfig>(value.clone())
                        .map_err(|_| "Invalid app settings")?;
                }
                if name == "sync-local.json" {
                    serde_json::from_value::<SyncSettings>(value.clone())
                        .map_err(|_| "Invalid local settings")?;
                }
            }
        }
        let expected: BTreeSet<_> = known_accounts(&self.server_ids()?).into_iter().collect();
        if self.credentials.keys().cloned().collect::<BTreeSet<_>>() != expected {
            return Err(
                "Backup credential inventory does not match known app accounts and server IDs"
                    .into(),
            );
        }
        if self.credentials.values().flatten().any(|s| s.len() > 65536) {
            return Err("Backup credential is too large".into());
        }
        Ok(())
    }
}

pub fn is_valid_server_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
}

impl Drop for Archive {
    fn drop(&mut self) {
        super::wipe_json_and_credentials(&mut self.files, &mut self.credentials);
    }
}

pub fn snapshot(dir: &Path, store: &mut impl SecretStore) -> Result<Archive, String> {
    snapshot_with_version(dir, env!("CARGO_PKG_VERSION"), store)
}

pub fn snapshot_with_version(
    dir: &Path,
    app_version: &str,
    store: &mut impl SecretStore,
) -> Result<Archive, String> {
    let mut files = BTreeMap::new();
    for name in FILES {
        let data = io::read_bounded(&dir.join(name))?.map(Zeroizing::new);
        let value = data
            .map(|bytes| {
                serde_json::from_slice(&bytes)
                    .map_err(|_| "Invalid settings JSON; backup aborted".to_string())
            })
            .transpose()?;
        files.insert(name.into(), value);
    }
    let mut archive = Archive {
        schema: 1,
        app_version: app_version.into(),
        platform: std::env::consts::OS.into(),
        created_at_ms: now_ms(),
        files,
        credentials: BTreeMap::new(),
        external_files: vec![],
    };
    for account in known_accounts(&archive.server_ids()?) {
        archive.credentials.insert(
            account.clone(),
            store.read(&account).map_err(|_| {
                format!("OS credential store could not read {account}; unlock it and retry. Full backup aborted.")
            })?,
        );
    }
    // Legacy plaintext may precede first normal config load. Do not migrate during snapshot.
    for (account, group, field) in legacy_fields() {
        if archive.credentials[account].is_none() {
            let value = archive.files["config.json"]
                .as_ref()
                .and_then(|v| v.get(group))
                .and_then(|v| v.get(field))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            archive.credentials.insert(account.into(), value);
        }
    }
    archive.external_files = references::inventory(&archive);
    archive.validate()?;
    Ok(archive)
}

fn legacy_fields() -> [(&'static str, &'static str, &'static str); 5] {
    [
        (KEYRING_OPENVPN_PASSWORD, "openvpn", "password"),
        (KEYRING_OPENVPN_OTP_SECRET, "openvpn", "otpSecret"),
        (KEYRING_BOUNDARY_PASSWORD, "boundary", "password"),
        (KEYRING_BOUNDARY_OIDC_PASSWORD, "boundary", "oidcPassword"),
        (
            KEYRING_BOUNDARY_OIDC_OTP_SECRET,
            "boundary",
            "oidcOtpSecret",
        ),
    ]
}

pub fn compile_restore(
    source: &Archive,
    destination: &Archive,
    options: &RestoreOptions,
) -> Result<Changes, String> {
    source.validate()?;
    destination.validate()?;
    if options.path_overrides.iter().any(|(key, value)| {
        !LOCAL_PATHS.contains(&key.as_str()) || value.len() > 4096 || value.contains('\0')
    }) {
        return Err("Invalid path remapping".into());
    }
    if !(options.shared || options.servers || options.local || options.secrets) {
        return Err("Select at least one section".into());
    }
    let mut files = BTreeMap::new();
    for (name, selected) in [
        ("config.json", options.shared),
        ("servers.json", options.servers),
        ("sync-local.json", options.local),
    ] {
        if selected {
            files.insert(name.into(), source.files[name].clone());
        }
    }
    if options.shared {
        files.insert(
            "config.json".into(),
            restore_shared(source.files["config.json"].clone(), destination, options),
        );
    }
    if options.local {
        files.insert(
            "sync-local.json".into(),
            restore_local(source, destination, options)?,
        );
    }
    let credentials = restore_credentials(source, destination, options)?;
    Ok(Changes { files, credentials })
}

fn restore_shared(
    mut config: Option<Value>,
    destination: &Archive,
    options: &RestoreOptions,
) -> Option<Value> {
    let shared = config.as_mut()?;
    for (_, group, field) in legacy_fields() {
        if let Some(value) = shared.get_mut(group).and_then(Value::as_object_mut) {
            value.remove(field);
        }
    }
    if options.preserve_paths {
        for (group, field) in [
            ("openvpn", "executablePath"),
            ("openvpn", "ovpnPath"),
            ("openvpn", "command"),
            ("boundary", "cliPath"),
        ] {
            if let Some(value) = shared.get_mut(group).and_then(Value::as_object_mut) {
                let existing = destination.files["config.json"]
                    .as_ref()
                    .and_then(|v| v.get(group))
                    .and_then(|v| v.get(field))
                    .cloned()
                    .unwrap_or(json!(""));
                value.insert(field.into(), existing);
            }
        }
    }
    config
}

fn restore_local(
    source: &Archive,
    destination: &Archive,
    options: &RestoreOptions,
) -> Result<Option<Value>, String> {
    let mut local = source.files["sync-local.json"].clone().unwrap_or(
        serde_json::to_value(SyncSettings::default()).map_err(|_| "Invalid local settings")?,
    );
    local["mcpEnabled"] = json!(false);
    local["ccEnabled"] = json!(false);
    if !options.recover_identity {
        local["mcpAuthToken"] = json!(generate_mcp_auth_token());
        for key in ["ccAgentToken", "ccDeviceId", "ccDeviceName"] {
            local[key] = json!("");
        }
        local["firstRunCompleted"] = json!(true);
    }
    if options.preserve_paths {
        for key in LOCAL_PATHS {
            local[key] = destination.files["sync-local.json"]
                .as_ref()
                .and_then(|v| v.get(key))
                .cloned()
                .unwrap_or(json!(""));
        }
    }
    for (key, value) in &options.path_overrides {
        local[key] = json!(value);
    }
    Ok(Some(local))
}

fn restore_credentials(
    source: &Archive,
    destination: &Archive,
    options: &RestoreOptions,
) -> Result<BTreeMap<String, Option<String>>, String> {
    let mut credentials = BTreeMap::new();
    let source_ids = source.server_ids()?;
    let destination_ids = destination.server_ids()?;
    if options.secrets {
        if !options.servers && source_ids != destination_ids {
            return Err("Import server records together with their passwords, or use matching destination server IDs".into());
        }
        for account in CONFIGURED.iter().map(|a| a.to_string()).chain(
            source_ids
                .iter()
                .map(|id| keyring_server_ssh_password_account(id)),
        ) {
            credentials.insert(account.clone(), source.credentials[&account].clone());
        }
    }
    if options.servers {
        for id in destination_ids.iter().filter(|id| !source_ids.contains(id)) {
            credentials.insert(keyring_server_ssh_password_account(id), None);
        }
    }
    if options.shared
        && boundary_identity(source) != boundary_identity(destination)
        && destination.credentials[KEYRING_BOUNDARY_APP_TOKEN].is_some()
    {
        credentials.insert(KEYRING_BOUNDARY_APP_TOKEN.into(), None);
    }
    Ok(credentials)
}

fn boundary_identity(archive: &Archive) -> Vec<Option<String>> {
    [
        "addr",
        "authMethodId",
        "loginName",
        "authKind",
        "oidcUsername",
    ]
    .iter()
    .map(|field| {
        archive.files["config.json"]
            .as_ref()
            .and_then(|config| config.get("boundary"))
            .and_then(|boundary| boundary.get(field))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(|value| {
                if *field == "addr" {
                    value.trim_end_matches('/').to_string()
                } else {
                    value.to_string()
                }
            })
    })
    .collect()
}

pub fn fingerprint(archive: &Archive) -> Result<[u8; 32], String> {
    let mut hash = Sha256::new();
    for name in FILES {
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

pub fn describe(
    source: &Archive,
    destination: &Archive,
    options: &RestoreOptions,
) -> Result<Preview, String> {
    let changes = compile_restore(source, destination, options)?;
    let sections = FILES
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
    let paths: BTreeMap<_, _> = LOCAL_PATHS
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
    let external_files = source
        .external_files
        .iter()
        .map(|reference| {
            json!({"kind": reference.kind, "path": reference.path, "existsHere": Path::new(&reference.path).exists()})
        })
        .collect();
    Ok(Preview {
        id: String::new(),
        source_platform: source.platform.clone(),
        source_version: source.app_version.clone(),
        created_at_ms: source.created_at_ms,
        servers: source.server_ids()?.len(),
        sections,
        credentials_set: changes.credentials.values().filter(|v| v.is_some()).count(),
        credentials_deleted: changes.credentials.values().filter(|v| v.is_none()).count(),
        credentials_kept: destination
            .credentials
            .keys()
            .filter(|account| !changes.credentials.contains_key(*account))
            .count(),
        credential_actions: destination
            .credentials
            .keys()
            .chain(changes.credentials.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|account| {
                json!({"account": account, "action": match changes.credentials.get(account) {
                    Some(Some(_)) => "set",
                    Some(None) => "delete",
                    None => "keep",
                }})
            })
            .collect(),
        external_files,
        executable_configuration: json!({
            "sourcePaths": paths,
            "preserveDestinationPaths": options.preserve_paths,
            "pathOverrides": options.path_overrides,
            "openvpnExtraArgs": config.and_then(|v| v.pointer("/openvpn/extraArgs")),
            "legacyOpenvpnCommand": config.and_then(|v| v.pointer("/openvpn/command")),
            "terminateOpenVpnWhenHelperExits": config.and_then(|v| v.pointer("/openvpn/terminateOpenVpnWhenHelperExits")),
            "sshOptions": config.and_then(|v| v.pointer("/boundary/connectSshOptions")),
        }),
        warnings,
    })
}

pub fn validate_export_target(
    output: &Path,
    profile_dir: &Path,
    require_absolute: bool,
) -> Result<PathBuf, String> {
    if output.as_os_str().is_empty() {
        return Err("Invalid backup destination".into());
    }
    let resolved = if output.is_absolute() {
        output.to_path_buf()
    } else if require_absolute {
        return Err("Select an absolute backup destination".into());
    } else {
        std::env::current_dir()
            .map_err(|_| "Could not resolve working directory for backup destination")?
            .join(output)
    };
    let file_name = resolved.file_name().ok_or("Invalid backup destination")?;
    let parent = resolved
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("Invalid backup destination")?
        .canonicalize()
        .map_err(|_| "Backup directory does not exist")?;
    let profile = profile_dir
        .canonicalize()
        .map_err(|_| "Settings directory does not exist")?;
    let name_lower = file_name.to_string_lossy().to_ascii_lowercase();
    if parent == profile
        && (FILES.contains(&name_lower.as_str())
            || matches!(
                name_lower.as_str(),
                "owner.lock" | "profile-meta.json" | "stdio-policy.json"
            )
            || name_lower == transaction::JOURNAL_FILE)
    {
        return Err("Backup destination cannot replace application settings".into());
    }
    Ok(parent.join(file_name))
}
