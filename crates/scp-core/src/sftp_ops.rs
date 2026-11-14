//! Persistent SFTP remote browser store and POSIX file operations over `ssh2`.
use crate::error::CoreError;
use crate::events::{DomainEvent, EventSink};
use crate::file_paths::normalize_remote_transfer_path;
use crate::models::{
    keyring_server_ssh_password_account, RemoteBrowseEntry, RemoteBrowseNodeInfo,
    RemoteBrowseSession, RemotePathKind, SshEndpoint,
};
use crate::runtime::ProfileRuntime;
use crate::secrets::KeyringBackend;
use crate::ssh_ops::{
    acquire_endpoint_for_server, connect_tcp_timeout, home_dir_from_env, host_key_untrusted,
    verify_session_host_key,
};
use crate::supervisor::CancelToken;
use parking_lot::Mutex;
use ssh2::{ErrorCode as Ssh2ErrorCode, FileStat, FileType, Session as SshSession, Sftp};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

static NEXT_BROWSER_SEQ: AtomicU64 = AtomicU64::new(1);

struct BrowserSession {
    server_id: String,
    temp_tunnel: bool,
    _ssh: SshSession,
    sftp: Sftp,
}

/// Thread-safe registry of persistent SFTP remote browser sessions.
#[derive(Default)]
pub struct RemoteBrowserStore {
    sessions: Mutex<HashMap<String, Arc<Mutex<BrowserSession>>>>,
    server_to_browser: Mutex<HashMap<String, String>>,
}

impl RemoteBrowserStore {
    /// Creates an empty remote browser registry.
    pub fn new() -> Self {
        Self::default()
    }

    fn insert_session(
        &self,
        server_id: &str,
        browser_id: &str,
        session: BrowserSession,
    ) -> Option<(String, bool, String)> {
        let wrapped = Arc::new(Mutex::new(session));
        let prev_id = self
            .server_to_browser
            .lock()
            .insert(server_id.to_string(), browser_id.to_string());
        let replaced = prev_id.and_then(|old_id| {
            let entry = self.sessions.lock().remove(&old_id)?;
            let guard = entry.lock();
            Some((old_id, guard.temp_tunnel, guard.server_id.clone()))
        });
        self.sessions.lock().insert(browser_id.to_string(), wrapped);
        replaced
    }

    fn get_session(&self, browser_id: &str) -> Result<Arc<Mutex<BrowserSession>>, CoreError> {
        self.sessions
            .lock()
            .get(browser_id)
            .cloned()
            .ok_or_else(|| {
                CoreError::not_found(
                    "browser_not_found",
                    format!("Remote browser session not found: {browser_id}"),
                    "open_remote_browser",
                )
            })
    }

    fn remove_session(&self, browser_id: &str) -> Option<(String, bool)> {
        let removed = self.sessions.lock().remove(browser_id)?;
        let (server_id, temp_tunnel) = {
            let guard = removed.lock();
            (guard.server_id.clone(), guard.temp_tunnel)
        };
        let mut idx = self.server_to_browser.lock();
        if idx.get(&server_id).map(String::as_str) == Some(browser_id) {
            idx.remove(&server_id);
        }
        Some((server_id, temp_tunnel))
    }

    fn take_for_server(&self, server_id: &str) -> Option<(String, bool)> {
        let browser_id = self.server_to_browser.lock().remove(server_id)?;
        let removed = self.sessions.lock().remove(&browser_id)?;
        let temp_tunnel = removed.lock().temp_tunnel;
        Some((browser_id, temp_tunnel))
    }

    fn drain_all(&self) -> Vec<(String, String, bool)> {
        self.server_to_browser.lock().clear();
        let drained: Vec<(String, Arc<Mutex<BrowserSession>>)> =
            self.sessions.lock().drain().collect();
        drained
            .into_iter()
            .map(|(bid, entry)| {
                let guard = entry.lock();
                (bid, guard.server_id.clone(), guard.temp_tunnel)
            })
            .collect()
    }
}

