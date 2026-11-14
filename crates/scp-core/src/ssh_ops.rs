//! SSH endpoint resolution, strict known_hosts checks, Boundary tunnels, SSH, and SCP transfers.
use crate::error::CoreError;
use crate::events::{DomainEvent, EventSink};
use crate::file_paths::{
    normalize_remote_transfer_path, validate_local_download_target, validate_local_upload_source,
};
use crate::models::{
    now_ms, AppConfig, ConnectionMethod, ScpAsyncStartResult, ScpDownloadRequest, ScpTransferEntry,
    ScpTransferState, Server, SshEndpoint, StartFunctionResult, SyncSettings,
    KEYRING_BOUNDARY_APP_TOKEN,
};
use crate::runtime::ProfileRuntime;
use crate::secrets::KeyringBackend;
use crate::supervisor::{AcceptOutcome, CancelToken, ChildLaunch, ProcessSupervisor, SpawnMode};
use crate::transfer_store::{
    attach_scp_transfer_session, complete_scp_transfer_session, create_scp_transfer_entry,
    get_scp_transfer_entry, handle_scp_transfer_output, update_scp_transfer_entry,
    ScpTransferStore,
};
use ssh2::{CheckResult, KnownHostFileKind, Session as SshSession};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEFAULT_BOUNDARY_LISTEN_PORT: u16 = 6222;
const DEFAULT_TUNNEL_READY_TIMEOUT: Duration = Duration::from_secs(12);
const DEFAULT_SSH_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// Function ID used for a server's Boundary tunnel.
pub fn server_tunnel_function_id(server_id: &str) -> String {
    format!("server-tunnel-{server_id}")
}

/// Function ID used for a server's interactive SSH PTY session.
pub fn server_ssh_function_id(server_id: &str) -> String {
    format!("server-ssh-{server_id}")
}

/// Resolves SSH endpoint host, port, and user for a server record.
pub fn resolve_server_ssh_endpoint(
    server: &Server,
    operation: &str,
) -> Result<SshEndpoint, String> {
    if server.ssh_user.trim().is_empty() {
        return Err(format!("SSH user is required for {operation}"));
    }
    match &server.connection_method {
        ConnectionMethod::Ssh => {
            if server.host.trim().is_empty() {
                return Err(format!("Host is required for {operation}"));
            }
            Ok(SshEndpoint {
                user: server.ssh_user.trim().to_string(),
                host: server.host.trim().to_string(),
                port: server.ssh_port,
            })
        }
        ConnectionMethod::Boundary => Ok(SshEndpoint {
            user: server.ssh_user.trim().to_string(),
            host: "127.0.0.1".to_string(),
            port: if server.boundary_listen_port == 0 {
                DEFAULT_BOUNDARY_LISTEN_PORT
            } else {
                server.boundary_listen_port
            },
        }),
    }
}

/// Formats `user@host` for OpenSSH `ssh` and `scp` invocations.
pub fn ssh_target(endpoint: &SshEndpoint) -> String {
    format!("{}@{}", endpoint.user, endpoint.host)
}

/// Resolves the Boundary CLI executable path from local settings, env, or `PATH`.
pub(crate) fn resolve_boundary_executable(sync: &SyncSettings) -> Result<String, CoreError> {
    let configured = sync.boundary_cli_path.trim();
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    if let Ok(env_path) = std::env::var("SCPCTL_BOUNDARY_PATH") {
        if !env_path.trim().is_empty() {
            return Ok(env_path.trim().to_string());
        }
    }
    Ok("boundary".to_string())
}

/// Builds Boundary CLI environment pairs isolated to the profile namespace.
pub(crate) fn build_boundary_env(
    cfg: &AppConfig,
    namespace: &str,
    token: Option<&str>,
) -> (Vec<(String, String)>, Vec<String>) {
    let mut env = vec![
        ("BOUNDARY_SKIP_CACHE_DAEMON".to_string(), "true".to_string()),
        (
            "BOUNDARY_TOKEN_NAME".to_string(),
            format!("scpctl-{namespace}"),
        ),
    ];
    let mut remove = Vec::new();
    if !cfg.boundary.addr.trim().is_empty() {
        env.push((
            "BOUNDARY_ADDR".to_string(),
            cfg.boundary.addr.trim().to_string(),
        ));
    }
    if let Some(tok) = token.map(str::trim).filter(|t| !t.is_empty()) {
        env.push(("BOUNDARY_TOKEN".to_string(), tok.to_string()));
        env.push(("BOUNDARY_KEYRING_TYPE".to_string(), "none".to_string()));
    } else {
        remove.push("BOUNDARY_TOKEN".to_string());
    }
    (env, remove)
}

