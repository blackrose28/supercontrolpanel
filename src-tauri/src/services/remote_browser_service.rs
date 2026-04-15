#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::{
    env,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::{ChildStderr, Command, Stdio},
    sync::{Arc, Mutex},
};

use ssh2::{FileStat, Session as SshSession};
use tauri::AppHandle;

use crate::{
    models::{
        RemoteBrowseEntry, RemoteBrowseNodeInfo, RemoteBrowseSession, RemotePathKind, SshEndpoint,
    },
    server_mgmt::ConnectionMethod,
    services::{
        add_common_ssh_options, ensure_boundary_tunnel_running, load_server_by_id, lock_err,
        normalize_remote_transfer_path, resolve_server_ssh_endpoint, ssh_target,
    },
    stores::{
        CliRemoteBrowserIo, CliRemoteBrowserSession, FunctionStore, RemoteBrowserSessionState,
        RemoteBrowserStore, RemoteBrowserTransport,
    },
};

pub fn shutdown_remote_browser_session_state(state: &RemoteBrowserSessionState) {
    match &state.transport {
        RemoteBrowserTransport::Sftp(_) => {}
        RemoteBrowserTransport::Cli(session) => {
            if let Ok(mut io) = lock_err(&session.io) {
                let _ = io.stdin.flush();
                let _ = io.child.kill();
                let _ = io.child.wait();
            }
        }
    }
}

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

pub fn normalize_remote_browser_path(path: &str, fallback: &str) -> String {
    normalize_remote_transfer_path(path, Some(fallback), "invalid_target_path", true)
        .unwrap_or_else(|_| fallback.to_string())
}

pub fn parse_remote_browser_ls_entry(base_path: &str, raw: &str) -> Option<RemoteBrowseEntry> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return None;
    }
    let (name, kind) = if let Some(name) = trimmed.strip_suffix('/') {
        (name.to_string(), RemotePathKind::Directory)
    } else {
        (trimmed.to_string(), RemotePathKind::File)
    };
    Some(RemoteBrowseEntry {
        path: join_remote_path(base_path, &name),
        name,
        kind,
    })
}

pub fn join_remote_path(base: &str, name: &str) -> String {
    let clean_name = name.trim_start_matches('/');
    if base == "/" {
        format!("/{clean_name}")
    } else {
        format!("{}/{}", base.trim_end_matches('/'), clean_name)
    }
}

fn shell_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "'\"'\"'"))
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

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn spawn_cli_remote_browser_session(
    endpoint: &SshEndpoint,
    #[cfg(target_os = "windows")] create_no_window: u32,
) -> Result<CliRemoteBrowserSession, String> {
    let mut cmd = Command::new("ssh");
    add_common_ssh_options(&mut cmd, "-p", endpoint.port);
    cmd.arg("-T")
        .arg(ssh_target(endpoint))
        .arg("sh")
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(create_no_window);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to start persistent remote browser shell: {e}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open remote browser shell stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to open remote browser shell stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "failed to open remote browser shell stderr".to_string())?;
    Ok(CliRemoteBrowserSession {
        io: Mutex::new(CliRemoteBrowserIo {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            stderr,
            next_marker_id: 1,
        }),
    })
}

fn read_cli_browser_stderr(stderr: &mut ChildStderr) -> String {
    let mut buf = String::new();
    let _ = stderr.read_to_string(&mut buf);
    buf.trim().to_string()
}

