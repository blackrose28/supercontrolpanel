//! Standalone headless runtime enforcing the 5-step profile startup, recovery, and query contract.
use crate::backup::{io, transaction};
use crate::config_repo::{
    load_app_config_and_migrate, load_servers, load_stdio_policy, load_sync_settings,
};
use crate::error::CoreError;
use crate::gate::MaintenanceGate;
use crate::models::{
    default_function_statuses, AppConfig, ConfigPaths, FunctionStatus, ScpTransferEntry, Server,
    StdioPolicy, SyncSettings, BOUNDARY_AUTH_FUNCTION_ID, CADDY_FUNCTION_ID, KEYRING_SERVICE,
    OPENVPN_FUNCTION_ID,
};
use crate::profile::{
    acquire_owner_lease, ensure_or_load_metadata, resolve_profile_paths, OwnerLease,
    ProfileLocationInput, ProfileMetadata, ProfilePaths,
};
use crate::secrets::{KeyringBackend, KeyringHealth, NamespacedSecretStore, OsKeyringBackend};
use parking_lot::Mutex;
use std::collections::HashMap;
use uuid::Uuid;

pub const DEFAULT_SERVER_LIST_LIMIT: usize = 100;
pub const MAX_SERVER_LIST_LIMIT: usize = 500;

#[derive(Debug, Clone)]
pub struct ServerPage {
    pub servers: Vec<Server>,
    pub truncated: bool,
    pub next_after: Option<String>,
    pub total: usize,
}

pub struct ProfileRuntime<B: KeyringBackend = OsKeyringBackend> {
    runtime_id: String,
    profile_paths: ProfilePaths,
    _owner_lease: OwnerLease,
    metadata: ProfileMetadata,
    gate: MaintenanceGate,
    secrets: Mutex<NamespacedSecretStore<B>>,
    servers: Vec<Server>,
    app_config: AppConfig,
    sync_settings: SyncSettings,
    stdio_policy: StdioPolicy,
    function_statuses: HashMap<String, FunctionStatus>,
    transfers: Vec<ScpTransferEntry>,
}

impl ProfileRuntime<OsKeyringBackend> {
    pub fn open(input: &ProfileLocationInput<'_>, mode: &str) -> Result<Self, CoreError> {
        Self::open_with_backend(input, mode, OsKeyringBackend)
    }
}