/// Maps `ssh2::FileStat` POSIX permissions / type to `RemotePathKind`.
pub fn remote_kind_from_file_stat(stat: &FileStat) -> RemotePathKind {
    match stat.file_type() {
        FileType::Directory => RemotePathKind::Directory,
        FileType::RegularFile => RemotePathKind::File,
        FileType::Symlink => RemotePathKind::Symlink,
        _ => RemotePathKind::Other,
    }
}

pub(crate) fn open_remote_browser_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    server_id: &str,
    root_path: Option<&str>,
    cancel: &CancelToken,
) -> Result<RemoteBrowseSession, CoreError> {
    let norm_root = normalize_remote_transfer_path(
        root_path.unwrap_or("/"),
        Some("/"),
        "invalid_remote_path",
        true,
    )
    .map_err(|e| e.into_core_error())?;
    let server = rt.get_server(server_id)?;
    let (endpoint, mut lease) =
        acquire_endpoint_for_server(rt, &server, "open_remote_browser", cancel)?;
    let (kh_path, id_path) = rt.resolved_ssh_paths();
    let password = rt.read_optional_secret(&keyring_server_ssh_password_account(&server.id));
    let (ssh, sftp) = connect_and_open_sftp(
        &endpoint,
        kh_path.as_deref(),
        id_path.as_deref(),
        password.as_deref(),
    )?;
    if let Err(err) = sftp.readdir(Path::new(&norm_root)) {
        return Err(map_sftp_error(
            &format!("Cannot open root path {norm_root}"),
            err,
        ));
    }

    let browser_id = format!("sftp-{}", NEXT_BROWSER_SEQ.fetch_add(1, Ordering::Relaxed));
    let is_temp = lease.is_temporary();
    let entry = BrowserSession {
        server_id: server.id.clone(),
        temp_tunnel: is_temp,
        _ssh: ssh,
        sftp,
    };
    let replaced = rt
        .remote_browsers()
        .insert_session(&server.id, &browser_id, entry);
    let session = RemoteBrowseSession {
        browser_id,
        root_path: norm_root,
    };
    lease.disarm();
    if let Some((old_id, old_temp, old_srv)) = replaced {
        emit_closed_and_cleanup(rt, &old_id, &old_srv, old_temp, "replaced");
    }
    Ok(session)
}

fn emit_closed_and_cleanup<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    browser_id: &str,
    server_id: &str,
    close_temp_tunnel: bool,
    reason: &str,
) {
    let sink: &Arc<dyn EventSink> = rt.event_sink();
    sink.emit(DomainEvent::RemoteBrowserClosed {
        session_id: browser_id.to_string(),
        server_id: server_id.to_string(),
        reason: reason.to_string(),
    });
    if close_temp_tunnel && reason != "tunnel_stopped" {
        let _ = rt.stop_server_tunnel(server_id, Duration::from_secs(2));
    }
}

pub(crate) fn close_remote_browser_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    browser_id: &str,
    reason: &str,
) -> bool {
    let Some((server_id, temp_tunnel)) = rt.remote_browsers().remove_session(browser_id) else {
        return false;
    };
    emit_closed_and_cleanup(rt, browser_id, &server_id, temp_tunnel, reason);
    true
}

pub(crate) fn close_browsers_for_server_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    server_id: &str,
    reason: &str,
) {
    if let Some((browser_id, temp_tunnel)) = rt.remote_browsers().take_for_server(server_id) {
        emit_closed_and_cleanup(rt, &browser_id, server_id, temp_tunnel, reason);
    }
}

pub(crate) fn close_all_browsers_impl<B: KeyringBackend>(rt: &ProfileRuntime<B>, reason: &str) {
    for (browser_id, server_id, temp_tunnel) in rt.remote_browsers().drain_all() {
        emit_closed_and_cleanup(rt, &browser_id, &server_id, temp_tunnel, reason);
    }
}