fn run_cli_remote_browser_command(
    session: &CliRemoteBrowserSession,
    script_body: &str,
) -> Result<String, String> {
    let mut io = lock_err(&session.io)?;
    if let Some(status) = io
        .child
        .try_wait()
        .map_err(|e| format!("failed to query remote browser shell status: {e}"))?
    {
        let stderr = read_cli_browser_stderr(&mut io.stderr);
        let base = format!("remote browser shell exited unexpectedly: {status}");
        return Err(if stderr.is_empty() {
            base
        } else {
            format!("{base}; {stderr}")
        });
    }
    let marker = format!("__SCP_REMOTE_BROWSER_DONE_{}__", io.next_marker_id);
    io.next_marker_id += 1;
    let script = format!(
        "exec 2>&1\n{{\n{script_body}\n}}\n__scp_status=$?\nprintf '\\n{marker} %s\\n' \"$__scp_status\"\n"
    );
    io.stdin
        .write_all(script.as_bytes())
        .and_then(|_| io.stdin.flush())
        .map_err(|e| format!("failed to send remote browser command: {e}"))?;

    let mut output = String::new();
    loop {
        let mut line = String::new();
        let read = io
            .stdout
            .read_line(&mut line)
            .map_err(|e| format!("failed to read remote browser output: {e}"))?;
        if read == 0 {
            let stderr = read_cli_browser_stderr(&mut io.stderr);
            return Err(if stderr.is_empty() {
                "remote browser shell closed before completing the command".to_string()
            } else {
                format!("remote browser shell closed before completing the command: {stderr}")
            });
        }
        if let Some(rest) = line.strip_prefix(&marker) {
            let status = rest.trim().parse::<i32>().unwrap_or(1);
            if status == 0 {
                return Ok(output.trim().to_string());
            }
            let stderr = read_cli_browser_stderr(&mut io.stderr);
            let body = output.trim();
            let detail = if !stderr.is_empty() {
                stderr
            } else if !body.is_empty() {
                body.to_string()
            } else {
                format!("remote browser command failed with exit code {status}")
            };
            return Err(detail);
        }
        output.push_str(&line);
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn remote_browser_root_path_via_cli_session(
    session: &CliRemoteBrowserSession,
) -> Result<String, String> {
    let output = run_cli_remote_browser_command(session, "cd && pwd -P")?;
    output
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "remote browser root path was empty".to_string())
}

pub fn list_remote_browser_children_via_cli_session(
    session: &CliRemoteBrowserSession,
    path: &str,
) -> Result<Vec<RemoteBrowseEntry>, String> {
    const SPLIT_MARKER: &str = "__SCP_REMOTE_BROWSER_LISTING__";
    let script = format!(
        "cd {} && pwd -P && printf '\\n{SPLIT_MARKER}\\n' && LC_ALL=C ls -A1p",
        shell_quote(path)
    );
    let output = run_cli_remote_browser_command(session, &script)?;
    let (resolved_path, raw_entries) = output
        .split_once(&format!("\n{SPLIT_MARKER}\n"))
        .ok_or_else(|| "remote browser listing output was not recognized".to_string())?;
    let resolved_path = normalize_remote_browser_path(resolved_path.trim(), path);
    Ok(raw_entries
        .lines()
        .filter_map(|line| parse_remote_browser_ls_entry(&resolved_path, line))
        .collect())
}

pub fn stat_remote_browser_path_via_cli_session(
    session: &CliRemoteBrowserSession,
    path: &str,
) -> Result<RemoteBrowseNodeInfo, String> {
    let quoted = shell_quote(path);
    let script = format!(
        "p={quoted}; if [ -d \"$p\" ]; then printf 'directory'; \\
         elif [ -f \"$p\" ]; then printf 'file'; \\
         elif [ -L \"$p\" ]; then printf 'symlink'; \\
         elif [ -e \"$p\" ]; then printf 'other'; \\
         else printf 'missing'; fi"
    );
    let raw_kind = run_cli_remote_browser_command(session, &script)?;
    let kind = match raw_kind.trim() {
        "directory" => RemotePathKind::Directory,
        "file" => RemotePathKind::File,
        "symlink" => RemotePathKind::Symlink,
        "other" => RemotePathKind::Other,
        _ => RemotePathKind::Missing,
    };
    Ok(RemoteBrowseNodeInfo {
        path: path.to_string(),
        exists: !matches!(kind, RemotePathKind::Missing),
        kind,
    })
}

pub fn create_remote_browser_directory_via_cli_session(
    session: &CliRemoteBrowserSession,
    path: &str,
) -> Result<(), String> {
    let script = format!("mkdir -p -- {}", shell_quote(path));
    run_cli_remote_browser_command(session, &script).map(|_| ())
}

pub fn delete_remote_browser_path_via_cli_session(
    session: &CliRemoteBrowserSession,
    path: &str,
    recursive: bool,
) -> Result<(), String> {
    let script = if recursive {
        format!("rm -rf -- {}", shell_quote(path))
    } else {
        format!(
            "if [ -d {p} ]; then rmdir -- {p}; else rm -f -- {p}; fi",
            p = shell_quote(path)
        )
    };
    run_cli_remote_browser_command(session, &script).map(|_| ())
}

pub fn remove_remote_sftp_path_recursive(
    sftp: &ssh2::Sftp,
    path: &std::path::Path,
) -> Result<(), String> {
    let stat = sftp
        .lstat(path)
        .map_err(|e| format!("failed to inspect remote path: {e}"))?;
    if matches!(remote_kind_from_file_stat(&stat), RemotePathKind::Directory) {
        for (child_path, _) in sftp
            .readdir(path)
            .map_err(|e| format!("failed to list remote directory: {e}"))?
        {
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
            .map_err(|e| format!("failed to remove remote directory: {e}"))?;
    } else {
        sftp.unlink(path)
            .map_err(|e| format!("failed to remove remote file: {e}"))?;
    }
    Ok(())
}

pub fn open_remote_sftp_session(
    endpoint: &SshEndpoint,
    via_boundary_tunnel: bool,
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
    session.set_timeout(15_000);
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|e| format!("ssh handshake failed: {e}"))?;
    let mut auth_attempts = Vec::new();
    match session.userauth_agent(&endpoint.user) {
        Ok(_) => {}
        Err(agent_err) => {
            auth_attempts.push(format!("ssh agent auth failed: {agent_err}"));
            let mut authed = false;
            for private_key in common_ssh_private_keys() {
                match session.userauth_pubkey_file(&endpoint.user, None, &private_key, None) {
                    Ok(_) => {
                        authed = true;
                        break;
                    }
                    Err(key_err) => auth_attempts.push(format!(
                        "key auth failed for {}: {key_err}",
                        private_key.display()
                    )),
                }
            }
            if !authed && !session.authenticated() {
                if auth_attempts.len() == 1 {
                    auth_attempts.push(
                        "no usable private key found in ~/.ssh for fallback auth".to_string(),
                    );
                }
                return Err(auth_attempts.join("; "));
            }
        }
    }
    if !session.authenticated() {
        return Err("ssh authentication did not succeed".to_string());
    }
    Ok(session)
}

pub fn open_remote_browser(
    app: &AppHandle,
    server_id: String,
    browser_store: &RemoteBrowserStore,
    function_store: &FunctionStore,
    server_tunnel_function_id: fn(&str) -> String,
    #[cfg(target_os = "windows")] create_no_window: u32,
) -> Result<RemoteBrowseSession, String> {
    let browser_id = next_remote_browser_id(browser_store)?;
    open_remote_browser_with_id(
        app,
        server_id,
        browser_id,
        browser_store,
        function_store,
        server_tunnel_function_id,
        #[cfg(target_os = "windows")]
        create_no_window,
    )
}

pub fn open_remote_browser_with_id(
    app: &AppHandle,
    server_id: String,
    browser_id: String,
    browser_store: &RemoteBrowserStore,
    function_store: &FunctionStore,
    server_tunnel_function_id: fn(&str) -> String,
    #[cfg(target_os = "windows")] create_no_window: u32,
) -> Result<RemoteBrowseSession, String> {
    close_all_remote_browser_sessions(browser_store)?;
    let server = load_server_by_id(app, &server_id)?;
    ensure_boundary_tunnel_running(function_store, &server, server_tunnel_function_id)?;
    let endpoint = resolve_server_ssh_endpoint(&server, "remote browser")?;
    #[cfg(target_os = "windows")]
    let (root_path, transport) = {
        let session = spawn_cli_remote_browser_session(&endpoint, create_no_window)?;
        let root_path = remote_browser_root_path_via_cli_session(&session)?;
        (root_path, RemoteBrowserTransport::Cli(session))
    };
    #[cfg(not(target_os = "windows"))]
    let (root_path, transport) = {
        let session = open_remote_sftp_session(
            &endpoint,
            server.connection_method == ConnectionMethod::Boundary,
        )?;
        let sftp = session
            .sftp()
            .map_err(|e| format!("failed to open sftp session: {e}"))?;
        let root_path = sftp
            .realpath(std::path::Path::new("."))
            .or_else(|_| sftp.realpath(std::path::Path::new("/")))
            .map(|path| path_buf_to_remote_string(&path))
            .map_err(|e| format!("failed to resolve remote root path: {e}"))?;
        (root_path, RemoteBrowserTransport::Sftp(session))
    };
    let state = Arc::new(RemoteBrowserSessionState {
        server_id,
        root_path: root_path.clone(),
        transport,
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
    let target_path =
        normalize_remote_transfer_path(&path, Some(&browser.root_path), "invalid_target_path", true)
            .map_err(|err| err.into_string())?;
    match &browser.transport {
        RemoteBrowserTransport::Sftp(session) => {
            let sftp = session
                .sftp()
                .map_err(|e| format!("failed to open sftp session: {e}"))?;
            let mut entries = Vec::new();
            for (child_path, stat) in sftp
                .readdir(std::path::Path::new(&target_path))
                .map_err(|e| format!("failed to list remote path: {e}"))?
            {
                let name = child_path
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_default();
                if name.is_empty() || name == "." || name == ".." {
                    continue;
                }
                entries.push(RemoteBrowseEntry {
                    name,
                    path: path_buf_to_remote_string(&child_path),
                    kind: remote_kind_from_file_stat(&stat),
                });
            }
            Ok(entries)
        }
        RemoteBrowserTransport::Cli(endpoint) => {
            list_remote_browser_children_via_cli_session(endpoint, &target_path)
        }
    }
}

pub fn stat_remote_browser_path(
    browser_id: String,
    path: String,
    browser_store: &RemoteBrowserStore,
) -> Result<RemoteBrowseNodeInfo, String> {
    let browser = remote_browser_state(&browser_id, browser_store)?;
    let target_path =
        normalize_remote_transfer_path(&path, Some(&browser.root_path), "invalid_target_path", true)
            .map_err(|err| err.into_string())?;
    match &browser.transport {
        RemoteBrowserTransport::Sftp(session) => {
            let sftp = session
                .sftp()
                .map_err(|e| format!("failed to open sftp session: {e}"))?;
            match sftp.lstat(std::path::Path::new(&target_path)) {
                Ok(stat) => Ok(RemoteBrowseNodeInfo {
                    path: target_path,
                    kind: remote_kind_from_file_stat(&stat),
                    exists: true,
                }),
                Err(_) => Ok(RemoteBrowseNodeInfo {
                    path: target_path,
                    kind: RemotePathKind::Missing,
                    exists: false,
                }),
            }
        }
        RemoteBrowserTransport::Cli(endpoint) => {
            stat_remote_browser_path_via_cli_session(endpoint, &target_path)
        }
    }
}

pub fn create_remote_browser_directory(
    browser_id: String,
    path: String,
    browser_store: &RemoteBrowserStore,
) -> Result<(), String> {
    let browser = remote_browser_state(&browser_id, browser_store)?;
    let target_path =
        normalize_remote_transfer_path(&path, Some(&browser.root_path), "invalid_target_path", false)
            .map_err(|err| err.into_string())?;
    match &browser.transport {
        RemoteBrowserTransport::Sftp(session) => {
            let sftp = session
                .sftp()
                .map_err(|e| format!("failed to open sftp session: {e}"))?;
            sftp.mkdir(std::path::Path::new(&target_path), 0o755)
                .map_err(|e| format!("failed to create remote directory: {e}"))?;
            Ok(())
        }
        RemoteBrowserTransport::Cli(session) => {
            create_remote_browser_directory_via_cli_session(session, &target_path)
        }
    }
}

pub fn delete_remote_browser_path(
    browser_id: String,
    path: String,
    recursive: Option<bool>,
    browser_store: &RemoteBrowserStore,
) -> Result<(), String> {
    let browser = remote_browser_state(&browser_id, browser_store)?;
    let target_path =
        normalize_remote_transfer_path(&path, Some(&browser.root_path), "invalid_target_path", false)
            .map_err(|err| err.into_string())?;
    match &browser.transport {
        RemoteBrowserTransport::Sftp(session) => {
            let sftp = session
                .sftp()
                .map_err(|e| format!("failed to open sftp session: {e}"))?;
            if recursive.unwrap_or(false) {
                remove_remote_sftp_path_recursive(&sftp, std::path::Path::new(&target_path))
            } else {
                let stat = sftp
                    .lstat(std::path::Path::new(&target_path))
                    .map_err(|e| format!("failed to inspect remote path: {e}"))?;
                if matches!(remote_kind_from_file_stat(&stat), RemotePathKind::Directory) {
                    sftp.rmdir(std::path::Path::new(&target_path))
                        .map_err(|e| format!("failed to remove remote directory: {e}"))?;
                } else {
                    sftp.unlink(std::path::Path::new(&target_path))
                        .map_err(|e| format!("failed to remove remote file: {e}"))?;
                }
                Ok(())
            }
        }
        RemoteBrowserTransport::Cli(session) => delete_remote_browser_path_via_cli_session(
            session,
            &target_path,
            recursive.unwrap_or(false),
        ),
    }
}