impl<B: KeyringBackend> ProfileRuntime<B> {
    pub fn open_with_backend(
        input: &ProfileLocationInput<'_>,
        mode: &str,
        backend: B,
    ) -> Result<Self, CoreError> {
        let runtime_id = format!("rt-{}", Uuid::new_v4().simple());
        // Step 1: resolve, validate, reject desktop aliases, and take exclusive OS lease.
        let profile_paths = resolve_profile_paths(input)?;
        let _owner_lease = acquire_owner_lease(&profile_paths, &runtime_id, mode)?;
        // Step 2: load or atomically initialize profile metadata (never on pending recovery).
        let metadata = ensure_or_load_metadata(&profile_paths)?;
        let gate = MaintenanceGate::new();
        let mut secrets = NamespacedSecretStore::new(
            KEYRING_SERVICE,
            metadata.credential_namespace.clone(),
            backend,
        );
        // Step 3: mandatory journal recovery BEFORE config/server parsing or cache use.
        run_recovery_step(&profile_paths, &gate, &mut secrets)?;
        // Step 4: load servers, sync settings, stdio policy, and migrate legacy plaintext secrets.
        let mut servers = load_servers(&profile_paths, &gate)?;
        servers.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.name.cmp(&b.name)));
        let sync_settings = load_sync_settings(&profile_paths, &gate)?;
        let stdio_policy = load_stdio_policy(&profile_paths, &gate)?;
        let app_config = load_app_config_and_migrate(&profile_paths, &gate, &mut secrets)?;
        // Step 5: initialize read-only runtime state.
        let function_statuses = default_function_statuses(
            OPENVPN_FUNCTION_ID,
            CADDY_FUNCTION_ID,
            BOUNDARY_AUTH_FUNCTION_ID,
        );
        Ok(Self {
            runtime_id,
            profile_paths,
            _owner_lease,
            metadata,
            gate,
            secrets: Mutex::new(secrets),
            servers,
            app_config,
            sync_settings,
            stdio_policy,
            function_statuses,
            transfers: Vec::new(),
        })
    }

    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    pub fn profile_paths(&self) -> &ProfilePaths {
        &self.profile_paths
    }

    pub fn credential_namespace(&self) -> &str {
        &self.metadata.credential_namespace
    }

    pub fn metadata(&self) -> &ProfileMetadata {
        &self.metadata
    }

    pub fn gate(&self) -> &MaintenanceGate {
        &self.gate
    }

    pub fn keyring_health(&self) -> KeyringHealth {
        self.secrets.lock().preflight()
    }

    pub fn list_function_statuses(&self) -> Result<Vec<FunctionStatus>, CoreError> {
        let _guard = self.gate.enter()?;
        let mut list: Vec<FunctionStatus> = self.function_statuses.values().cloned().collect();
        list.sort_by(|a, b| a.function_id.cmp(&b.function_id));
        Ok(list)
    }

    pub fn get_function_status(&self, function_id: &str) -> Result<FunctionStatus, CoreError> {
        let _guard = self.gate.enter()?;
        if function_id.trim().is_empty() {
            return Err(CoreError::invalid_arguments(
                "function_id_required",
                "functionId is required",
            ));
        }
        self.function_statuses
            .get(function_id)
            .cloned()
            .ok_or_else(|| {
                CoreError::not_found(
                    "function_not_found",
                    format!("Unknown function: {function_id}"),
                    "inspect_status",
                )
            })
    }

    pub fn get_function_logs(&self, function_id: &str) -> Result<Vec<String>, CoreError> {
        let _ = self.get_function_status(function_id)?;
        Ok(Vec::new())
    }

    pub fn list_servers(
        &self,
        limit: Option<usize>,
        after: Option<&str>,
    ) -> Result<ServerPage, CoreError> {
        let _guard = self.gate.enter()?;
        let limit = limit.unwrap_or(DEFAULT_SERVER_LIST_LIMIT);
        if limit == 0 || limit > MAX_SERVER_LIST_LIMIT {
            return Err(CoreError::invalid_arguments(
                "invalid_pagination",
                format!("limit must be between 1 and {MAX_SERVER_LIST_LIMIT}"),
            ));
        }
        let start = match after {
            None => 0,
            Some(cursor) if cursor.trim().is_empty() => {
                return Err(CoreError::invalid_arguments(
                    "invalid_pagination",
                    "after cursor must not be empty",
                ));
            }
            Some(cursor) => self
                .servers
                .iter()
                .position(|s| s.id == cursor)
                .map(|idx| idx + 1)
                .ok_or_else(|| {
                    CoreError::invalid_arguments(
                        "invalid_pagination",
                        format!("Unknown after cursor: {cursor}"),
                    )
                })?,
        };
        let end = (start + limit).min(self.servers.len());
        let page = self.servers[start..end].to_vec();
        let truncated = end < self.servers.len();
        let next_after = if truncated {
            page.last().map(|s| s.id.clone())
        } else {
            None
        };
        Ok(ServerPage {
            servers: page,
            truncated,
            next_after,
            total: self.servers.len(),
        })
    }

    pub fn get_server(&self, server_id: &str) -> Result<Server, CoreError> {
        let _guard = self.gate.enter()?;
        if server_id.trim().is_empty() {
            return Err(CoreError::invalid_arguments(
                "server_id_required",
                "serverId is required",
            ));
        }
        self.servers
            .iter()
            .find(|s| s.id == server_id)
            .cloned()
            .ok_or_else(|| {
                CoreError::not_found(
                    "server_not_found",
                    format!("Unknown serverId: {server_id}"),
                    "inspect_servers",
                )
            })
    }

    pub fn get_app_config(&self) -> Result<AppConfig, CoreError> {
        let _guard = self.gate.enter()?;
        Ok(self.app_config.clone())
    }

    pub fn get_sync_settings(&self) -> Result<SyncSettings, CoreError> {
        let _guard = self.gate.enter()?;
        Ok(self.sync_settings.clone())
    }

    pub fn get_stdio_policy(&self) -> Result<&StdioPolicy, CoreError> {
        let _guard = self.gate.enter()?;
        Ok(&self.stdio_policy)
    }

    pub fn get_config_paths(&self) -> Result<ConfigPaths, CoreError> {
        let _guard = self.gate.enter()?;
        Ok(ConfigPaths {
            app_config_path: self
                .profile_paths
                .config_path
                .to_string_lossy()
                .into_owned(),
        })
    }

    pub fn list_transfers(&self) -> Result<Vec<ScpTransferEntry>, CoreError> {
        let _guard = self.gate.enter()?;
        Ok(self.transfers.clone())
    }
}

fn run_recovery_step<B: KeyringBackend>(
    paths: &ProfilePaths,
    gate: &MaintenanceGate,
    secrets: &mut NamespacedSecretStore<B>,
) -> Result<(), CoreError> {
    let pending = io::read_bounded(&paths.journal_path)
        .map_err(|e| {
            gate.block(e.clone());
            CoreError::conflict("recovery_blocked", e, false, "unlock_keyring_and_retry")
        })?
        .is_some_and(|b| b != transaction::DONE);
    if !pending {
        return Ok(());
    }
    let _maint = gate.maintain()?;
    if let Err(err) = transaction::recover(&paths.profile_dir, secrets) {
        let msg = format!("Startup settings recovery failed: {err}");
        gate.block(msg.clone());
        return Err(CoreError::conflict(
            "recovery_blocked",
            msg,
            false,
            "unlock_keyring_and_retry",
        ));
    }
    Ok(())
}
