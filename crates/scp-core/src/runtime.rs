//! Standalone headless runtime enforcing 5-step startup, recovery, supervision, and ordered shutdown.
use crate::backup::{io, transaction};
use crate::boundary_ops::{
    refresh_boundary_status_impl, search_boundary_targets_impl, start_boundary_auth_impl,
};
use crate::caddy_ops::{start_caddy_impl, stop_caddy_impl};
use crate::config_repo::{
    load_app_config_and_migrate, load_servers, load_stdio_policy, load_sync_settings,
};
use crate::error::CoreError;
use crate::events::{DomainEvent, EventSink, NoopEventSink};
use crate::gate::MaintenanceGate;
use crate::models::{
    default_function_statuses, now_ms, AppConfig, BoundaryTargetSummary, ConfigPaths,
    FunctionStatus, RemoteBrowseEntry, RemoteBrowseNodeInfo, RemoteBrowseSession,
    ScpAsyncStartResult, ScpDownloadRequest, ScpTransferEntry, Server, StartFunctionResult,
    StdioPolicy, SyncSettings, BOUNDARY_AUTH_FUNCTION_ID, CADDY_FUNCTION_ID, KEYRING_SERVICE,
    OPENVPN_FUNCTION_ID,
};
use crate::openvpn_ops::{
    detect_openvpn_impl, start_openvpn_impl, stop_openvpn_impl, OpenVpnRuntimeOverrides,
};
use crate::profile::{
    acquire_owner_lease, ensure_or_load_metadata, resolve_profile_paths, OwnerLease,
    ProfileLocationInput, ProfileMetadata, ProfilePaths,
};
use crate::secrets::{KeyringBackend, KeyringHealth, NamespacedSecretStore, OsKeyringBackend};
use crate::sftp_ops::{
    close_all_browsers_impl, close_browsers_for_server_impl, close_remote_browser_impl,
    create_remote_directory_impl, delete_remote_path_impl, list_remote_children_impl,
    open_remote_browser_impl, stat_remote_path_impl, RemoteBrowserStore,
};
use crate::ssh_ops::{
    acquire_endpoint_for_server, default_identity_path, default_known_hosts_path,
    prepare_download_spec, prepare_ssh_launch, run_scp_sync, scp_upload_impl,
    server_ssh_function_id, server_tunnel_function_id, start_scp_async, start_scp_upload_impl,
    start_server_tunnel_impl,
};
use crate::supervisor::{
    AcceptOutcome, CancelToken, ChildLaunch, ExitCallback, OutputCallback, ProcessSupervisor,
    SpawnMode,
};
use crate::transfer_store::{get_scp_transfer_entry, list_scp_transfer_entries, ScpTransferStore};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

pub const DEFAULT_SERVER_LIST_LIMIT: usize = 100;
pub const MAX_SERVER_LIST_LIMIT: usize = 500;
const SHUTDOWN_KIND_ORDER: &[&str] = &[
    "browser",
    "transfer",
    "ssh",
    "tunnel",
    "caddy",
    "boundary-auth",
    "vpn",
];

#[derive(Debug, Clone)]
pub struct ServerPage {
    pub servers: Vec<Server>,
    pub truncated: bool,
    pub next_after: Option<String>,
    pub total: usize,
}

pub(crate) struct RuntimeState {
    pub(crate) function_statuses: HashMap<String, FunctionStatus>,
    pub(crate) reserved_functions: HashSet<String>,
    pub(crate) reserved_ports: HashMap<u16, String>,
    pub(crate) explicit_stops: HashSet<String>,
    pub(crate) shutting_down: bool,
}

impl RuntimeState {
    fn new(function_statuses: HashMap<String, FunctionStatus>) -> Self {
        Self {
            function_statuses,
            reserved_functions: HashSet::new(),
            reserved_ports: HashMap::new(),
            explicit_stops: HashSet::new(),
            shutting_down: false,
        }
    }
}

