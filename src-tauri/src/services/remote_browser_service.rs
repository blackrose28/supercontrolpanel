use std::{env, net::TcpStream, path::PathBuf, sync::Arc};

use ssh2::{FileStat, KeyboardInteractivePrompt, Prompt, Session as SshSession};
use tauri::AppHandle;

use crate::{
    models::{
        RemoteBrowseEntry, RemoteBrowseNodeInfo, RemoteBrowseSession, RemotePathKind, SshEndpoint,
    },
    server_mgmt::ConnectionMethod,
    services::{
        ensure_boundary_tunnel_running, keyring_get_optional, keyring_server_ssh_password_account,
        load_server_by_id, lock_err, normalize_remote_transfer_path, resolve_server_ssh_endpoint,
    },
    stores::{
        FunctionStore, RemoteBrowserSessionState, RemoteBrowserStore,
    },
};

pub fn shutdown_remote_browser_session_state(_state: &RemoteBrowserSessionState) {}

pub fn close_remote_browser_session_internal(
    browser_id: &str,
    browser_store: &RemoteBrowserStore,
) -> Result<(), String> {
    let removed = {
        let mut sessions = lock_err(&browser_store.sessions)?;
        sessions.remove(browser_id)
    };
    if let Some(state) = removed {
        shutdown_remote_browser_session_state(&state);
    }
    Ok(())
}

pub fn close_remote_browser_sessions_for_server(
    server_id: &str,
    browser_store: &RemoteBrowserStore,
) -> Result<usize, String> {
    let removed = {
        let mut sessions = lock_err(&browser_store.sessions)?;
        let ids = sessions
            .iter()
            .filter_map(|(browser_id, session)| {
                if session.server_id == server_id {
                    Some(browser_id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let mut removed = Vec::with_capacity(ids.len());
        for browser_id in ids {
            if let Some(session) = sessions.remove(&browser_id) {
                removed.push(session);
            }
        }
        removed
    };
    let count = removed.len();
    for state in removed {
        shutdown_remote_browser_session_state(&state);
    }
    Ok(count)
}

pub fn close_all_remote_browser_sessions(browser_store: &RemoteBrowserStore) -> Result<(), String> {
    let removed = {
        let mut sessions = lock_err(&browser_store.sessions)?;
        sessions
            .drain()
            .map(|(_, session)| session)
            .collect::<Vec<_>>()
    };
    for state in removed {
        shutdown_remote_browser_session_state(&state);
    }
    Ok(())
}

pub fn next_remote_browser_id(store: &RemoteBrowserStore) -> Result<String, String> {
    let mut next = lock_err(&store.next_id)?;
    let id = format!("browser-{}", *next);
    *next += 1;
    Ok(id)
}

pub fn remote_browser_state(
    browser_id: &str,
    store: &RemoteBrowserStore,
) -> Result<Arc<RemoteBrowserSessionState>, String> {
    lock_err(&store.sessions)?
        .get(browser_id)
        .cloned()
        .ok_or_else(|| "remote browser session not found".to_string())
}

pub fn remote_kind_from_file_stat(stat: &FileStat) -> RemotePathKind {
    const S_IFMT: u32 = 0o170000;
    const S_IFDIR: u32 = 0o040000;
    const S_IFREG: u32 = 0o100000;
    const S_IFLNK: u32 = 0o120000;

    match stat.perm.map(|perm| perm & S_IFMT) {
        Some(S_IFDIR) => RemotePathKind::Directory,
        Some(S_IFREG) => RemotePathKind::File,
        Some(S_IFLNK) => RemotePathKind::Symlink,
        Some(_) => RemotePathKind::Other,
        None => RemotePathKind::Other,
    }
}

pub fn path_buf_to_remote_string(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(PathBuf::from))
    }

    #[cfg(not(target_os = "windows"))]
    {
        env::var_os("HOME").map(PathBuf::from)
    }
}

fn common_ssh_private_keys() -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
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
    .filter(|path| path.is_file())
    .collect()
}

struct SshPasswordKeyboardPrompter<'a> {
    password: &'a str,
}

impl KeyboardInteractivePrompt for SshPasswordKeyboardPrompter<'_> {
    fn prompt(
        &mut self,
        _username: &str,
        _instructions: &str,
        prompts: &[Prompt<'_>],
    ) -> Vec<String> {
        prompts
            .iter()
            .map(|p| {
                let text = p.text.to_lowercase();
                if !p.echo || text.contains("password") || text.contains("passphrase") {
                    self.password.to_string()
                } else {
                    String::new()
                }
            })
            .collect()
    }
}

fn format_sftp_error(context: &str, path: &str, err: impl std::fmt::Display) -> String {
    let es = err.to_string();
    let lower = es.to_lowercase();
    if lower.contains("permission denied") || lower.contains("access denied") {
        format!("{context} {path}: {es}")
    } else {
        format!("{context}: {es}")
    }
}

