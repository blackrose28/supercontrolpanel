use std::{
    fs::File,
    io::Read,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use portable_pty::CommandBuilder;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::{
    models::RemotePathKind,
    server_mgmt::ConnectionMethod,
    server_tunnel_function_id,
    services::{
        add_common_ssh_options, build_scp_command, ensure_boundary_tunnel_running,
        keyring_get_optional, keyring_server_ssh_password_account, load_server_by_id,
        normalize_remote_transfer_path, open_remote_sftp_session, remote_kind_from_file_stat,
        resolve_server_ssh_endpoint, ssh_target, validate_local_download_target,
        validate_local_upload_source,
    },
    stores::{FunctionStore, SessionStore},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwritePolicy {
    Prompt,
    Force,
    Skip,
}

#[derive(Debug, Clone)]
pub enum StartScpResult {
    Started {
        session_id: String,
        temp_path_hint: Option<String>,
    },
    Skipped(String),
}

#[derive(Debug, Clone)]
struct ResumeRequest {
    resume_token: String,
    resume_offset_bytes: u64,
    temp_path_hint: String,
}

pub fn parse_overwrite_policy(value: Option<&str>) -> Result<OverwritePolicy, String> {
    match value.unwrap_or("prompt").trim() {
        "" | "prompt" => Ok(OverwritePolicy::Prompt),
        "force" => Ok(OverwritePolicy::Force),
        "skip" => Ok(OverwritePolicy::Skip),
        other => Err(format!(
            "INVALID_PAYLOAD: overwritePolicy must be one of prompt, force, skip (got {other})"
        )),
    }
}

pub fn scp_upload_service(
    app: &AppHandle,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
) -> Result<String, String> {
    let local_path = validate_local_upload_source(&local_path).map_err(|err| err.into_string())?;
    let remote_path =
        normalize_remote_transfer_path(&remote_path, None, "invalid_target_path", false)
            .map_err(|err| err.into_string())?;
    let server = load_server_by_id(app, &server_id)?;
    ensure_boundary_tunnel_running(
        &app.state::<FunctionStore>(),
        &server,
        server_tunnel_function_id,
    )?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let remote_spec = format!("{}:{}", ssh_target(&endpoint), remote_path);
    let mut scp = Command::new("scp");
    add_common_ssh_options(&mut scp, "-P", endpoint.port);
    if recursive.unwrap_or(false) {
        scp.arg("-r");
    }
    let o = scp
        .arg(&local_path)
        .arg(&remote_spec)
        .output()
        .map_err(|e| format!("scp failed to run: {e}"))?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    Ok("Upload finished.".into())
}

pub fn start_scp_upload_service(
    app: &AppHandle,
    session_store: &SessionStore,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    open_pty_with_command_builder: fn(
        u16,
        u16,
        portable_pty::CommandBuilder,
        &AppHandle,
        &SessionStore,
        fn(&AppHandle, &crate::stores::FunctionStore, &str, Option<i32>, &str) -> Result<(), String>,
        Option<&str>,
    ) -> Result<String, String>,
    update_functions_for_session_exit: fn(
        &AppHandle,
        &crate::stores::FunctionStore,
        &str,
        Option<i32>,
        &str,
    ) -> Result<(), String>,
) -> Result<String, String> {
    let local_path = validate_local_upload_source(&local_path).map_err(|err| err.into_string())?;
    let remote_path =
        normalize_remote_transfer_path(&remote_path, None, "invalid_target_path", false)
            .map_err(|err| err.into_string())?;
    let server = load_server_by_id(app, &server_id)?;
    ensure_boundary_tunnel_running(
        &app.state::<FunctionStore>(),
        &server,
        server_tunnel_function_id,
    )?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let cmd = build_scp_command(
        &endpoint,
        &local_path,
        &remote_path,
        recursive.unwrap_or(false),
        false,
    );
    open_pty_with_command_builder(
        120,
        24,
        cmd,
        app,
        session_store,
        update_functions_for_session_exit,
        None,
    )
}

pub fn cc_scp_upload_service(
    app: &AppHandle,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    overwrite_policy: Option<&str>,
    follow_symlinks: bool,
    verify_checksum: bool,
) -> Result<String, String> {
    let local_path = validate_local_upload_source(&local_path).map_err(|err| err.into_string())?;
    let remote_path =
        normalize_remote_transfer_path(&remote_path, None, "invalid_target_path", false)
            .map_err(|err| err.into_string())?;
    let overwrite_policy = parse_overwrite_policy(overwrite_policy)?;
    enforce_symlink_policy_for_upload(&local_path, follow_symlinks)?;
    ensure_checksum_supported(recursive.unwrap_or(false), verify_checksum)?;
    let server = load_server_by_id(app, &server_id)?;
    let remote_path = resolve_remote_upload_target_path(&server, &local_path, &remote_path)?;
    ensure_boundary_tunnel_running(
        &app.state::<FunctionStore>(),
        &server,
        server_tunnel_function_id,
    )?;
    match check_remote_target_overwrite(&server, &remote_path, overwrite_policy)? {
        OverwriteDecision::Proceed => {}
        OverwriteDecision::Skipped(message) => return Ok(message),
    }
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let temp_remote_path = build_atomic_remote_temp_path(&remote_path);
    let remote_spec = format!("{}:{}", ssh_target(&endpoint), temp_remote_path);
    let mut scp = Command::new("scp");
    add_common_ssh_options(&mut scp, "-P", endpoint.port);
    if recursive.unwrap_or(false) {
        scp.arg("-r");
    }
    let o = scp
        .arg(&local_path)
        .arg(&remote_spec)
        .output()
        .map_err(|e| format!("agent_error: scp failed to run: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "agent_error: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    finalize_atomic_remote_upload(&endpoint, &temp_remote_path, &remote_path)?;
    if verify_checksum {
        verify_local_and_remote_sha256(&endpoint, &local_path, &remote_path, "upload")?;
        Ok("Upload finished. SHA-256 verified.".into())
    } else {
        Ok("Upload finished.".into())
    }
}

pub fn scp_download_service(
    app: &AppHandle,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
) -> Result<String, String> {
    let remote_path =
        normalize_remote_transfer_path(&remote_path, None, "invalid_source_path", false)
            .map_err(|err| err.into_string())?;
    let local_path =
        validate_local_download_target(&local_path).map_err(|err| err.into_string())?;
    let server = load_server_by_id(app, &server_id)?;
    ensure_boundary_tunnel_running(
        &app.state::<FunctionStore>(),
        &server,
        server_tunnel_function_id,
    )?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let remote_spec = format!("{}:{}", ssh_target(&endpoint), remote_path);
    let mut scp = Command::new("scp");
    add_common_ssh_options(&mut scp, "-P", endpoint.port);
    if recursive.unwrap_or(false) {
        scp.arg("-r");
    }
    let o = scp
        .arg(&remote_spec)
        .arg(&local_path)
        .output()
        .map_err(|e| format!("scp failed to run: {e}"))?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    Ok("Download finished.".into())
}

pub fn start_scp_download_service(
    app: &AppHandle,
    session_store: &SessionStore,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    open_pty_with_command_builder: fn(
        u16,
        u16,
        portable_pty::CommandBuilder,
        &AppHandle,
        &SessionStore,
        fn(&AppHandle, &crate::stores::FunctionStore, &str, Option<i32>, &str) -> Result<(), String>,
        Option<&str>,
    ) -> Result<String, String>,
    update_functions_for_session_exit: fn(
        &AppHandle,
        &crate::stores::FunctionStore,
        &str,
        Option<i32>,
        &str,
    ) -> Result<(), String>,
) -> Result<String, String> {
    let remote_path =
        normalize_remote_transfer_path(&remote_path, None, "invalid_source_path", false)
            .map_err(|err| err.into_string())?;
    let local_path =
        validate_local_download_target(&local_path).map_err(|err| err.into_string())?;
    let server = load_server_by_id(app, &server_id)?;
    ensure_boundary_tunnel_running(
        &app.state::<FunctionStore>(),
        &server,
        server_tunnel_function_id,
    )?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let cmd = build_scp_command(
        &endpoint,
        &local_path,
        &remote_path,
        recursive.unwrap_or(false),
        true,
    );
    open_pty_with_command_builder(
        120,
        24,
        cmd,
        app,
        session_store,
        update_functions_for_session_exit,
        None,
    )
}

pub fn cc_start_scp_upload_service(
    app: &AppHandle,
    session_store: &SessionStore,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    overwrite_policy: Option<&str>,
    follow_symlinks: bool,
    verify_checksum: bool,
    resume_token: Option<String>,
    resume_offset_bytes: Option<u64>,
    temp_path_hint: Option<String>,
    open_pty_with_command_builder: fn(
        u16,
        u16,
        portable_pty::CommandBuilder,
        &AppHandle,
        &SessionStore,
        fn(&AppHandle, &crate::stores::FunctionStore, &str, Option<i32>, &str) -> Result<(), String>,
        Option<&str>,
    ) -> Result<String, String>,
    update_functions_for_session_exit: fn(
        &AppHandle,
        &crate::stores::FunctionStore,
        &str,
        Option<i32>,
        &str,
    ) -> Result<(), String>,
) -> Result<StartScpResult, String> {
    let local_path = validate_local_upload_source(&local_path).map_err(|err| err.into_string())?;
    let remote_path =
        normalize_remote_transfer_path(&remote_path, None, "invalid_target_path", false)
            .map_err(|err| err.into_string())?;
    let overwrite_policy = parse_overwrite_policy(overwrite_policy)?;
    enforce_symlink_policy_for_upload(&local_path, follow_symlinks)?;
    ensure_checksum_supported(recursive.unwrap_or(false), verify_checksum)?;
    let server = load_server_by_id(app, &server_id)?;
    let remote_path = resolve_remote_upload_target_path(&server, &local_path, &remote_path)?;
    ensure_boundary_tunnel_running(
        &app.state::<FunctionStore>(),
        &server,
        server_tunnel_function_id,
    )?;
    match check_remote_target_overwrite(&server, &remote_path, overwrite_policy)? {
        OverwriteDecision::Proceed => {}
        OverwriteDecision::Skipped(message) => return Ok(StartScpResult::Skipped(message)),
    }
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let recursive = recursive.unwrap_or(false);
    let resume_request = parse_resume_request(resume_token, resume_offset_bytes, temp_path_hint)?;
    let temp_remote_path = resume_request
        .as_ref()
        .map(|resume| resume.temp_path_hint.clone())
        .unwrap_or_else(|| build_atomic_remote_temp_path(&remote_path));
    let cmd = match resume_request.as_ref() {
        Some(resume) => build_resume_upload_command(
            &endpoint,
            &local_path,
            &temp_remote_path,
            &remote_path,
            recursive,
            verify_checksum,
            resume,
        )?,
        None => build_atomic_upload_command(
            &endpoint,
            &local_path,
            &temp_remote_path,
            &remote_path,
            recursive,
            verify_checksum,
        ),
    };
    let session_id = open_pty_with_command_builder(
        120,
        24,
        cmd,
        app,
        session_store,
        update_functions_for_session_exit,
        None,
    )?;
    Ok(StartScpResult::Started {
        session_id,
        temp_path_hint: Some(temp_remote_path),
    })
}

pub fn cc_scp_download_service(
    app: &AppHandle,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    overwrite_policy: Option<&str>,
    follow_symlinks: bool,
    verify_checksum: bool,
) -> Result<String, String> {
    let remote_path =
        normalize_remote_transfer_path(&remote_path, None, "invalid_source_path", false)
            .map_err(|err| err.into_string())?;
    let local_path =
        validate_local_download_target(&local_path).map_err(|err| err.into_string())?;
    let overwrite_policy = parse_overwrite_policy(overwrite_policy)?;
    enforce_symlink_policy_for_download(follow_symlinks)?;
    ensure_checksum_supported(recursive.unwrap_or(false), verify_checksum)?;
    match check_local_target_overwrite(&local_path, overwrite_policy)? {
        OverwriteDecision::Proceed => {}
        OverwriteDecision::Skipped(message) => return Ok(message),
    }
    let server = load_server_by_id(app, &server_id)?;
    ensure_boundary_tunnel_running(
        &app.state::<FunctionStore>(),
        &server,
        server_tunnel_function_id,
    )?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let remote_spec = format!("{}:{}", ssh_target(&endpoint), remote_path);
    let mut scp = Command::new("scp");
    add_common_ssh_options(&mut scp, "-P", endpoint.port);
    if recursive.unwrap_or(false) {
        scp.arg("-r");
    }
    let o = scp
        .arg(&remote_spec)
        .arg(&local_path)
        .output()
        .map_err(|e| format!("agent_error: scp failed to run: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "agent_error: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    if verify_checksum {
        verify_remote_and_local_sha256(&endpoint, &remote_path, &local_path, "download")?;
        Ok("Download finished. SHA-256 verified.".into())
    } else {
        Ok("Download finished.".into())
    }
}

pub fn cc_start_scp_download_service(
    app: &AppHandle,
    session_store: &SessionStore,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    overwrite_policy: Option<&str>,
    follow_symlinks: bool,
    verify_checksum: bool,
    resume_token: Option<String>,
    resume_offset_bytes: Option<u64>,
    temp_path_hint: Option<String>,
    open_pty_with_command_builder: fn(
        u16,
        u16,
        portable_pty::CommandBuilder,
        &AppHandle,
        &SessionStore,
        fn(&AppHandle, &crate::stores::FunctionStore, &str, Option<i32>, &str) -> Result<(), String>,
        Option<&str>,
    ) -> Result<String, String>,
    update_functions_for_session_exit: fn(
        &AppHandle,
        &crate::stores::FunctionStore,
        &str,
        Option<i32>,
        &str,
    ) -> Result<(), String>,
) -> Result<StartScpResult, String> {
    let remote_path =
        normalize_remote_transfer_path(&remote_path, None, "invalid_source_path", false)
            .map_err(|err| err.into_string())?;
    let local_path =
        validate_local_download_target(&local_path).map_err(|err| err.into_string())?;
    let overwrite_policy = parse_overwrite_policy(overwrite_policy)?;
    enforce_symlink_policy_for_download(follow_symlinks)?;
    ensure_checksum_supported(recursive.unwrap_or(false), verify_checksum)?;
    let resume_request = parse_resume_request(resume_token, resume_offset_bytes, temp_path_hint)?;
    if resume_request.is_none() {
        match check_local_target_overwrite(&local_path, overwrite_policy)? {
            OverwriteDecision::Proceed => {}
            OverwriteDecision::Skipped(message) => return Ok(StartScpResult::Skipped(message)),
        }
    }
    let server = load_server_by_id(app, &server_id)?;
    ensure_boundary_tunnel_running(
        &app.state::<FunctionStore>(),
        &server,
        server_tunnel_function_id,
    )?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let recursive = recursive.unwrap_or(false);
    let local_temp_path = resume_request
        .as_ref()
        .map(|resume| resume.temp_path_hint.clone())
        .unwrap_or_else(|| local_path.clone());
    let cmd = match resume_request.as_ref() {
        Some(resume) => build_resume_download_command(
            &endpoint,
            &local_temp_path,
            &remote_path,
            recursive,
            verify_checksum,
            resume,
        )?,
        None => build_verified_download_command(
            &endpoint,
            &local_temp_path,
            &remote_path,
            recursive,
            verify_checksum,
        ),
    };
    let session_id = open_pty_with_command_builder(
        120,
        24,
        cmd,
        app,
        session_store,
        update_functions_for_session_exit,
        None,
    )?;
    Ok(StartScpResult::Started {
        session_id,
        temp_path_hint: Some(local_temp_path),
    })
}

fn parse_resume_request(
    resume_token: Option<String>,
    resume_offset_bytes: Option<u64>,
    temp_path_hint: Option<String>,
) -> Result<Option<ResumeRequest>, String> {
    match (resume_token, resume_offset_bytes, temp_path_hint) {
        (None, None, None) => Ok(None),
        (Some(resume_token), Some(resume_offset_bytes), Some(temp_path_hint)) => {
            let resume_token = resume_token.trim().to_string();
            let temp_path_hint = temp_path_hint.trim().to_string();
            if resume_token.is_empty() || temp_path_hint.is_empty() {
                return Err("resume_state_lost: Resume state is incomplete or empty".to_string());
            }
            if resume_offset_bytes == 0 {
                return Err("resume_offset_mismatch: resumeOffsetBytes must be greater than zero for resumed transfers".to_string());
            }
            Ok(Some(ResumeRequest {
                resume_token,
                resume_offset_bytes,
                temp_path_hint,
            }))
        }
        _ => Err("resume_state_lost: Resume state is incomplete".to_string()),
    }
}

enum OverwriteDecision {
    Proceed,
    Skipped(String),
}

fn build_atomic_remote_temp_path(remote_path: &str) -> String {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{remote_path}.scp_tmp.{unique}")
}

fn open_sftp_for_server(
    server: &crate::server_mgmt::Server,
) -> Result<ssh2::Session, String> {
    let endpoint = resolve_server_ssh_endpoint(server, "SCP")?;
    let pw = keyring_get_optional(&keyring_server_ssh_password_account(&server.id))?;
    open_remote_sftp_session(
        &endpoint,
        server.connection_method == ConnectionMethod::Boundary,
        pw.as_deref(),
    )
}

fn resolve_remote_upload_target_path(
    server: &crate::server_mgmt::Server,
    local_path: &str,
    remote_path: &str,
) -> Result<String, String> {
    let session = open_sftp_for_server(server).map_err(|err| format!("agent_error: {err}"))?;
    let sftp = session
        .sftp()
        .map_err(|err| format!("agent_error: failed to open sftp session: {err}"))?;
    match sftp.lstat(Path::new(remote_path)) {
        Ok(stat) if matches!(remote_kind_from_file_stat(&stat), RemotePathKind::Directory) => {
            let file_name = Path::new(local_path)
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    "invalid_source_path: Local upload source must have a file or folder name"
                        .to_string()
                })?;
            Ok(join_remote_path(remote_path, file_name))
        }
        Ok(_) => Ok(remote_path.to_string()),
        Err(_) => Ok(remote_path.to_string()),
    }
}

fn join_remote_path(base: &str, child: &str) -> String {
    let trimmed_base = base.trim_end_matches('/');
    if trimmed_base.is_empty() {
        format!("/{}", child)
    } else {
        format!("{}/{}", trimmed_base, child)
    }
}

fn finalize_atomic_remote_upload(
    endpoint: &crate::models::SshEndpoint,
    temp_remote_path: &str,
    remote_path: &str,
) -> Result<(), String> {
    let remote_command = build_remote_finalize_command(temp_remote_path, remote_path);
    let mut ssh = Command::new("ssh");
    add_common_ssh_options(&mut ssh, "-p", endpoint.port);
    let output = ssh
        .arg(ssh_target(endpoint))
        .arg(remote_command)
        .output()
        .map_err(|err| format!("agent_error: failed to finalize atomic upload: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "agent_error: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn build_remote_finalize_command(temp_remote_path: &str, remote_path: &str) -> String {
    format!(
        "if mv -f -- {tmp} {dst}; then exit 0; else code=$?; rm -rf -- {tmp}; exit $code; fi",
        tmp = shell_quote(temp_remote_path),
        dst = shell_quote(remote_path),
    )
}

fn build_atomic_upload_command(
    endpoint: &crate::models::SshEndpoint,
    local_path: &str,
    temp_remote_path: &str,
    remote_path: &str,
    recursive: bool,
    verify_checksum: bool,
) -> CommandBuilder {
    let remote_spec = format!("{}:{}", ssh_target(endpoint), temp_remote_path);
    let remote_finalize = build_remote_finalize_command(temp_remote_path, remote_path);
    #[cfg(target_os = "windows")]
    {
        let mut parts = vec![
            "& scp".to_string(),
            powershell_quote("-P"),
            powershell_quote(&endpoint.port.to_string()),
            powershell_quote("-o"),
            powershell_quote("StrictHostKeyChecking=accept-new"),
            powershell_quote("-o"),
            powershell_quote("MACs=hmac-sha2-512"),
        ];
        if recursive {
            parts.push(powershell_quote("-r"));
        }
        parts.push(powershell_quote(local_path));
        parts.push(powershell_quote(&remote_spec));
        parts.push("; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }".to_string());
        parts.push("& ssh".to_string());
        parts.push(powershell_quote("-p"));
        parts.push(powershell_quote(&endpoint.port.to_string()));
        parts.push(powershell_quote("-o"));
        parts.push(powershell_quote("StrictHostKeyChecking=accept-new"));
        parts.push(powershell_quote("-o"));
        parts.push(powershell_quote("MACs=hmac-sha2-512"));
        parts.push(powershell_quote(&ssh_target(endpoint)));
        parts.push(powershell_quote(&remote_finalize));
        if verify_checksum {
            parts.push("; Write-Host 'Running SHA-256 checksum verification...'".to_string());
            parts.push(format!(
                "$localHash = (Get-FileHash -Algorithm SHA256 -LiteralPath {}).Hash.ToLowerInvariant()",
                powershell_quote(local_path)
            ));
            parts.push(build_powershell_remote_hash_command(endpoint, remote_path));
            parts.push("Write-Host ('Local SHA-256: ' + $localHash)".to_string());
            parts.push("Write-Host ('Remote SHA-256: ' + $remoteHash)".to_string());
            parts.push("if ($localHash -ne $remoteHash) { Write-Error ('checksum_mismatch: SHA-256 mismatch after upload (local=' + $localHash + ', remote=' + $remoteHash + ')'); exit 42 }".to_string());
        }
        let script = parts.join(" ");
        let mut cmd = CommandBuilder::new("powershell");
        cmd.arg("-NoLogo");
        cmd.arg("-NoProfile");
        cmd.arg("-Command");
        cmd.arg(script);
        cmd
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut scp_parts = vec![
            "scp".to_string(),
            shell_quote("-P"),
            shell_quote(&endpoint.port.to_string()),
            shell_quote("-o"),
            shell_quote("StrictHostKeyChecking=accept-new"),
            shell_quote("-o"),
            shell_quote("MACs=hmac-sha2-512"),
        ];
        if recursive {
            scp_parts.push(shell_quote("-r"));
        }
        scp_parts.push(shell_quote(local_path));
        scp_parts.push(shell_quote(&remote_spec));
        let ssh_parts = vec![
            "ssh".to_string(),
            shell_quote("-p"),
            shell_quote(&endpoint.port.to_string()),
            shell_quote("-o"),
            shell_quote("StrictHostKeyChecking=accept-new"),
            shell_quote("-o"),
            shell_quote("MACs=hmac-sha2-512"),
            shell_quote(&ssh_target(endpoint)),
            shell_quote(&remote_finalize),
        ];
        let mut script = format!("{} && {}", scp_parts.join(" "), ssh_parts.join(" "));
        if verify_checksum {
            let verify_script = format!(
                "printf '%s\\n' 'Running SHA-256 checksum verification...' \
&& local_hash=$({local_hash_cmd}) \
&& remote_hash=$({remote_hash_cmd} | awk '{{print $1}}') \
&& printf 'Local SHA-256: %s\\n' \"$local_hash\" \
&& printf 'Remote SHA-256: %s\\n' \"$remote_hash\" \
&& [ \"$local_hash\" = \"$remote_hash\" ] \
|| {{ printf 'checksum_mismatch: SHA-256 mismatch after upload (local=%s, remote=%s)\\n' \"${{local_hash:-<unavailable>}}\" \"${{remote_hash:-<unavailable>}}\" >&2; exit 42; }}",
                local_hash_cmd = build_local_sha256_shell_command(local_path),
                remote_hash_cmd = build_ssh_remote_command(endpoint, &build_remote_sha256_command(remote_path)),
            );
            script = format!("{script} && {verify_script}");
        }
        let mut cmd = CommandBuilder::new("sh");
        cmd.arg("-lc");
        cmd.arg(script);
        cmd
    }
}

fn build_resume_upload_command(
    endpoint: &crate::models::SshEndpoint,
    local_path: &str,
    temp_remote_path: &str,
    remote_path: &str,
    recursive: bool,
    verify_checksum: bool,
    resume: &ResumeRequest,
) -> Result<CommandBuilder, String> {
    let _ = &resume.resume_token;
    if recursive {
        return Err(
            "resume_not_supported: Upload resume is not supported for recursive transfers"
                .to_string(),
        );
    }
    #[cfg(target_os = "windows")]
    {
        let _ = endpoint;
        let _ = local_path;
        let _ = temp_remote_path;
        let _ = remote_path;
        let _ = verify_checksum;
        return Err(
            "resume_not_supported: Upload resume is not supported on Windows yet".to_string(),
        );
    }
    #[cfg(not(target_os = "windows"))]
    {
        let resume_script = format!(
            "local_size=$(wc -c < {local_path}) \
&& [ {offset} -le \"$local_size\" ] \
|| {{ printf 'resume_offset_mismatch: resume offset %s exceeds local file size %s\\n' {offset} \"$local_size\" >&2; exit 43; }} \
&& if [ {offset} -lt \"$local_size\" ]; then \
  tail -c +$(({offset} + 1)) -- {local_path} | {append_cmd}; \
fi",
            local_path = shell_quote(local_path),
            offset = resume.resume_offset_bytes,
            append_cmd = build_ssh_remote_command(
                endpoint,
                &build_remote_append_command(temp_remote_path, resume.resume_offset_bytes),
            ),
        );
        let finalize_cmd = build_ssh_remote_command(
            endpoint,
            &build_remote_finalize_command(temp_remote_path, remote_path),
        );
        let mut script = format!("{resume_script} && {finalize_cmd}");
        if verify_checksum {
            let verify_script = format!(
                "printf '%s\\n' 'Running SHA-256 checksum verification...' \
&& local_hash=$({local_hash_cmd}) \
&& remote_hash=$({remote_hash_cmd} | awk '{{print $1}}') \
&& printf 'Local SHA-256: %s\\n' \"$local_hash\" \
&& printf 'Remote SHA-256: %s\\n' \"$remote_hash\" \
&& [ \"$local_hash\" = \"$remote_hash\" ] \
|| {{ printf 'checksum_mismatch: SHA-256 mismatch after upload (local=%s, remote=%s)\\n' \"${{local_hash:-<unavailable>}}\" \"${{remote_hash:-<unavailable>}}\" >&2; exit 42; }}",
                local_hash_cmd = build_local_sha256_shell_command(local_path),
                remote_hash_cmd = build_ssh_remote_command(endpoint, &build_remote_sha256_command(remote_path)),
            );
            script = format!("{script} && {verify_script}");
        }
        let mut cmd = CommandBuilder::new("sh");
        cmd.arg("-lc");
        cmd.arg(script);
        Ok(cmd)
    }
}

fn build_verified_download_command(
    endpoint: &crate::models::SshEndpoint,
    local_path: &str,
    remote_path: &str,
    recursive: bool,
    verify_checksum: bool,
) -> CommandBuilder {
    if !verify_checksum {
        return build_scp_command(endpoint, local_path, remote_path, recursive, true);
    }
    let remote_spec = format!("{}:{}", ssh_target(endpoint), remote_path);
    #[cfg(target_os = "windows")]
    {
        let mut parts = vec![
            "& scp".to_string(),
            powershell_quote("-P"),
            powershell_quote(&endpoint.port.to_string()),
            powershell_quote("-o"),
            powershell_quote("StrictHostKeyChecking=accept-new"),
            powershell_quote("-o"),
            powershell_quote("MACs=hmac-sha2-512"),
        ];
        if recursive {
            parts.push(powershell_quote("-r"));
        }
        parts.push(powershell_quote(&remote_spec));
        parts.push(powershell_quote(local_path));
        parts.push("; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }".to_string());
        parts.push("Write-Host 'Running SHA-256 checksum verification...'".to_string());
        parts.push(build_powershell_remote_hash_command(endpoint, remote_path));
        parts.push(format!(
            "$localHash = (Get-FileHash -Algorithm SHA256 -LiteralPath {}).Hash.ToLowerInvariant()",
            powershell_quote(local_path)
        ));
        parts.push("Write-Host ('Local SHA-256: ' + $localHash)".to_string());
        parts.push("Write-Host ('Remote SHA-256: ' + $remoteHash)".to_string());
        parts.push("if ($localHash -ne $remoteHash) { Write-Error ('checksum_mismatch: SHA-256 mismatch after download (local=' + $localHash + ', remote=' + $remoteHash + ')'); exit 42 }".to_string());
        let script = parts.join(" ");
        let mut cmd = CommandBuilder::new("powershell");
        cmd.arg("-NoLogo");
        cmd.arg("-NoProfile");
        cmd.arg("-Command");
        cmd.arg(script);
        cmd
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut scp_parts = vec![
            "scp".to_string(),
            shell_quote("-P"),
            shell_quote(&endpoint.port.to_string()),
            shell_quote("-o"),
            shell_quote("StrictHostKeyChecking=accept-new"),
            shell_quote("-o"),
            shell_quote("MACs=hmac-sha2-512"),
        ];
        if recursive {
            scp_parts.push(shell_quote("-r"));
        }
        scp_parts.push(shell_quote(&remote_spec));
        scp_parts.push(shell_quote(local_path));
        let verify_script = format!(
            "printf '%s\\n' 'Running SHA-256 checksum verification...' \
&& remote_hash=$({remote_hash_cmd} | awk '{{print $1}}') \
&& local_hash=$({local_hash_cmd}) \
&& printf 'Remote SHA-256: %s\\n' \"$remote_hash\" \
&& printf 'Local SHA-256: %s\\n' \"$local_hash\" \
&& [ \"$local_hash\" = \"$remote_hash\" ] \
|| {{ printf 'checksum_mismatch: SHA-256 mismatch after download (local=%s, remote=%s)\\n' \"${{local_hash:-<unavailable>}}\" \"${{remote_hash:-<unavailable>}}\" >&2; exit 42; }}",
            remote_hash_cmd = build_ssh_remote_command(endpoint, &build_remote_sha256_command(remote_path)),
            local_hash_cmd = build_local_sha256_shell_command(local_path),
        );
        let script = format!("{} && {}", scp_parts.join(" "), verify_script);
        let mut cmd = CommandBuilder::new("sh");
        cmd.arg("-lc");
        cmd.arg(script);
        cmd
    }
}

fn build_resume_download_command(
    endpoint: &crate::models::SshEndpoint,
    local_path: &str,
    remote_path: &str,
    recursive: bool,
    verify_checksum: bool,
    resume: &ResumeRequest,
) -> Result<CommandBuilder, String> {
    let _ = &resume.resume_token;
    if recursive {
        return Err(
            "resume_not_supported: Download resume is not supported for recursive transfers"
                .to_string(),
        );
    }
    #[cfg(target_os = "windows")]
    {
        let _ = endpoint;
        let _ = local_path;
        let _ = remote_path;
        let _ = verify_checksum;
        return Err(
            "resume_not_supported: Download resume is not supported on Windows yet".to_string(),
        );
    }
    #[cfg(not(target_os = "windows"))]
    {
        let resume_script = format!(
            "local_size=$(wc -c < {local_path}) \
&& [ \"$local_size\" -eq {offset} ] \
|| {{ printf 'resume_offset_mismatch: local partial size %s does not match resume offset %s\\n' \"$local_size\" {offset} >&2; exit 43; }} \
&& if remote_size=$({remote_size_cmd}); then :; else exit $?; fi \
&& [ {offset} -le \"$remote_size\" ] \
|| {{ printf 'resume_offset_mismatch: resume offset %s exceeds remote file size %s\\n' {offset} \"$remote_size\" >&2; exit 43; }} \
&& if [ {offset} -lt \"$remote_size\" ]; then \
  {download_cmd} >> {local_path}; \
fi",
            local_path = shell_quote(local_path),
            offset = resume.resume_offset_bytes,
            remote_size_cmd = build_ssh_remote_command(endpoint, &build_remote_size_command(remote_path)),
            download_cmd = build_ssh_remote_command(endpoint, &build_remote_tail_command(remote_path, resume.resume_offset_bytes)),
        );
        let mut script = resume_script;
        if verify_checksum {
            let verify_script = format!(
                "printf '%s\\n' 'Running SHA-256 checksum verification...' \
&& remote_hash=$({remote_hash_cmd} | awk '{{print $1}}') \
&& local_hash=$({local_hash_cmd}) \
&& printf 'Remote SHA-256: %s\\n' \"$remote_hash\" \
&& printf 'Local SHA-256: %s\\n' \"$local_hash\" \
&& [ \"$local_hash\" = \"$remote_hash\" ] \
|| {{ printf 'checksum_mismatch: SHA-256 mismatch after download (local=%s, remote=%s)\\n' \"${{local_hash:-<unavailable>}}\" \"${{remote_hash:-<unavailable>}}\" >&2; exit 42; }}",
                remote_hash_cmd = build_ssh_remote_command(endpoint, &build_remote_sha256_command(remote_path)),
                local_hash_cmd = build_local_sha256_shell_command(local_path),
            );
            script = format!("{script} && {verify_script}");
        }
        let mut cmd = CommandBuilder::new("sh");
        cmd.arg("-lc");
        cmd.arg(script);
        Ok(cmd)
    }
}

fn build_remote_append_command(temp_remote_path: &str, expected_offset: u64) -> String {
    format!(
        "sh -lc {}",
        shell_quote(&format!(
            "actual_size=$(wc -c < {path}) && [ \"$actual_size\" -eq {offset} ] || {{ printf 'resume_offset_mismatch: remote temp size %s does not match resume offset %s\\n' \"${{actual_size:-<unavailable>}}\" {offset} >&2; exit 43; }}; cat >> {path}",
            path = shell_quote(temp_remote_path),
            offset = expected_offset,
        ))
    )
}

fn build_remote_tail_command(remote_path: &str, expected_offset: u64) -> String {
    format!(
        "sh -lc {}",
        shell_quote(&format!(
            "remote_size=$(wc -c < {path}) && [ {offset} -le \"$remote_size\" ] || {{ printf 'resume_offset_mismatch: resume offset %s exceeds remote file size %s\\n' {offset} \"${{remote_size:-<unavailable>}}\" >&2; exit 43; }}; tail -c +$(({offset} + 1)) -- {path}",
            path = shell_quote(remote_path),
            offset = expected_offset,
        ))
    )
}

fn build_remote_size_command(remote_path: &str) -> String {
    format!(
        "sh -lc {}",
        shell_quote(&format!("wc -c < {}", shell_quote(remote_path)))
    )
}

fn shell_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "windows")]
fn powershell_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "''"))
}

fn ensure_checksum_supported(recursive: bool, verify_checksum: bool) -> Result<(), String> {
    if recursive && verify_checksum {
        return Err(
            "agent_error: checksum verification for recursive transfers is not supported yet"
                .to_string(),
        );
    }
    Ok(())
}

fn enforce_symlink_policy_for_upload(
    local_path: &str,
    follow_symlinks: bool,
) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(local_path)
        .map_err(|err| format!("agent_error: failed to inspect local upload source: {err}"))?;
    if metadata.file_type().is_symlink() && !follow_symlinks {
        return Err(
            "invalid_source_path: Local upload source is a symlink. Retry with followSymlinks=true to upload the target."
                .to_string(),
        );
    }
    Ok(())
}

fn enforce_symlink_policy_for_download(follow_symlinks: bool) -> Result<(), String> {
    if !follow_symlinks {
        return Err(
            "agent_error: followSymlinks=false is not supported for downloads yet".to_string(),
        );
    }
    Ok(())
}

fn verify_local_and_remote_sha256(
    endpoint: &crate::models::SshEndpoint,
    local_path: &str,
    remote_path: &str,
    operation: &str,
) -> Result<(), String> {
    let local_hash = compute_local_sha256(local_path)?;
    let remote_hash = compute_remote_sha256(endpoint, remote_path)?;
    ensure_matching_sha256(&local_hash, &remote_hash, operation)
}

fn verify_remote_and_local_sha256(
    endpoint: &crate::models::SshEndpoint,
    remote_path: &str,
    local_path: &str,
    operation: &str,
) -> Result<(), String> {
    let remote_hash = compute_remote_sha256(endpoint, remote_path)?;
    let local_hash = compute_local_sha256(local_path)?;
    ensure_matching_sha256(&local_hash, &remote_hash, operation)
}

fn ensure_matching_sha256(
    local_hash: &str,
    remote_hash: &str,
    operation: &str,
) -> Result<(), String> {
    if local_hash.eq_ignore_ascii_case(remote_hash) {
        Ok(())
    } else {
        Err(format!(
            "checksum_mismatch: SHA-256 mismatch after {operation} (local={local_hash}, remote={remote_hash})"
        ))
    }
}

fn compute_local_sha256(path: &str) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("agent_error: failed to open file for checksum: {err}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("agent_error: failed to read file for checksum: {err}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn compute_remote_sha256(
    endpoint: &crate::models::SshEndpoint,
    remote_path: &str,
) -> Result<String, String> {
    let mut ssh = Command::new("ssh");
    add_common_ssh_options(&mut ssh, "-p", endpoint.port);
    let output = ssh
        .arg(ssh_target(endpoint))
        .arg(build_remote_sha256_command(remote_path))
        .output()
        .map_err(|err| format!("agent_error: failed to run remote checksum command: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "agent_error: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_sha256_output(&output.stdout)
}

fn parse_sha256_output(stdout: &[u8]) -> Result<String, String> {
    let output = String::from_utf8_lossy(stdout);
    let line = output
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .ok_or_else(|| "agent_error: checksum command returned empty output".to_string())?;
    let hash = line
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if hash.len() == 64 && hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Ok(hash)
    } else {
        Err(format!(
            "agent_error: failed to parse checksum output: {line}"
        ))
    }
}

fn build_remote_sha256_command(remote_path: &str) -> String {
    format!(
        "sh -lc {}",
        shell_quote(&format!(
            "if command -v sha256sum >/dev/null 2>&1; then sha256sum -- {path}; \
             elif command -v shasum >/dev/null 2>&1; then shasum -a 256 -- {path}; \
             else echo 'agent_error: remote SHA-256 tool unavailable' >&2; exit 127; fi",
            path = shell_quote(remote_path),
        ))
    )
}

fn build_local_sha256_shell_command(local_path: &str) -> String {
    format!(
        "if command -v sha256sum >/dev/null 2>&1; then sha256sum -- {path}; \
         elif command -v shasum >/dev/null 2>&1; then shasum -a 256 -- {path}; \
         else echo 'agent_error: local SHA-256 tool unavailable' >&2; exit 127; fi | awk '{{print $1}}'",
        path = shell_quote(local_path)
    )
}

fn build_ssh_remote_command(endpoint: &crate::models::SshEndpoint, remote_command: &str) -> String {
    [
        "ssh".to_string(),
        shell_quote("-p"),
        shell_quote(&endpoint.port.to_string()),
        shell_quote("-o"),
        shell_quote("StrictHostKeyChecking=accept-new"),
        shell_quote("-o"),
        shell_quote("MACs=hmac-sha2-512"),
        shell_quote(&ssh_target(endpoint)),
        shell_quote(remote_command),
    ]
    .join(" ")
}

#[cfg(target_os = "windows")]
fn build_powershell_remote_hash_command(
    endpoint: &crate::models::SshEndpoint,
    remote_path: &str,
) -> String {
    format!(
        "$remoteHash = (& ssh {} {} {} {} {} {} {} {} | Select-Object -First 1).Trim().ToLowerInvariant()",
        powershell_quote("-p"),
        powershell_quote(&endpoint.port.to_string()),
        powershell_quote("-o"),
        powershell_quote("StrictHostKeyChecking=accept-new"),
        powershell_quote("-o"),
        powershell_quote("MACs=hmac-sha2-512"),
        powershell_quote(&ssh_target(endpoint)),
        powershell_quote(&build_remote_sha256_command(remote_path)),
    )
}

fn check_remote_target_overwrite(
    server: &crate::server_mgmt::Server,
    remote_path: &str,
    overwrite_policy: OverwritePolicy,
) -> Result<OverwriteDecision, String> {
    if overwrite_policy == OverwritePolicy::Force {
        return Ok(OverwriteDecision::Proceed);
    }
    let session = open_sftp_for_server(server).map_err(|err| format!("agent_error: {err}"))?;
    let sftp = session
        .sftp()
        .map_err(|err| format!("agent_error: failed to open sftp session: {err}"))?;
    match sftp.lstat(Path::new(remote_path)) {
        Ok(stat) => {
            if matches!(remote_kind_from_file_stat(&stat), RemotePathKind::Directory) {
                return Ok(OverwriteDecision::Proceed);
            }
            match overwrite_policy {
                OverwritePolicy::Prompt => Err(
                    "path_already_exists: Remote target already exists. Retry with overwritePolicy=force or skip."
                        .to_string(),
                ),
                OverwritePolicy::Skip => Ok(OverwriteDecision::Skipped(
                    "Skipped upload because the remote target already exists.".to_string(),
                )),
                OverwritePolicy::Force => Ok(OverwriteDecision::Proceed),
            }
        }
        Err(_) => Ok(OverwriteDecision::Proceed),
    }
}

fn check_local_target_overwrite(
    local_path: &str,
    overwrite_policy: OverwritePolicy,
) -> Result<OverwriteDecision, String> {
    if overwrite_policy == OverwritePolicy::Force {
        return Ok(OverwriteDecision::Proceed);
    }
    let path = Path::new(local_path);
    if !path.exists() || path.is_dir() {
        return Ok(OverwriteDecision::Proceed);
    }
    match overwrite_policy {
        OverwritePolicy::Prompt => Err(
            "path_already_exists: Local target already exists. Retry with overwritePolicy=force or skip."
                .to_string(),
        ),
        OverwritePolicy::Skip => Ok(OverwriteDecision::Skipped(
            "Skipped download because the local target already exists.".to_string(),
        )),
        OverwritePolicy::Force => Ok(OverwriteDecision::Proceed),
    }
}