/// Strictly verifies a remote SSH host key against OpenSSH `known_hosts`.
pub fn verify_ssh_host_key(
    host: &str,
    port: u16,
    known_hosts_path: Option<&Path>,
) -> Result<(), CoreError> {
    let path = known_hosts_path
        .map(PathBuf::from)
        .or_else(default_known_hosts_path)
        .ok_or_else(|| host_key_untrusted("No SSH known_hosts path available".into()))?;
    if !path.is_file() {
        return Err(host_key_untrusted(format!(
            "SSH known_hosts file missing: {}",
            path.display()
        )));
    }
    let tcp = connect_tcp_timeout(host, port, DEFAULT_SSH_CONNECT_TIMEOUT)?;
    let mut session = SshSession::new().map_err(|e| {
        CoreError::operation_failed("ssh_init_failed", e.to_string(), false, "retry_later")
    })?;
    session.set_tcp_stream(tcp);
    session.handshake().map_err(|e| {
        CoreError::operation_failed(
            "ssh_handshake_failed",
            format!("SSH handshake with {host}:{port} failed: {e}"),
            true,
            "inspect_network",
        )
    })?;
    verify_session_host_key(&session, host, port, &path)
}

pub(crate) fn verify_session_host_key(
    session: &SshSession,
    host: &str,
    port: u16,
    known_hosts_path: &Path,
) -> Result<(), CoreError> {
    let (key, _) = session
        .host_key()
        .ok_or_else(|| host_key_untrusted("Remote peer presented no SSH host key".into()))?;
    let mut kh = session
        .known_hosts()
        .map_err(|e| host_key_untrusted(format!("Unable to initialize known_hosts: {e}")))?;
    kh.read_file(known_hosts_path, KnownHostFileKind::OpenSSH)
        .map_err(|e| host_key_untrusted(format!("Unable to read known_hosts: {e}")))?;
    match kh.check_port(host, port, key) {
        CheckResult::Match => Ok(()),
        CheckResult::Mismatch => Err(host_key_untrusted(format!(
            "SSH host key mismatch for {host}:{port}"
        ))),
        CheckResult::NotFound => Err(host_key_untrusted(format!(
            "SSH host key for {host}:{port} is not trusted in known_hosts"
        ))),
        CheckResult::Failure => Err(host_key_untrusted(format!(
            "SSH host key verification failed for {host}:{port}"
        ))),
    }
}

pub(crate) fn connect_tcp_timeout(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<TcpStream, CoreError> {
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| {
            CoreError::operation_failed(
                "ssh_connect_failed",
                format!("Unable to resolve {host}:{port}: {e}"),
                true,
                "inspect_network",
            )
        })?
        .collect();
    let addr = addrs.into_iter().next().ok_or_else(|| {
        CoreError::operation_failed(
            "ssh_connect_failed",
            format!("No socket addresses for {host}:{port}"),
            true,
            "inspect_network",
        )
    })?;
    TcpStream::connect_timeout(&addr, timeout).map_err(|e| {
        CoreError::operation_failed(
            "ssh_connect_failed",
            format!("SSH TCP connect to {host}:{port} failed: {e}"),
            true,
            "inspect_network",
        )
    })
}

pub(crate) fn host_key_untrusted(message: String) -> CoreError {
    CoreError::capability_unavailable("host_key_untrusted", message, "verify_host_key")
}

pub(crate) fn default_known_hosts_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SCPCTL_SSH_KNOWN_HOSTS") {
        if !path.trim().is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    home_dir_from_env().map(|h| h.join(".ssh").join("known_hosts"))
}

pub(crate) fn default_identity_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SCPCTL_SSH_IDENTITY_FILE") {
        if !path.trim().is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    None
}

pub(crate) fn home_dir_from_env() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub(crate) struct TunnelLease<'a, B: KeyringBackend> {
    rt: &'a ProfileRuntime<B>,
    pub(crate) server_id: String,
    stop_on_drop: bool,
}

impl<B: KeyringBackend> TunnelLease<'_, B> {
    pub(crate) fn is_temporary(&self) -> bool {
        self.stop_on_drop
    }

    pub(crate) fn disarm(&mut self) {
        self.stop_on_drop = false;
    }
}