pub struct ProfileRuntime<B: KeyringBackend = OsKeyringBackend> {
    runtime_id: String,
    profile_paths: ProfilePaths,
    _owner_lease: OwnerLease,
    metadata: ProfileMetadata,
    gate: MaintenanceGate,
    secrets: Arc<Mutex<NamespacedSecretStore<B>>>,
    servers: Mutex<Vec<Server>>,
    app_config: Mutex<AppConfig>,
    sync_settings: Mutex<SyncSettings>,
    stdio_policy: StdioPolicy,
    runtime_state: Arc<Mutex<RuntimeState>>,
    transfers: Arc<ScpTransferStore>,
    remote_browsers: Arc<RemoteBrowserStore>,
    supervisor: Arc<ProcessSupervisor>,
    event_sink: Arc<dyn EventSink>,
    ssh_known_hosts_override: Mutex<Option<PathBuf>>,
    ssh_identity_override: Mutex<Option<PathBuf>>,
    tunnel_ready_timeout_override: Mutex<Option<Duration>>,
    openvpn_overrides: Mutex<OpenVpnRuntimeOverrides>,
    caddy_executable_override: Mutex<Option<String>>,
    vpn_lease: Arc<Mutex<Option<(String, OwnerLease)>>>,
    shutdown_done: AtomicBool,
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
        Self::open_with_backend_and_sink(input, mode, backend, Arc::new(NoopEventSink))
    }

    pub fn open_with_backend_and_sink(
        input: &ProfileLocationInput<'_>,
        mode: &str,
        backend: B,
        event_sink: Arc<dyn EventSink>,
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
        // Step 5: initialize runtime state stores and process supervisor.
        let statuses = default_function_statuses(
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
            secrets: Arc::new(Mutex::new(secrets)),
            servers: Mutex::new(servers),
            app_config: Mutex::new(app_config),
            sync_settings: Mutex::new(sync_settings),
            stdio_policy,
            runtime_state: Arc::new(Mutex::new(RuntimeState::new(statuses))),
            transfers: Arc::new(ScpTransferStore::default()),
            remote_browsers: Arc::new(RemoteBrowserStore::new()),
            supervisor: Arc::new(ProcessSupervisor::new()),
            event_sink,
            ssh_known_hosts_override: Mutex::new(None),
            ssh_identity_override: Mutex::new(None),
            tunnel_ready_timeout_override: Mutex::new(None),
            openvpn_overrides: Mutex::new(OpenVpnRuntimeOverrides::default()),
            caddy_executable_override: Mutex::new(None),
            vpn_lease: Arc::new(Mutex::new(None)),
            shutdown_done: AtomicBool::new(false),
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

    pub fn supervisor(&self) -> &Arc<ProcessSupervisor> {
        &self.supervisor
    }

    pub fn event_sink(&self) -> &Arc<dyn EventSink> {
        &self.event_sink
    }

    pub fn transfer_store(&self) -> &Arc<ScpTransferStore> {
        &self.transfers
    }

    pub fn remote_browsers(&self) -> &Arc<RemoteBrowserStore> {
        &self.remote_browsers
    }

    pub fn set_ssh_paths(&self, known_hosts: Option<PathBuf>, identity_file: Option<PathBuf>) {
        *self.ssh_known_hosts_override.lock() = known_hosts;
        *self.ssh_identity_override.lock() = identity_file;
    }

    pub fn set_tunnel_ready_timeout(&self, timeout: Option<Duration>) {
        *self.tunnel_ready_timeout_override.lock() = timeout;
    }

    pub fn set_openvpn_overrides(&self, overrides: OpenVpnRuntimeOverrides) {
        *self.openvpn_overrides.lock() = overrides;
    }

    pub fn set_caddy_executable(&self, executable: Option<String>) {
        *self.caddy_executable_override.lock() = executable;
    }

    pub(crate) fn caddy_executable_override(&self) -> Option<String> {
        self.caddy_executable_override.lock().clone()
    }

    pub(crate) fn openvpn_overrides(&self) -> OpenVpnRuntimeOverrides {
        self.openvpn_overrides.lock().clone()
    }

    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub(crate) fn vpn_lease_arc(&self) -> &Arc<Mutex<Option<(String, OwnerLease)>>> {
        &self.vpn_lease
    }

    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub(crate) fn is_shutting_down(&self) -> bool {
        self.runtime_state.lock().shutting_down
    }

    pub(crate) fn resolved_ssh_paths(&self) -> (Option<PathBuf>, Option<PathBuf>) {
        let kh = self
            .ssh_known_hosts_override
            .lock()
            .clone()
            .or_else(default_known_hosts_path);
        let id = self
            .ssh_identity_override
            .lock()
            .clone()
            .or_else(default_identity_path);
        (kh, id)
    }

    pub(crate) fn tunnel_ready_timeout_override(&self) -> Option<Duration> {
        *self.tunnel_ready_timeout_override.lock()
    }

    pub fn read_optional_secret(&self, account: &str) -> Option<String> {
        self.secrets.lock().get(account).ok().flatten()
    }

    pub fn write_secret(&self, account: &str, value: Option<&str>) -> Result<(), CoreError> {
        match value {
            Some(v) => self.secrets.lock().set(account, v),
            None => self.secrets.lock().delete(account),
        }
    }

    pub(crate) fn secrets_arc(&self) -> &Arc<Mutex<NamespacedSecretStore<B>>> {
        &self.secrets
    }

    pub(crate) fn runtime_state_arc(&self) -> &Arc<Mutex<RuntimeState>> {
        &self.runtime_state
    }

    pub fn keyring_health(&self) -> KeyringHealth {
        self.secrets.lock().preflight()
    }

    pub(crate) fn with_gate<T>(
        &self,
        f: impl FnOnce() -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        let _guard = self.gate.enter()?;
        f()
    }

    pub(crate) fn update_status_snapshot(&self, update: StatusUpdate<'_>) {
        let status = {
            let mut st = self.runtime_state.lock();
            let preserved_token = st
                .function_statuses
                .get(update.function_id)
                .and_then(|s| s.boundary_token_source.clone());
            let token_source = match update.boundary_token_source {
                Some(explicit) => explicit,
                None => preserved_token,
            };
            let entry = FunctionStatus {
                function_id: update.function_id.to_string(),
                state: update.state.to_string(),
                message: update.message,
                boundary_token_source: token_source,
                session_id: update.session_id,
                updated_at_ms: now_ms(),
            };
            st.function_statuses
                .insert(update.function_id.to_string(), entry.clone());
            entry
        };
        self.event_sink
            .emit(DomainEvent::FunctionStatusChanged { status });
    }

    pub fn list_function_statuses(&self) -> Result<Vec<FunctionStatus>, CoreError> {
        self.with_gate(|| {
            let st = self.runtime_state.lock();
            let mut list: Vec<FunctionStatus> = st.function_statuses.values().cloned().collect();
            list.sort_by(|a, b| a.function_id.cmp(&b.function_id));
            Ok(list)
        })
    }

    pub fn get_function_status(&self, function_id: &str) -> Result<FunctionStatus, CoreError> {
        require_non_empty(
            function_id,
            "function_id_required",
            "functionId is required",
        )?;
        self.with_gate(|| {
            self.runtime_state
                .lock()
                .function_statuses
                .get(function_id)
                .cloned()
                .ok_or_else(|| {
                    CoreError::not_found(
                        "function_not_found",
                        format!("Unknown function: {function_id}"),
                        "inspect_status",
                    )
                })
        })
    }

    pub fn get_function_logs(&self, function_id: &str) -> Result<Vec<String>, CoreError> {
        require_non_empty(
            function_id,
            "function_id_required",
            "functionId is required",
        )?;
        let _guard = self.gate.enter()?;
        let logs = self.supervisor.get_function_logs(function_id);
        let known = self
            .runtime_state
            .lock()
            .function_statuses
            .contains_key(function_id);
        if !known && logs.is_empty() {
            return Err(CoreError::not_found(
                "function_not_found",
                format!("Unknown function: {function_id}"),
                "inspect_status",
            ));
        }
        Ok(logs)
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
        let servers = self.servers.lock();
        let start = resolve_server_cursor(&servers, after)?;
        let end = (start + limit).min(servers.len());
        let page = servers[start..end].to_vec();
        let truncated = end < servers.len();
        let next_after = truncated
            .then(|| page.last().map(|s| s.id.clone()))
            .flatten();
        Ok(ServerPage {
            servers: page,
            truncated,
            next_after,
            total: servers.len(),
        })
    }

    pub fn get_server(&self, server_id: &str) -> Result<Server, CoreError> {
        let id = require_non_empty(server_id, "server_id_required", "serverId is required")?;
        let _guard = self.gate.enter()?;
        self.servers
            .lock()
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .ok_or_else(|| {
                CoreError::not_found(
                    "server_not_found",
                    format!("Unknown serverId: {id}"),
                    "inspect_servers",
                )
            })
    }

    pub fn get_app_config(&self) -> Result<AppConfig, CoreError> {
        self.gated_clone(&self.app_config)
    }

    pub fn get_sync_settings(&self) -> Result<SyncSettings, CoreError> {
        self.gated_clone(&self.sync_settings)
    }

    fn gated_clone<T: Clone>(&self, slot: &Mutex<T>) -> Result<T, CoreError> {
        self.with_gate(|| Ok(slot.lock().clone()))
    }

    pub fn get_stdio_policy(&self) -> Result<&StdioPolicy, CoreError> {
        let _guard = self.gate.enter()?;
        Ok(&self.stdio_policy)
    }

    pub fn get_config_paths(&self) -> Result<ConfigPaths, CoreError> {
        let _guard = self.gate.enter()?;
        let app_config_path = self
            .profile_paths
            .config_path
            .to_string_lossy()
            .into_owned();
        Ok(ConfigPaths { app_config_path })
    }

    pub fn list_transfers(&self) -> Result<Vec<ScpTransferEntry>, CoreError> {
        self.with_gate(|| list_scp_transfer_entries(&self.transfers).map_err(map_transfer_err))
    }

    pub fn get_transfer(&self, transfer_id: &str) -> Result<Option<ScpTransferEntry>, CoreError> {
        self.with_gate(|| {
            get_scp_transfer_entry(&self.transfers, transfer_id).map_err(map_transfer_err)
        })
    }

    /// Reserves a loopback TCP port inside this runtime and verifies OS bindability.
    pub fn reserve_port(&self, port: u16, owner: &str) -> Result<(), CoreError> {
        let mut st = self.runtime_state.lock();
        reserve_port_in_state(&mut st, port, owner)
    }

    /// Releases a previously reserved loopback TCP port.
    pub fn release_port(&self, port: u16) {
        self.runtime_state.lock().reserved_ports.remove(&port);
    }

    /// Reserves function/port slots, launches a supervised child, and wires status/events.
    pub fn start_supervised_function(
        &self,
        launch: ChildLaunch,
        cancel: &CancelToken,
        port: Option<u16>,
        initial_message: &str,
    ) -> Result<String, CoreError> {
        self.start_supervised_with_hooks(
            launch,
            cancel,
            port,
            initial_message,
            SuperviseHooks {
                on_output: None,
                on_exit: None,
            },
        )
    }

    pub(crate) fn start_supervised_with_hooks(
        &self,
        launch: ChildLaunch,
        cancel: &CancelToken,
        port: Option<u16>,
        initial_message: &str,
        hooks: SuperviseHooks,
    ) -> Result<String, CoreError> {
        let _guard = self.gate.enter()?;
        let function_id = launch.function_id.clone();
        reserve_launch_slots(&self.runtime_state, &function_id, port)?;
        let req = ReservedSpawn {
            launch,
            port,
            initial_message,
            hooks,
        };
        let res = self.spawn_reserved(req, cancel);
        if res.is_err() {
            release_launch_slots(&self.runtime_state, &function_id, port);
        }
        res
    }

    fn spawn_reserved(
        &self,
        req: ReservedSpawn<'_>,
        cancel: &CancelToken,
    ) -> Result<String, CoreError> {
        let function_id = req.launch.function_id.clone();
        let state_accept = Arc::clone(&self.runtime_state);
        let sink_accept = Arc::clone(&self.event_sink);
        let initial_msg = req.initial_message.to_string();
        let port = req.port;
        let on_exit = wrap_exit_callback(
            Arc::clone(&self.runtime_state),
            Arc::clone(&self.event_sink),
            function_id.clone(),
            port,
            req.hooks.on_exit,
        );
        let on_output = wrap_output_callback(Arc::clone(&self.event_sink), req.hooks.on_output);
        let spawn_result = self.supervisor.spawn_with_gate(
            req.launch,
            cancel,
            move |session_id| {
                let Some(status) = try_accept_launch(
                    &state_accept,
                    cancel,
                    &function_id,
                    port,
                    session_id,
                    &initial_msg,
                ) else {
                    return AcceptOutcome::CancelledBeforeAccept;
                };
                sink_accept.emit(DomainEvent::FunctionStatusChanged { status });
                AcceptOutcome::Accepted(())
            },
            on_exit,
            Some(on_output),
        );
        match spawn_result {
            Ok(AcceptOutcome::Accepted(sid)) => Ok(sid),
            Ok(AcceptOutcome::CancelledBeforeAccept) => Err(CoreError::interrupted(
                "Operation cancelled before acceptance",
            )),
            Err(err) => Err(err),
        }
    }

    /// Stops an active supervised function idempotently.
    pub fn stop_supervised_function(
        &self,
        function_id: &str,
        timeout: Duration,
    ) -> Result<bool, CoreError> {
        let Some(sid) = self.supervisor.active_session_for_function(function_id) else {
            return Ok(false);
        };
        self.runtime_state.lock().explicit_stops.insert(sid.clone());
        let stopped = self.supervisor.stop_session(&sid, timeout)?;
        if !stopped {
            self.runtime_state.lock().explicit_stops.remove(&sid);
        }
        Ok(stopped)
    }

    /// Starts a Boundary local TCP forwarder for `server_id` and waits until ready.
    pub fn start_server_tunnel(
        &self,
        server_id: &str,
        cancel: &CancelToken,
    ) -> Result<StartFunctionResult, CoreError> {
        start_server_tunnel_impl(self, server_id, cancel)
    }

    /// Closes open remote browsers for `server_id` and stops its Boundary tunnel idempotently.
    pub fn stop_server_tunnel(
        &self,
        server_id: &str,
        timeout: Duration,
    ) -> Result<bool, CoreError> {
        close_browsers_for_server_impl(self, server_id, "tunnel_stopped");
        self.stop_supervised_function(&server_tunnel_function_id(server_id), timeout)
    }

    /// Starts an interactive PTY-backed SSH session for `server_id`.
    pub fn start_server_ssh(
        &self,
        server_id: &str,
        cols: u16,
        rows: u16,
        cancel: &CancelToken,
    ) -> Result<StartFunctionResult, CoreError> {
        let server = self.get_server(server_id)?;
        let (endpoint, mut lease) =
            acquire_endpoint_for_server(self, &server, "start_server_ssh", cancel)?;
        let (program, args) = prepare_ssh_launch(self, &server, &endpoint)?;
        let function_id = server_ssh_function_id(&server.id);
        let launch = ChildLaunch {
            function_id: function_id.clone(),
            program,
            args,
            cwd: None,
            env: Vec::new(),
            env_remove: Vec::new(),
            mode: SpawnMode::Pty { cols, rows },
            kill_tree: true,
            kind: "ssh",
        };
        let session_id = self.start_supervised_function(
            launch,
            cancel,
            None,
            &format!("Connected SSH to {}", server.name),
        )?;
        lease.disarm();
        Ok(StartFunctionResult {
            function_id,
            session_id,
        })
    }

    /// Prepares command args (and optional temporary Boundary tunnel) for direct foreground TTY SSH.
    pub fn prepare_interactive_ssh(
        &self,
        server_id: &str,
        cancel: &CancelToken,
    ) -> Result<(String, Vec<String>, Option<String>), CoreError> {
        let server = self.get_server(server_id)?;
        let (endpoint, mut lease) =
            acquire_endpoint_for_server(self, &server, "interactive_ssh", cancel)?;
        let (program, args) = prepare_ssh_launch(self, &server, &endpoint)?;
        let temp_server = lease.is_temporary().then(|| server.id.clone());
        lease.disarm();
        Ok((program, args, temp_server))
    }

    /// Stops an active supervised SSH session for `server_id`.
    pub fn stop_server_ssh(&self, server_id: &str, timeout: Duration) -> Result<bool, CoreError> {
        self.stop_supervised_function(&server_ssh_function_id(server_id), timeout)
    }

    /// Runs a strictly verified SCP upload (with automatic temporary Boundary tunnel if needed).
    pub fn scp_upload(
        &self,
        server_id: &str,
        local_path: &str,
        remote_dir: &str,
        recursive: bool,
        cancel: &CancelToken,
    ) -> Result<ScpTransferEntry, CoreError> {
        scp_upload_impl(self, server_id, local_path, remote_dir, recursive, cancel)
    }

    /// Runs a strictly verified SCP download (with automatic temporary Boundary tunnel if needed).
    pub fn scp_download(
        &self,
        server_id: &str,
        remote_path: &str,
        local_target: &str,
        recursive: bool,
        overwrite: bool,
        cancel: &CancelToken,
    ) -> Result<ScpTransferEntry, CoreError> {
        let req = ScpDownloadRequest {
            server_id,
            remote_path,
            local_target,
            recursive,
            overwrite,
        };
        let (spec, _lease) = prepare_download_spec(self, &req, false, cancel)?;
        run_scp_sync(self, spec, cancel)
    }

    /// Searches configured Boundary targets using isolated profile environment.
    pub fn search_boundary_targets(
        &self,
        filter: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<BoundaryTargetSummary>, CoreError> {
        search_boundary_targets_impl(self, filter, limit)
    }

    /// Queries the active Boundary token source (`app` vs `cli`) and updates status.
    pub fn refresh_boundary_status(&self) -> Result<FunctionStatus, CoreError> {
        refresh_boundary_status_impl(self)
    }

    /// Starts supervised Boundary authentication when supported in headless CLI mode.
    pub fn start_boundary_auth(
        &self,
        cancel: &CancelToken,
    ) -> Result<StartFunctionResult, CoreError>
    where
        B: 'static,
    {
        start_boundary_auth_impl(self, cancel)
    }

    /// Stops this runtime's active Boundary authentication process idempotently.
    pub fn stop_boundary_auth(&self, timeout: Duration) -> Result<bool, CoreError> {
        self.stop_supervised_function(BOUNDARY_AUTH_FUNCTION_ID, timeout)
    }

    /// Probes the host system for an installed OpenVPN executable.
    pub fn detect_openvpn(&self) -> Result<Option<String>, CoreError> {
        detect_openvpn_impl(self)
    }

    /// Validates, acquires host `vpn.lock`, elevates via `sudo -n` on Unix, and connects OpenVPN.
    pub fn start_openvpn(&self, cancel: &CancelToken) -> Result<StartFunctionResult, CoreError>
    where
        B: 'static,
    {
        start_openvpn_impl(self, cancel)
    }

    /// Stops an owned OpenVPN session idempotently and releases its `vpn.lock`.
    pub fn stop_openvpn(&self, timeout: Duration) -> Result<bool, CoreError> {
        stop_openvpn_impl(self, timeout)
    }

    /// Starts Caddy in the specified folder (or syncSettings default) and awaits readiness.
    pub fn start_caddy(
        &self,
        folder_override: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<StartFunctionResult, CoreError> {
        start_caddy_impl(self, folder_override, cancel)
    }

    /// Stops an owned Caddy session idempotently.
    pub fn stop_caddy(&self, timeout: Duration) -> Result<bool, CoreError> {
        stop_caddy_impl(self, timeout)
    }

    /// Starts an asynchronous host-owned SCP upload and correlates `transfer_id` with `session_id`.
    pub fn start_scp_upload(
        &self,
        server_id: &str,
        local_path: &str,
        remote_dir: &str,
        recursive: bool,
        cancel: &CancelToken,
    ) -> Result<ScpAsyncStartResult, CoreError> {
        start_scp_upload_impl(self, server_id, local_path, remote_dir, recursive, cancel)
    }

    /// Starts an asynchronous host-owned SCP download and correlates `transfer_id` with `session_id`.
    pub fn start_scp_download(
        &self,
        req: ScpDownloadRequest<'_>,
        cancel: &CancelToken,
    ) -> Result<ScpAsyncStartResult, CoreError> {
        let (spec, _lease) = prepare_download_spec(self, &req, true, cancel)?;
        start_scp_async(self, spec, cancel)
    }

    /// Opens a persistent SFTP remote browser session for `server_id`.
    pub fn open_remote_browser(
        &self,
        server_id: &str,
        root_path: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<RemoteBrowseSession, CoreError> {
        open_remote_browser_impl(self, server_id, root_path, cancel)
    }

    /// Closes a remote browser session idempotently.
    pub fn close_remote_browser(&self, browser_id: &str) -> Result<bool, CoreError> {
        Ok(close_remote_browser_impl(self, browser_id, "closed"))
    }

    /// Lists entries in a directory via an open remote browser session.
    pub fn list_remote_children(
        &self,
        browser_id: &str,
        path: &str,
    ) -> Result<Vec<RemoteBrowseEntry>, CoreError> {
        list_remote_children_impl(&self.remote_browsers, browser_id, path)
    }

    /// Stats a remote path via an open remote browser session.
    pub fn stat_remote_path(
        &self,
        browser_id: &str,
        path: &str,
    ) -> Result<RemoteBrowseNodeInfo, CoreError> {
        stat_remote_path_impl(&self.remote_browsers, browser_id, path)
    }

    /// Creates a remote directory via an open remote browser session.
    pub fn create_remote_directory(
        &self,
        browser_id: &str,
        path: &str,
    ) -> Result<RemoteBrowseNodeInfo, CoreError> {
        create_remote_directory_impl(&self.remote_browsers, browser_id, path)
    }

    /// Deletes a remote file or directory via an open remote browser session.
    pub fn delete_remote_path(
        &self,
        browser_id: &str,
        path: &str,
        recursive: bool,
    ) -> Result<(), CoreError> {
        delete_remote_path_impl(&self.remote_browsers, browser_id, path, recursive)
    }

    /// Stops all active browsers, transfers, SSH sessions, tunnels, Caddy, Boundary, and VPN in order.
    pub fn shutdown(&self, timeout: Duration) {
        if self.shutdown_done.swap(true, Ordering::SeqCst) {
            return;
        }
        {
            let mut st = self.runtime_state.lock();
            st.shutting_down = true;
            for sid in st
                .function_statuses
                .values()
                .filter_map(|s| s.session_id.clone())
                .collect::<Vec<_>>()
            {
                st.explicit_stops.insert(sid);
            }
        }
        close_all_browsers_impl(self, "runtime_shutdown");
        self.supervisor.shutdown_all(SHUTDOWN_KIND_ORDER, timeout);
        *self.vpn_lease.lock() = None;
        let mut st = self.runtime_state.lock();
        st.reserved_functions.clear();
        st.reserved_ports.clear();
        st.explicit_stops.clear();
    }
}

impl<B: KeyringBackend> Drop for ProfileRuntime<B> {
    fn drop(&mut self) {
        self.shutdown(Duration::from_millis(400));
    }
}

fn reserve_port_in_state(st: &mut RuntimeState, port: u16, owner: &str) -> Result<(), CoreError> {
    if port == 0 {
        return Err(CoreError::invalid_arguments(
            "invalid_port",
            "Port must be between 1 and 65535",
        ));
    }
    if st.shutting_down {
        return Err(CoreError::conflict(
            "shutting_down",
            "Runtime is shutting down",
            false,
            "restart_runtime",
        ));
    }
    if st.reserved_ports.contains_key(&port) {
        return Err(CoreError::conflict(
            "port_in_use",
            format!("Port {port} is already reserved in this runtime"),
            false,
            "stop_or_change_port",
        ));
    }
    if let Err(err) = TcpListener::bind(("127.0.0.1", port)) {
        if err.kind() == std::io::ErrorKind::AddrInUse {
            return Err(CoreError::conflict(
                "port_in_use",
                format!("Port {port} is already in use on loopback"),
                false,
                "stop_or_change_port",
            ));
        }
    }
    st.reserved_ports.insert(port, owner.to_string());
    Ok(())
}

fn reserve_launch_slots(
    state: &Arc<Mutex<RuntimeState>>,
    function_id: &str,
    port: Option<u16>,
) -> Result<(), CoreError> {
    let mut st = state.lock();
    if st.shutting_down {
        return Err(CoreError::conflict(
            "shutting_down",
            "Runtime is shutting down",
            false,
            "restart_runtime",
        ));
    }
    if st.reserved_functions.contains(function_id)
        || st
            .function_statuses
            .get(function_id)
            .is_some_and(|s| s.state == "running" || s.state == "starting")
    {
        return Err(CoreError::conflict(
            "function_already_running",
            format!("Function {function_id} is already running"),
            false,
            "stop_existing_session",
        ));
    }
    if let Some(p) = port {
        reserve_port_in_state(&mut st, p, function_id)?;
    }
    st.reserved_functions.insert(function_id.to_string());
    Ok(())
}

fn release_launch_slots(state: &Arc<Mutex<RuntimeState>>, function_id: &str, port: Option<u16>) {
    let mut st = state.lock();
    if !function_id.is_empty() {
        st.reserved_functions.remove(function_id);
    }
    if let Some(p) = port {
        st.reserved_ports.remove(&p);
    }
}

fn try_accept_launch(
    state: &Arc<Mutex<RuntimeState>>,
    cancel: &CancelToken,
    function_id: &str,
    port: Option<u16>,
    session_id: &str,
    initial_message: &str,
) -> Option<FunctionStatus> {
    let mut st = state.lock();
    if cancel.is_cancelled() || st.shutting_down {
        st.reserved_functions.remove(function_id);
        if let Some(p) = port {
            st.reserved_ports.remove(&p);
        }
        return None;
    }
    let token_source = st
        .function_statuses
        .get(function_id)
        .and_then(|s| s.boundary_token_source.clone());
    let status = FunctionStatus {
        function_id: function_id.to_string(),
        state: "running".to_string(),
        message: initial_message.to_string(),
        boundary_token_source: token_source,
        session_id: Some(session_id.to_string()),
        updated_at_ms: now_ms(),
    };
    st.function_statuses
        .insert(function_id.to_string(), status.clone());
    Some(status)
}

fn handle_supervised_exit(
    state: &Arc<Mutex<RuntimeState>>,
    sink: &Arc<dyn EventSink>,
    function_id: &str,
    port: Option<u16>,
    session_id: &str,
    exit_code: Option<i32>,
    reason: &str,
) {
    let status_event = {
        let mut st = state.lock();
        st.reserved_functions.remove(function_id);
        if let Some(p) = port {
            st.reserved_ports.remove(&p);
        }
        let was_stopped = st.explicit_stops.remove(session_id);
        update_exit_status(
            &mut st,
            function_id,
            session_id,
            was_stopped,
            exit_code,
            reason,
        )
    };
    sink.emit(DomainEvent::ProcessExited {
        id: session_id.to_string(),
        exit_code,
        reason: reason.to_string(),
    });
    if let Some(status) = status_event {
        sink.emit(DomainEvent::FunctionStatusChanged { status });
    }
}

fn update_exit_status(
    st: &mut RuntimeState,
    function_id: &str,
    session_id: &str,
    was_stopped: bool,
    exit_code: Option<i32>,
    reason: &str,
) -> Option<FunctionStatus> {
    let is_static = matches!(
        function_id,
        OPENVPN_FUNCTION_ID | CADDY_FUNCTION_ID | BOUNDARY_AUTH_FUNCTION_ID
    );
    let existing = st.function_statuses.get(function_id);
    if existing.and_then(|s| s.session_id.as_deref()) != Some(session_id) {
        return None;
    }
    let clean_stop = was_stopped || exit_code == Some(0);
    if !is_static && clean_stop {
        st.function_statuses.remove(function_id);
        return None;
    }
    let preserve_auth = function_id == BOUNDARY_AUTH_FUNCTION_ID
        && clean_stop
        && !was_stopped
        && existing.is_some_and(|s| s.state == "authenticated");
    let next_state = if preserve_auth {
        "authenticated"
    } else if clean_stop {
        "idle"
    } else {
        "error"
    };
    let message = if preserve_auth {
        existing.map(|s| s.message.clone()).unwrap_or_default()
    } else if was_stopped {
        "Stopped".to_string()
    } else if exit_code == Some(0) {
        "Ready".to_string()
    } else {
        reason.to_string()
    };
    let token_source = existing.and_then(|s| s.boundary_token_source.clone());
    let updated = FunctionStatus {
        function_id: function_id.to_string(),
        state: next_state.to_string(),
        message,
        boundary_token_source: token_source,
        session_id: None,
        updated_at_ms: now_ms(),
    };
    st.function_statuses
        .insert(function_id.to_string(), updated.clone());
    Some(updated)
}

struct ReservedSpawn<'a> {
    launch: ChildLaunch,
    port: Option<u16>,
    initial_message: &'a str,
    hooks: SuperviseHooks,
}

fn wrap_exit_callback(
    state: Arc<Mutex<RuntimeState>>,
    sink: Arc<dyn EventSink>,
    function_id: String,
    port: Option<u16>,
    extra_exit: Option<crate::supervisor::ExitCallback>,
) -> impl Fn(&str, Option<i32>, &str) + Send + Sync + 'static {
    move |session_id, exit_code, reason| {
        if let Some(cb) = &extra_exit {
            cb(session_id, exit_code, reason);
        }
        handle_supervised_exit(
            &state,
            &sink,
            &function_id,
            port,
            session_id,
            exit_code,
            reason,
        );
    }
}

fn wrap_output_callback(
    sink: Arc<dyn EventSink>,
    extra_out: Option<crate::supervisor::OutputCallback>,
) -> crate::supervisor::OutputCallback {
    Arc::new(move |id, data, is_pty| {
        sink.emit(DomainEvent::ProcessOutput {
            id: id.to_string(),
            data: data.to_string(),
            is_pty,
        });
        if let Some(cb) = &extra_out {
            cb(id, data, is_pty);
        }
    })
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(crate) fn update_active_session_status(
    state: &Arc<Mutex<RuntimeState>>,
    sink: &Arc<dyn EventSink>,
    update: StatusUpdate<'_>,
    expected_session_id: &str,
) {
    let status = {
        let mut st = state.lock();
        let active = st
            .function_statuses
            .get(update.function_id)
            .and_then(|s| s.session_id.as_deref());
        if active != Some(expected_session_id) {
            return;
        }
        let preserved = st
            .function_statuses
            .get(update.function_id)
            .and_then(|s| s.boundary_token_source.clone());
        let entry = FunctionStatus {
            function_id: update.function_id.to_string(),
            state: update.state.to_string(),
            message: update.message,
            boundary_token_source: update.boundary_token_source.unwrap_or(preserved),
            session_id: update.session_id,
            updated_at_ms: now_ms(),
        };
        st.function_statuses
            .insert(update.function_id.to_string(), entry.clone());
        entry
    };
    sink.emit(DomainEvent::FunctionStatusChanged { status });
}

pub(crate) struct SuperviseHooks {
    pub(crate) on_output: Option<OutputCallback>,
    pub(crate) on_exit: Option<ExitCallback>,
}

pub(crate) struct StatusUpdate<'a> {
    pub(crate) function_id: &'a str,
    pub(crate) state: &'a str,
    pub(crate) message: String,
    pub(crate) boundary_token_source: Option<Option<String>>,
    pub(crate) session_id: Option<String>,
}

fn require_non_empty<'a>(
    val: &'a str,
    reason: &'static str,
    message: &str,
) -> Result<&'a str, CoreError> {
    if val.trim().is_empty() {
        return Err(CoreError::invalid_arguments(reason, message));
    }
    Ok(val)
}

fn map_transfer_err(err: String) -> CoreError {
    CoreError::operation_failed("transfer_store_error", err, false, "retry_later")
}

fn resolve_server_cursor(servers: &[Server], after: Option<&str>) -> Result<usize, CoreError> {
    match after {
        None => Ok(0),
        Some(cursor) if cursor.trim().is_empty() => Err(CoreError::invalid_arguments(
            "invalid_pagination",
            "after cursor must not be empty",
        )),
        Some(cursor) => servers
            .iter()
            .position(|s| s.id == cursor)
            .map(|idx| idx + 1)
            .ok_or_else(|| {
                CoreError::invalid_arguments(
                    "invalid_pagination",
                    format!("Unknown after cursor: {cursor}"),
                )
            }),
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