fn connect_and_open_sftp(
    endpoint: &SshEndpoint,
    known_hosts_path: Option<&Path>,
    identity_file: Option<&Path>,
    password: Option<&str>,
) -> Result<(SshSession, Sftp), CoreError> {
    let kh = known_hosts_path
        .ok_or_else(|| host_key_untrusted("No SSH known_hosts path available".into()))?;
    if !kh.is_file() {
        return Err(host_key_untrusted(format!(
            "SSH known_hosts file missing: {}",
            kh.display()
        )));
    }
    let tcp = connect_tcp_timeout(&endpoint.host, endpoint.port, Duration::from_secs(8))?;
    let mut session = SshSession::new().map_err(|e| {
        CoreError::operation_failed("ssh_init_failed", e.to_string(), false, "retry_later")
    })?;
    session.set_tcp_stream(tcp);
    session.handshake().map_err(|e| {
        CoreError::operation_failed(
            "ssh_handshake_failed",
            e.to_string(),
            true,
            "inspect_network",
        )
    })?;
    verify_session_host_key(&session, &endpoint.host, endpoint.port, kh)?;
    authenticate_sftp_session(&session, &endpoint.user, identity_file, password)?;
    let sftp = session.sftp().map_err(|e| {
        CoreError::operation_failed(
            "sftp_open_failed",
            format!("SFTP subsystem failed to start: {e}"),
            false,
            "inspect_server_config",
        )
    })?;
    Ok((session, sftp))
}

fn authenticate_sftp_session(
    session: &SshSession,
    user: &str,
    identity_file: Option<&Path>,
    password: Option<&str>,
) -> Result<(), CoreError> {
    if let Some(key) = identity_file {
        if session.userauth_pubkey_file(user, None, key, None).is_ok() && session.authenticated() {
            return Ok(());
        }
    }
    if session.userauth_agent(user).is_ok() && session.authenticated() {
        return Ok(());
    }
    if identity_file.is_none() {
        for key_path in common_ssh_private_keys() {
            if session
                .userauth_pubkey_file(user, None, &key_path, None)
                .is_ok()
                && session.authenticated()
            {
                return Ok(());
            }
        }
    }
    if let Some(pw) = password.filter(|s| !s.is_empty()) {
        if session.userauth_password(user, pw).is_ok() && session.authenticated() {
            return Ok(());
        }
    }
    Err(CoreError::operation_failed(
        "sftp_auth_failed",
        format!("SSH authentication failed for user {user}"),
        false,
        "configure_ssh_credentials",
    ))
}

/// Discovers standard private key files under `$HOME/.ssh` (`USERPROFILE` fallback on Windows).
pub fn common_ssh_private_keys() -> Vec<PathBuf> {
    let Some(home) = home_dir_from_env() else {
        return Vec::new();
    };
    let ssh_dir = home.join(".ssh");
    [
        "id_ed25519",
        "id_ecdsa",
        "id_ecdsa_sk",
        "id_rsa",
        "id_dsa",
        "id_xmss",
    ]
    .into_iter()
    .map(|name| ssh_dir.join(name))
    .filter(|p| p.is_file())
    .collect()
}

pub(crate) fn list_remote_children_impl(
    store: &RemoteBrowserStore,
    browser_id: &str,
    path: &str,
) -> Result<Vec<RemoteBrowseEntry>, CoreError> {
    let norm = normalize_remote_transfer_path(path, Some("/"), "invalid_remote_path", true)
        .map_err(|e| e.into_core_error())?;
    let entry_lock = store.get_session(browser_id)?;
    let guard = entry_lock.lock();
    let entries = guard
        .sftp
        .readdir(Path::new(&norm))
        .map_err(|e| map_sftp_error(&format!("Failed to list {norm}"), e))?;
    let mut result = Vec::new();
    for (item_path, stat) in entries {
        let Some(name_os) = item_path.file_name() else {
            continue;
        };
        let name = name_os.to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let child_path = if norm == "/" {
            format!("/{name}")
        } else {
            format!("{norm}/{name}")
        };
        result.push(RemoteBrowseEntry {
            name,
            path: child_path,
            kind: remote_kind_from_file_stat(&stat),
        });
    }
    result.sort_by(|a, b| {
        let a_dir = a.kind == RemotePathKind::Directory;
        let b_dir = b.kind == RemotePathKind::Directory;
        b_dir.cmp(&a_dir).then_with(|| a.name.cmp(&b.name))
    });
    Ok(result)
}