impl<B: KeyringBackend> Drop for TunnelLease<'_, B> {
    fn drop(&mut self) {
        if self.stop_on_drop {
            let _ = self
                .rt
                .stop_server_tunnel(&self.server_id, Duration::from_secs(2));
        }
    }
}

pub(crate) fn start_server_tunnel_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    server_id: &str,
    cancel: &CancelToken,
) -> Result<StartFunctionResult, CoreError> {
    let server = rt.get_server(server_id)?;
    if server.connection_method != ConnectionMethod::Boundary {
        return Err(CoreError::invalid_arguments(
            "tunnel_not_applicable",
            "Tunnel is only applicable to Boundary servers",
        ));
    }
    if server.boundary_target_id.trim().is_empty() {
        return Err(CoreError::invalid_arguments(
            "boundary_target_required",
            "Boundary target ID is required",
        ));
    }
    let endpoint = resolve_server_ssh_endpoint(&server, "server tunnel")
        .map_err(|m| CoreError::invalid_arguments("invalid_ssh_endpoint", m))?;
    let launch = build_boundary_tunnel_launch(rt, &server, endpoint.port)?;
    let function_id = launch.function_id.clone();
    let session_id = rt.start_supervised_function(
        launch,
        cancel,
        Some(endpoint.port),
        &format!("Connecting Boundary tunnel on 127.0.0.1:{}", endpoint.port),
    )?;
    let ready_timeout = rt
        .tunnel_ready_timeout_override()
        .unwrap_or(DEFAULT_TUNNEL_READY_TIMEOUT);
    match wait_for_tunnel_ready(
        rt,
        &function_id,
        &session_id,
        endpoint.port,
        ready_timeout,
        cancel,
    ) {
        Ok(()) => Ok(StartFunctionResult {
            function_id,
            session_id,
        }),
        Err(err) => {
            let _ = rt.stop_supervised_function(&function_id, Duration::from_secs(2));
            Err(err)
        }
    }
}

fn build_boundary_tunnel_launch<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    server: &Server,
    listen_port: u16,
) -> Result<ChildLaunch, CoreError> {
    let sync = rt.get_sync_settings()?;
    let cfg = rt.get_app_config()?;
    let exe = resolve_boundary_executable(&sync)?;
    let token = rt.read_optional_secret(KEYRING_BOUNDARY_APP_TOKEN);
    let (env, env_remove) = build_boundary_env(&cfg, rt.credential_namespace(), token.as_deref());
    let mut args = vec!["connect".to_string()];
    if token.as_deref().is_some_and(|t| !t.trim().is_empty()) {
        args.push("-token".to_string());
        args.push("env://BOUNDARY_TOKEN".to_string());
    }
    args.extend([
        "-target-id".to_string(),
        server.boundary_target_id.trim().to_string(),
        "-listen-port".to_string(),
        listen_port.to_string(),
    ]);
    Ok(ChildLaunch {
        function_id: server_tunnel_function_id(&server.id),
        program: exe,
        args,
        cwd: None,
        env,
        env_remove,
        mode: SpawnMode::Pipe,
        kill_tree: true,
        kind: "tunnel",
    })
}

