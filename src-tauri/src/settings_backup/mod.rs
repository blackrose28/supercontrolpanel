//! Local-only encrypted settings backups. Never expose Archive or Changes over IPC/MCP.
mod crypto;
pub mod desktop;
pub mod gate;
mod io;
mod references;
mod transaction;

use crate::services::config_service::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const FILES: [&str; 3] = ["config.json", "servers.json", "sync-local.json"];
const CONFIGURED: [&str; 5] = [
    KEYRING_OPENVPN_PASSWORD,
    KEYRING_OPENVPN_OTP_SECRET,
    KEYRING_BOUNDARY_PASSWORD,
    KEYRING_BOUNDARY_OIDC_PASSWORD,
    KEYRING_BOUNDARY_OIDC_OTP_SECRET,
];
const LOCAL_PATHS: [&str; 5] = [
    "repoRoot",
    "openvpnExecutablePath",
    "openvpnOvpnPath",
    "boundaryCliPath",
    "caddyFolder",
];

pub trait SecretStore {
    fn read(&mut self, account: &str) -> Result<Option<String>, String>;
    fn write(&mut self, account: &str, value: Option<&str>) -> Result<(), String>;
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Archive {
    schema: u32,
    app_version: String,
    platform: String,
    created_at_ms: u64,
    files: BTreeMap<String, Option<Value>>,
    credentials: BTreeMap<String, Option<String>>,
    external_files: Vec<ExternalFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ExternalFile {
    kind: String,
    path: String,
}

#[derive(Serialize, Deserialize, Clone)]
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

pub struct Changes {
    files: BTreeMap<String, Option<Value>>,
    credentials: BTreeMap<String, Option<String>>,
}

fn known_accounts(ids: &[String]) -> Vec<String> {
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
    fn server_ids(&self) -> Result<Vec<String>, String> {
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
            let server: crate::server_mgmt::Server = serde_json::from_value(value.clone())
                .map_err(|_| "Invalid backup server record")?;
            if server.id.is_empty()
                || server.id.len() > 256
                || !server
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
                || !ids.insert(server.id)
            {
                return Err("Invalid or duplicate backup server ID".into());
            }
        }
        Ok(ids.into_iter().collect())
    }

    fn validate(&self) -> Result<(), String> {
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
                    serde_json::from_value::<crate::models::AppConfig>(value.clone())
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

pub fn snapshot(dir: &Path, store: &mut impl SecretStore) -> Result<Archive, String> {
    let mut files = BTreeMap::new();
    for name in FILES {
        let data = io::read_bounded(&dir.join(name))?.map(zeroize::Zeroizing::new);
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
        app_version: env!("CARGO_PKG_VERSION").into(),
        platform: std::env::consts::OS.into(),
        created_at_ms: crate::services::now_ms(),
        files,
        credentials: BTreeMap::new(),
        external_files: vec![],
    };
    for account in known_accounts(&archive.server_ids()?) {
        // Direct OS reads: missing is valid, locked/denied is never mistaken for absence.
        archive.credentials.insert(account.clone(), store.read(&account).map_err(|_| format!("OS credential store could not read {account}; unlock it and retry. Full backup aborted."))?);
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

fn compile_restore(
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
    let Some(shared) = config.as_mut() else {
        return config;
    };
    {
        let config = shared;
        for (_, group, field) in legacy_fields() {
            if let Some(value) = config.get_mut(group).and_then(Value::as_object_mut) {
                value.remove(field);
            }
        }
        // Legacy executable paths also obey the portable path policy.
        if options.preserve_paths {
            for (group, field) in [
                ("openvpn", "executablePath"),
                ("openvpn", "ovpnPath"),
                ("openvpn", "command"),
                ("boundary", "cliPath"),
            ] {
                if let Some(value) = config.get_mut(group).and_then(Value::as_object_mut) {
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
        // Keep enrollment UI from opening during restore. User explicitly enrolls from Config.
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
        // A destination token must never follow an imported controller/user identity.
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

#[cfg(test)]
mod tests;

fn wipe_value(value: &mut Value) {
    use zeroize::Zeroize;
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(wipe_value),
        Value::Object(fields) => fields.values_mut().for_each(wipe_value),
        _ => {}
    }
}
impl Drop for Archive {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.files.values_mut().flatten().for_each(wipe_value);
        self.credentials
            .values_mut()
            .flatten()
            .for_each(|value| value.zeroize());
    }
}
impl Drop for Changes {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.files.values_mut().flatten().for_each(wipe_value);
        self.credentials
            .values_mut()
            .flatten()
            .for_each(|value| value.zeroize());
    }
}