pub(crate) fn stat_remote_path_impl(
    store: &RemoteBrowserStore,
    browser_id: &str,
    path: &str,
) -> Result<RemoteBrowseNodeInfo, CoreError> {
    let norm = normalize_remote_transfer_path(path, Some("/"), "invalid_remote_path", true)
        .map_err(|e| e.into_core_error())?;
    let entry_lock = store.get_session(browser_id)?;
    let guard = entry_lock.lock();
    match guard.sftp.lstat(Path::new(&norm)) {
        Ok(stat) => Ok(RemoteBrowseNodeInfo {
            path: norm,
            kind: remote_kind_from_file_stat(&stat),
            exists: true,
        }),
        Err(err) if is_sftp_not_found(&err) => Ok(RemoteBrowseNodeInfo {
            path: norm,
            kind: RemotePathKind::Missing,
            exists: false,
        }),
        Err(err) => Err(map_sftp_error(&format!("Failed to stat {norm}"), err)),
    }
}

pub(crate) fn create_remote_directory_impl(
    store: &RemoteBrowserStore,
    browser_id: &str,
    path: &str,
) -> Result<RemoteBrowseNodeInfo, CoreError> {
    let norm = normalize_remote_transfer_path(path, None, "invalid_remote_path", false)
        .map_err(|e| e.into_core_error())?;
    let entry_lock = store.get_session(browser_id)?;
    let guard = entry_lock.lock();
    if let Err(err) = guard.sftp.mkdir(Path::new(&norm), 0o755) {
        return Err(map_sftp_error(
            &format!("Failed to create directory {norm}"),
            err,
        ));
    }
    let stat = guard
        .sftp
        .lstat(Path::new(&norm))
        .map_err(|e| map_sftp_error(&format!("Created directory unstatable: {norm}"), e))?;
    Ok(RemoteBrowseNodeInfo {
        path: norm,
        kind: remote_kind_from_file_stat(&stat),
        exists: true,
    })
}

pub(crate) fn delete_remote_path_impl(
    store: &RemoteBrowserStore,
    browser_id: &str,
    path: &str,
    recursive: bool,
) -> Result<(), CoreError> {
    let norm = normalize_remote_transfer_path(path, None, "invalid_remote_path", false)
        .map_err(|e| e.into_core_error())?;
    let entry_lock = store.get_session(browser_id)?;
    let guard = entry_lock.lock();
    let stat = guard
        .sftp
        .lstat(Path::new(&norm))
        .map_err(|e| map_sftp_error(&format!("Path not found: {norm}"), e))?;
    remove_remote_recursive(&guard.sftp, Path::new(&norm), &stat, recursive)
}

fn remove_remote_recursive(
    sftp: &Sftp,
    path: &Path,
    stat: &FileStat,
    recursive: bool,
) -> Result<(), CoreError> {
    if stat.file_type() != FileType::Directory {
        return sftp
            .unlink(path)
            .map_err(|e| map_sftp_error(&format!("Failed to delete {}", path.display()), e));
    }
    if recursive {
        let entries = sftp
            .readdir(path)
            .map_err(|e| map_sftp_error(&format!("Failed to list {}", path.display()), e))?;
        for (child, child_stat) in entries {
            let name = child
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name == "." || name == ".." {
                continue;
            }
            remove_remote_recursive(sftp, &child, &child_stat, true)?;
        }
    }
    sftp.rmdir(path)
        .map_err(|e| map_sftp_error(&format!("Failed to remove dir {}", path.display()), e))
}

fn is_sftp_not_found(err: &ssh2::Error) -> bool {
    matches!(err.code(), Ssh2ErrorCode::SFTP(2))
}

fn map_sftp_error(prefix: &str, err: ssh2::Error) -> CoreError {
    if is_sftp_not_found(&err) {
        return CoreError::not_found(
            "remote_path_not_found",
            format!("{prefix}: {err}"),
            "inspect_remote_path",
        );
    }
    CoreError::operation_failed(
        "sftp_operation_failed",
        format!("{prefix}: {err}"),
        false,
        "inspect_remote_path",
    )
}