/// Opens an SSH session suitable for SFTP (non-interactive). Uses ssh-agent and default
/// `~/.ssh` private keys, then optional `ssh_password` (e.g. from OS keyring).
pub fn open_remote_sftp_session(
    endpoint: &SshEndpoint,
    via_boundary_tunnel: bool,
    ssh_password: Option<&str>,
) -> Result<SshSession, String> {
    let tcp = TcpStream::connect((endpoint.host.as_str(), endpoint.port)).map_err(|e| {
        if via_boundary_tunnel {
            format!(
                "failed to connect to Boundary tunnel at {}:{}: {e}",
                endpoint.host, endpoint.port
            )
        } else {
            format!(
                "failed to connect to {}:{}: {e}",
                endpoint.host, endpoint.port
            )
        }
    })?;
    let mut session = SshSession::new().map_err(|e| format!("ssh session init failed: {e}"))?;
    session.set_timeout(30_000);
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|e| format!("ssh handshake failed: {e}"))?;

    let mut notes: Vec<String> = Vec::new();

    if let Err(e) = session.userauth_agent(&endpoint.user) {
        notes.push(format!("ssh agent: {e}"));
    }
    if session.authenticated() {
        return Ok(session);
    }

    for private_key in common_ssh_private_keys() {
        match session.userauth_pubkey_file(&endpoint.user, None, &private_key, None) {
            Ok(()) => {
                if session.authenticated() {
                    return Ok(session);
                }
            }
            Err(key_err) => notes.push(format!("key {}: {key_err}", private_key.display())),
        }
    }

    if let Some(pw) = ssh_password.filter(|p| !p.trim().is_empty()) {
        if session.userauth_password(&endpoint.user, pw).is_ok() && session.authenticated() {
            return Ok(session);
        }
        notes.push("password authentication did not succeed".to_string());

        let mut prompter = SshPasswordKeyboardPrompter { password: pw };
        if session
            .userauth_keyboard_interactive(&endpoint.user, &mut prompter)
            .is_ok()
            && session.authenticated()
        {
            return Ok(session);
        }
        notes.push("keyboard-interactive authentication did not succeed".to_string());
    }

    if session.authenticated() {
        return Ok(session);
    }

    Err(format!(
        "ssh authentication did not succeed. {}. For password-only servers, open the server editor, set “SSH password (optional)”, and save — it is stored in the OS keyring. You can also use ssh-agent or place a private key under ~/.ssh.",
        notes.join("; ")
    ))
}

pub fn remove_remote_sftp_path_recursive(
    sftp: &ssh2::Sftp,
    path: &std::path::Path,
) -> Result<(), String> {
    let stat = sftp
        .lstat(path)
        .map_err(|e| format_sftp_error("failed to inspect remote path", &path.display().to_string(), e))?;
    if matches!(remote_kind_from_file_stat(&stat), RemotePathKind::Directory) {
        for (child_path, _) in sftp.readdir(path).map_err(|e| {
            format_sftp_error("failed to list remote directory", &path.display().to_string(), e)
        })? {
            let name = child_path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_default();
            if name.is_empty() || name == "." || name == ".." {
                continue;
            }
            remove_remote_sftp_path_recursive(sftp, &child_path)?;
        }
        sftp.rmdir(path)
            .map_err(|e| format_sftp_error("failed to remove remote directory", &path.display().to_string(), e))?;
    } else {
        sftp.unlink(path)
            .map_err(|e| format_sftp_error("failed to remove remote file", &path.display().to_string(), e))?;
    }
    Ok(())
}

pub fn open_remote_browser(
    app: &AppHandle,
    server_id: String,
    browser_store: &RemoteBrowserStore,
    function_store: &FunctionStore,
    server_tunnel_function_id: fn(&str) -> String,
) -> Result<RemoteBrowseSession, String> {
    let browser_id = next_remote_browser_id(browser_store)?;
    open_remote_browser_with_id(
        app,
        server_id,
        browser_id,
        browser_store,
        function_store,
        server_tunnel_function_id,
    )
}

pub fn open_remote_browser_with_id(
    app: &AppHandle,
    server_id: String,
    browser_id: String,
    browser_store: &RemoteBrowserStore,
    function_store: &FunctionStore,
    server_tunnel_function_id: fn(&str) -> String,
) -> Result<RemoteBrowseSession, String> {
    close_all_remote_browser_sessions(browser_store)?;
    let server = load_server_by_id(app, &server_id)?;
    ensure_boundary_tunnel_running(function_store, &server, server_tunnel_function_id)?;
    let endpoint = resolve_server_ssh_endpoint(&server, "remote browser")?;
    let ssh_password = keyring_get_optional(&keyring_server_ssh_password_account(&server_id))?;
    let session = open_remote_sftp_session(
        &endpoint,
        server.connection_method == ConnectionMethod::Boundary,
        ssh_password.as_deref(),
    )?;
    let sftp = session
        .sftp()
        .map_err(|e| format!("failed to open sftp session: {e}"))?;
    let root_path = sftp
        .realpath(std::path::Path::new("."))
        .or_else(|_| sftp.realpath(std::path::Path::new("/")))
        .map(|path| path_buf_to_remote_string(&path))
        .map_err(|e| format!("failed to resolve remote root path: {e}"))?;
    drop(sftp);
    let state = Arc::new(RemoteBrowserSessionState {
        server_id,
        root_path: root_path.clone(),
        ssh: session,
    });
    lock_err(&browser_store.sessions)?.insert(browser_id.clone(), state);
    Ok(RemoteBrowseSession {
        browser_id,
        root_path,
    })
}