fn wait_for_tunnel_ready<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    function_id: &str,
    session_id: &str,
    port: u16,
    timeout: Duration,
    cancel: &CancelToken,
) -> Result<(), CoreError> {
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.is_cancelled() {
            return Err(CoreError::interrupted("Boundary tunnel cancelled"));
        }
        if TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(80))
            .is_ok()
        {
            return Ok(());
        }
        if rt
            .supervisor()
            .active_session_for_function(function_id)
            .as_deref()
            != Some(session_id)
        {
            return Err(CoreError::operation_failed(
                "tunnel_exited_early",
                "Boundary tunnel exited before local port became ready",
                false,
                "inspect_boundary_logs",
            ));
        }
        if Instant::now() >= deadline {
            return Err(CoreError::timeout(
                "tunnel_ready_timeout",
                format!("Boundary tunnel did not listen on 127.0.0.1:{port} within timeout"),
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub(crate) fn acquire_endpoint_for_server<'a, B: KeyringBackend>(
    rt: &'a ProfileRuntime<B>,
    server: &Server,
    operation: &str,
    cancel: &CancelToken,
) -> Result<(SshEndpoint, TunnelLease<'a, B>), CoreError> {
    let endpoint = resolve_server_ssh_endpoint(server, operation)
        .map_err(|m| CoreError::invalid_arguments("invalid_ssh_endpoint", m))?;
    if server.connection_method == ConnectionMethod::Ssh {
        return Ok((
            endpoint,
            TunnelLease {
                rt,
                server_id: server.id.clone(),
                stop_on_drop: false,
            },
        ));
    }
    let fid = server_tunnel_function_id(&server.id);
    if rt.supervisor().active_session_for_function(&fid).is_some() {
        return Ok((
            endpoint,
            TunnelLease {
                rt,
                server_id: server.id.clone(),
                stop_on_drop: false,
            },
        ));
    }
    let _started = start_server_tunnel_impl(rt, &server.id, cancel)?;
    Ok((
        endpoint,
        TunnelLease {
            rt,
            server_id: server.id.clone(),
            stop_on_drop: true,
        },
    ))
}

pub(crate) struct ScpJobSpec {
    title: String,
    src: String,
    dst: String,
    args: Vec<String>,
}

fn validate_upload_inputs(
    local_path: &str,
    remote_dir: &str,
    recursive: bool,
) -> Result<(String, String, bool), CoreError> {
    let local = validate_local_upload_source(local_path).map_err(|e| e.into_core_error())?;
    let remote = normalize_remote_transfer_path(remote_dir, Some("/"), "invalid_remote_path", true)
        .map_err(|e| e.into_core_error())?;
    let is_dir = Path::new(&local).is_dir();
    if is_dir && !recursive {
        return Err(CoreError::invalid_arguments(
            "recursive_required",
            "Uploading a directory requires recursive=true",
        ));
    }
    Ok((local, remote, recursive || is_dir))
}

fn validate_download_inputs(req: &ScpDownloadRequest<'_>) -> Result<(String, String), CoreError> {
    let remote =
        normalize_remote_transfer_path(req.remote_path, None, "invalid_remote_path", false)
            .map_err(|e| e.into_core_error())?;
    let local =
        validate_local_download_target(req.local_target).map_err(|e| e.into_core_error())?;
    if Path::new(&local).exists() && !req.overwrite {
        return Err(CoreError::conflict(
            "destination_exists",
            format!("Local destination already exists: {local}"),
            false,
            "pass_overwrite_flag",
        ));
    }
    Ok((remote, local))
}

fn require_running_endpoint<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    server: &Server,
    op: &str,
) -> Result<SshEndpoint, CoreError> {
    let endpoint = resolve_server_ssh_endpoint(server, op)
        .map_err(|m| CoreError::invalid_arguments("invalid_ssh_endpoint", m))?;
    if server.connection_method == ConnectionMethod::Boundary {
        let fid = server_tunnel_function_id(&server.id);
        if rt.supervisor().active_session_for_function(&fid).is_none() {
            return Err(CoreError::conflict(
                "tunnel_not_running",
                "Boundary tunnel must be running before starting an async SCP transfer",
                false,
                "start_server_tunnel",
            ));
        }
    }
    Ok(endpoint)
}

fn build_verified_scp_args<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    endpoint: &SshEndpoint,
    local_path: &str,
    remote_path: &str,
    recursive: bool,
    download: bool,
) -> Result<Vec<String>, CoreError> {
    let (kh_path, id_path) = rt.resolved_ssh_paths();
    verify_ssh_host_key(&endpoint.host, endpoint.port, kh_path.as_deref())?;
    let mut args = build_scp_transport_args(endpoint, kh_path.as_deref(), id_path.as_deref())?;
    if recursive {
        args.push("-r".to_string());
    }
    let remote_spec = format!("{}:{}", ssh_target(endpoint), remote_path);
    if download {
        args.extend([remote_spec, local_path.to_string()]);
    } else {
        args.extend([local_path.to_string(), remote_spec]);
    }
    Ok(args)
}

fn build_scp_transport_args(
    endpoint: &SshEndpoint,
    known_hosts: Option<&Path>,
    identity_file: Option<&Path>,
) -> Result<Vec<String>, CoreError> {
    let kh = known_hosts.ok_or_else(|| host_key_untrusted("Missing known_hosts path".into()))?;
    let mut args = vec![
        "-P".to_string(),
        endpoint.port.to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=yes".to_string(),
        "-o".to_string(),
        format!("UserKnownHostsFile={}", kh.display()),
    ];
    if let Some(id) = identity_file {
        args.extend([
            "-i".to_string(),
            id.to_string_lossy().into_owned(),
            "-o".to_string(),
            "IdentitiesOnly=yes".to_string(),
        ]);
    }
    Ok(args)
}