pub fn close_remote_browser(
    browser_id: String,
    browser_store: &RemoteBrowserStore,
) -> Result<(), String> {
    close_remote_browser_session_internal(&browser_id, browser_store)
}

pub fn list_remote_browser_children(
    browser_id: String,
    path: String,
    browser_store: &RemoteBrowserStore,
) -> Result<Vec<RemoteBrowseEntry>, String> {
    let browser = remote_browser_state(&browser_id, browser_store)?;
    let target_path = normalize_remote_transfer_path(
        &path,
        Some(&browser.root_path),
        "invalid_target_path",
        true,
    )
    .map_err(|err| err.into_string())?;
    let sftp = browser
        .ssh
        .sftp()
        .map_err(|e| format!("failed to open sftp session: {e}"))?;
    let mut entries = Vec::new();
    for (child_path, stat) in sftp.readdir(std::path::Path::new(&target_path)).map_err(|e| {
        format_sftp_error(
            "permission denied or failed listing remote folder",
            &target_path,
            e,
        )
    })? {
        let name = child_path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        let child_remote = path_buf_to_remote_string(&child_path);
        if child_remote == target_path {
            continue;
        }
        entries.push(RemoteBrowseEntry {
            name,
            path: child_remote,
            kind: remote_kind_from_file_stat(&stat),
        });
    }
    Ok(entries)
}

pub fn stat_remote_browser_path(
    browser_id: String,
    path: String,
    browser_store: &RemoteBrowserStore,
) -> Result<RemoteBrowseNodeInfo, String> {
    let browser = remote_browser_state(&browser_id, browser_store)?;
    let target_path = normalize_remote_transfer_path(
        &path,
        Some(&browser.root_path),
        "invalid_target_path",
        true,
    )
    .map_err(|err| err.into_string())?;
    let sftp = browser
        .ssh
        .sftp()
        .map_err(|e| format!("failed to open sftp session: {e}"))?;
    match sftp.lstat(std::path::Path::new(&target_path)) {
        Ok(stat) => Ok(RemoteBrowseNodeInfo {
            path: target_path,
            kind: remote_kind_from_file_stat(&stat),
            exists: true,
        }),
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            if msg.contains("permission denied") || msg.contains("access denied") {
                return Err(format_sftp_error(
                    "permission denied inspecting remote path",
                    &target_path,
                    e,
                ));
            }
            Ok(RemoteBrowseNodeInfo {
                path: target_path,
                kind: RemotePathKind::Missing,
                exists: false,
            })
        }
    }
}

pub fn create_remote_browser_directory(
    browser_id: String,
    path: String,
    browser_store: &RemoteBrowserStore,
) -> Result<(), String> {
    let browser = remote_browser_state(&browser_id, browser_store)?;
    let target_path = normalize_remote_transfer_path(
        &path,
        Some(&browser.root_path),
        "invalid_target_path",
        false,
    )
    .map_err(|err| err.into_string())?;
    let sftp = browser
        .ssh
        .sftp()
        .map_err(|e| format!("failed to open sftp session: {e}"))?;
    sftp.mkdir(std::path::Path::new(&target_path), 0o755)
        .map_err(|e| format_sftp_error("failed to create remote directory", &target_path, e))?;
    Ok(())
}

pub fn delete_remote_browser_path(
    browser_id: String,
    path: String,
    recursive: Option<bool>,
    browser_store: &RemoteBrowserStore,
) -> Result<(), String> {
    let browser = remote_browser_state(&browser_id, browser_store)?;
    let target_path = normalize_remote_transfer_path(
        &path,
        Some(&browser.root_path),
        "invalid_target_path",
        false,
    )
    .map_err(|err| err.into_string())?;
    let sftp = browser
        .ssh
        .sftp()
        .map_err(|e| format!("failed to open sftp session: {e}"))?;
    if recursive.unwrap_or(false) {
        remove_remote_sftp_path_recursive(&sftp, std::path::Path::new(&target_path))
    } else {
        let stat = sftp
            .lstat(std::path::Path::new(&target_path))
            .map_err(|e| format_sftp_error("failed to inspect remote path", &target_path, e))?;
        if matches!(remote_kind_from_file_stat(&stat), RemotePathKind::Directory) {
            sftp.rmdir(std::path::Path::new(&target_path)).map_err(|e| {
                format_sftp_error("failed to remove remote directory", &target_path, e)
            })?;
        } else {
            sftp.unlink(std::path::Path::new(&target_path))
                .map_err(|e| format_sftp_error("failed to remove remote file", &target_path, e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_prompter_fills_password_for_hidden_prompt() {
        let mut p = SshPasswordKeyboardPrompter {
            password: "secret",
        };
        let prompts = vec![Prompt {
            text: std::borrow::Cow::Borrowed("Password: "),
            echo: false,
        }];
        assert_eq!(p.prompt("", "", &prompts), vec!["secret"]);
    }
}