fn prepare_scp_endpoint<'a, B: KeyringBackend>(
    rt: &'a ProfileRuntime<B>,
    server: &Server,
    op: &str,
    async_mode: bool,
    cancel: &CancelToken,
) -> Result<(SshEndpoint, Option<TunnelLease<'a, B>>), CoreError> {
    if async_mode {
        Ok((require_running_endpoint(rt, server, op)?, None))
    } else {
        let (ep, lease) = acquire_endpoint_for_server(rt, server, op, cancel)?;
        Ok((ep, Some(lease)))
    }
}

fn prepare_upload_spec<'a, B: KeyringBackend>(
    rt: &'a ProfileRuntime<B>,
    server_id: &str,
    paths: (&str, &str, bool),
    async_mode: bool,
    cancel: &CancelToken,
) -> Result<(ScpJobSpec, Option<TunnelLease<'a, B>>), CoreError> {
    let (local, remote, rec) = validate_upload_inputs(paths.0, paths.1, paths.2)?;
    let server = rt.get_server(server_id)?;
    let op = if async_mode {
        "scp_start_upload"
    } else {
        "scp_upload"
    };
    let (endpoint, lease) = prepare_scp_endpoint(rt, &server, op, async_mode, cancel)?;
    let args = build_verified_scp_args(rt, &endpoint, &local, &remote, rec, false)?;
    let spec = ScpJobSpec {
        title: format!("Upload to {}", server.name),
        src: local,
        dst: remote,
        args,
    };
    Ok((spec, lease))
}

pub(crate) fn prepare_download_spec<'a, B: KeyringBackend>(
    rt: &'a ProfileRuntime<B>,
    req: &ScpDownloadRequest<'_>,
    async_mode: bool,
    cancel: &CancelToken,
) -> Result<(ScpJobSpec, Option<TunnelLease<'a, B>>), CoreError> {
    let (remote, local) = validate_download_inputs(req)?;
    let server = rt.get_server(req.server_id)?;
    let op = if async_mode {
        "scp_start_download"
    } else {
        "scp_download"
    };
    let (endpoint, lease) = prepare_scp_endpoint(rt, &server, op, async_mode, cancel)?;
    let args = build_verified_scp_args(rt, &endpoint, &local, &remote, req.recursive, true)?;
    let spec = ScpJobSpec {
        title: format!("Download from {}", server.name),
        src: remote,
        dst: local,
        args,
    };
    Ok((spec, lease))
}

pub(crate) fn scp_upload_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    server_id: &str,
    local_path: &str,
    remote_dir: &str,
    recursive: bool,
    cancel: &CancelToken,
) -> Result<ScpTransferEntry, CoreError> {
    let (spec, _lease) = prepare_upload_spec(
        rt,
        server_id,
        (local_path, remote_dir, recursive),
        false,
        cancel,
    )?;
    run_scp_sync(rt, spec, cancel)
}

pub(crate) fn start_scp_upload_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    server_id: &str,
    local_path: &str,
    remote_dir: &str,
    recursive: bool,
    cancel: &CancelToken,
) -> Result<ScpAsyncStartResult, CoreError> {
    let (spec, _lease) = prepare_upload_spec(
        rt,
        server_id,
        (local_path, remote_dir, recursive),
        true,
        cancel,
    )?;
    start_scp_async(rt, spec, cancel)
}

pub(crate) fn start_scp_async<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    spec: ScpJobSpec,
    cancel: &CancelToken,
) -> Result<ScpAsyncStartResult, CoreError> {
    if cancel.is_cancelled() {
        return Err(CoreError::interrupted("SCP transfer cancelled"));
    }
    let store = Arc::clone(rt.transfer_store());
    let sink = Arc::clone(rt.event_sink());
    let transfer_id = create_scp_transfer_entry(
        &store,
        now_ms,
        spec.title,
        "Starting transfer",
        Some(spec.src),
        Some(spec.dst),
    )
    .map_err(|e| CoreError::operation_failed("transfer_store_error", e, false, "retry_later"))?;
    match spawn_scp_session(
        rt.supervisor(),
        &store,
        &sink,
        &transfer_id,
        spec.args,
        cancel,
    ) {
        Ok(session_id) => Ok(ScpAsyncStartResult {
            transfer_id,
            session_id,
        }),
        Err(err) => {
            let _ = update_scp_transfer_entry(
                &store,
                &transfer_id,
                ScpTransferState::Error,
                &err.message,
                None,
                now_ms,
            );
            Err(err)
        }
    }
}

fn spawn_scp_session(
    supervisor: &ProcessSupervisor,
    store: &Arc<ScpTransferStore>,
    sink: &Arc<dyn EventSink>,
    transfer_id: &str,
    args: Vec<String>,
    cancel: &CancelToken,
) -> Result<String, CoreError> {
    let launch = ChildLaunch {
        function_id: format!("scp-transfer-{transfer_id}"),
        program: "scp".to_string(),
        args,
        cwd: None,
        env: Vec::new(),
        env_remove: Vec::new(),
        mode: SpawnMode::Pipe,
        kill_tree: true,
        kind: "transfer",
    };
    let store_exit = Arc::clone(store);
    let sink_exit = Arc::clone(sink);
    let tid_exit = transfer_id.to_string();
    let store_out = Arc::clone(store);
    let sink_out = Arc::clone(sink);
    let tid_out = transfer_id.to_string();

    let res = supervisor.spawn_with_gate(
        launch,
        cancel,
        |sid| {
            let _ = attach_scp_transfer_session(store, transfer_id, sid);
            if let Ok(Some(entry)) = get_scp_transfer_entry(store, transfer_id) {
                sink.emit(DomainEvent::TransferUpdated { entry });
            }
            AcceptOutcome::Accepted(())
        },
        move |sid, code, reason| {
            let _ = complete_scp_transfer_session(&store_exit, sid, code, reason, now_ms);
            if let Ok(Some(entry)) = get_scp_transfer_entry(&store_exit, &tid_exit) {
                sink_exit.emit(DomainEvent::TransferUpdated { entry });
            }
        },
        Some(Arc::new(move |sid, data, _is_pty| {
            let _ = handle_scp_transfer_output(&store_out, sid, data, now_ms);
            if let Ok(Some(entry)) = get_scp_transfer_entry(&store_out, &tid_out) {
                sink_out.emit(DomainEvent::TransferUpdated { entry });
            }
        })),
    )?;
    match res {
        AcceptOutcome::Accepted(s) => Ok(s),
        AcceptOutcome::CancelledBeforeAccept => {
            Err(CoreError::interrupted("SCP transfer cancelled"))
        }
    }
}

pub(crate) fn run_scp_sync<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    spec: ScpJobSpec,
    cancel: &CancelToken,
) -> Result<ScpTransferEntry, CoreError> {
    let started = start_scp_async(rt, spec, cancel)?;
    let _ = rt
        .supervisor()
        .wait_for_session_exit(&started.session_id, Duration::from_secs(300));
    let entry = get_scp_transfer_entry(rt.transfer_store(), &started.transfer_id)
        .map_err(|e| CoreError::operation_failed("transfer_store_error", e, false, "retry_later"))?
        .ok_or_else(|| {
            CoreError::operation_failed(
                "transfer_missing",
                "Transfer record missing",
                false,
                "retry_later",
            )
        })?;
    if entry.state == ScpTransferState::Success {
        Ok(entry)
    } else {
        Err(CoreError::operation_failed(
            "scp_transfer_failed",
            entry.detail,
            false,
            "inspect_transfer_output",
        ))
    }
}

/// Builds the OpenSSH interactive `ssh` command & argument list for a server.
pub(crate) fn prepare_ssh_launch<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    server: &Server,
    endpoint: &SshEndpoint,
) -> Result<(String, Vec<String>), CoreError> {
    let (kh_path, id_path) = rt.resolved_ssh_paths();
    let kh = kh_path
        .as_deref()
        .ok_or_else(|| host_key_untrusted("No SSH known_hosts path available".into()))?;
    verify_ssh_host_key(&endpoint.host, endpoint.port, Some(kh))?;
    let mut args = vec![
        "-p".to_string(),
        endpoint.port.to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=yes".to_string(),
        "-o".to_string(),
        format!("UserKnownHostsFile={}", kh.display()),
    ];
    if let Some(id) = id_path.as_deref() {
        args.push("-i".to_string());
        args.push(id.to_string_lossy().into_owned());
        args.push("-o".to_string());
        args.push("IdentitiesOnly=yes".to_string());
    }
    args.push(ssh_target(endpoint));
    if server.connection_method == ConnectionMethod::Boundary {
        let extra = rt.get_app_config()?.boundary.connect_ssh_options;
        for token in extra.split_whitespace() {
            args.push(token.to_string());
        }
    }
    Ok(("ssh".to_string(), args))
}
