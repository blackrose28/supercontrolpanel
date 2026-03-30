#[cfg(target_os = "windows")]
use std::{fs, path::PathBuf};

#[cfg(target_os = "windows")]
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
#[cfg(not(target_os = "windows"))]
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use tauri::{AppHandle, Manager};

use crate::models::{AppConfig, FunctionStatus, OpenVpnConfig, PtyExitPayload};
use crate::stores::FunctionStore;

#[cfg(target_os = "windows")]
use tauri::Emitter;
#[cfg(target_os = "windows")]
use crate::{
    models::OpenVpnLogPayload,
    services::lock_err,
    stores::{OpenVpnAdminSession, OpenVpnAdminStore},
};

#[cfg(target_os = "windows")]
use crate::{
    models::{HelperStatusFile, OpenVpnLaunchConfig},
};
#[cfg(not(target_os = "windows"))]
use crate::stores::SessionStore;

#[cfg(target_os = "windows")]
pub fn runtime_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config dir: {e}"))?
        .join("runtime");
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create runtime dir: {e}"))?;
    Ok(dir)
}

#[cfg(target_os = "windows")]
pub fn openvpn_runtime_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = runtime_dir(app)?.join("openvpn");
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create openvpn runtime dir: {e}"))?;
    Ok(dir)
}

#[cfg(target_os = "windows")]
pub fn write_openvpn_helper_script(
    app: &AppHandle,
    openvpn_helper_script_name: &str,
) -> Result<PathBuf, String> {
    let path = runtime_dir(app)?.join(openvpn_helper_script_name);
    let script = r#"
param(
  [Parameter(Mandatory=$true)][string]$ConfigPath,
  [Parameter(Mandatory=$true)][string]$StatusPath,
  [Parameter(Mandatory=$true)][string]$LogPath,
  [Parameter(Mandatory=$true)][string]$ErrPath,
  [Parameter(Mandatory=$true)][string]$StopPath,
  [Parameter(Mandatory=$true)][string]$HelperPidPath
)

$ErrorActionPreference = 'Stop'
try {
  [System.IO.File]::WriteAllText($HelperPidPath, [string][System.Diagnostics.Process]::GetCurrentProcess().Id)
} catch { }

function Write-Status([string]$State, [string]$Message) {
  $payload = [ordered]@{
    state = $State
    message = $Message
    updatedAtMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  }
  ($payload | ConvertTo-Json -Compress) | Set-Content -Path $StatusPath -Encoding UTF8
}

try {
  if (!(Test-Path -LiteralPath $ConfigPath)) {
    throw "Launch config not found: $ConfigPath"
  }

  $cfg = Get-Content -LiteralPath $ConfigPath -Raw | ConvertFrom-Json
  if ([string]::IsNullOrWhiteSpace($cfg.executablePath)) {
    throw "OpenVPN executable path is missing."
  }
  $authFile = [System.IO.Path]::GetTempFileName()
  try {
    "$($cfg.username)`r`n$($cfg.password)`r`n" | Set-Content -LiteralPath $authFile -NoNewline -Encoding ASCII
    $exe = $cfg.executablePath
    $args = $cfg.args
    if ([string]::IsNullOrWhiteSpace($args)) {
      $args = "--auth-user-pass `"$authFile`" --auth-nocache"
    } else {
      $args = "$args --auth-user-pass `"$authFile`" --auth-nocache"
    }

    $killWithHelper = $false
    if (($cfg | Get-Member -Name terminateOpenVpnWhenHelperExits -MemberType NoteProperty) -and ($cfg.terminateOpenVpnWhenHelperExits -eq $true)) {
      $killWithHelper = $true
    }

    Write-Status "starting" "Starting OpenVPN process"
    $jobHandle = [IntPtr]::Zero
    if ($killWithHelper) {
      $scpJobType = @'
using System;
using System.Runtime.InteropServices;
[StructLayout(LayoutKind.Sequential, Pack=8)]
public struct ScpJobBasic {
  public Int64 PerProcessUserTimeLimit, PerJobUserTimeLimit;
  public UInt32 LimitFlags;
  public UIntPtr MinimumWorkingSetSize, MaximumWorkingSetSize;
  public UInt32 ActiveProcessLimit;
  public UIntPtr Affinity;
  public UInt32 PriorityClass, SchedulingClass;
}
[StructLayout(LayoutKind.Sequential, Pack=8)]
public struct ScpJobIo {
  public UInt64 ReadOperationCount, WriteOperationCount, OtherOperationCount;
  public UInt64 ReadTransferCount, WriteTransferCount, OtherTransferCount;
}
[StructLayout(LayoutKind.Sequential, Pack=8)]
public struct ScpJobExt {
  public ScpJobBasic BasicLimitInformation;
  public ScpJobIo IoInfo;
  public UIntPtr ProcessMemoryLimit, JobMemoryLimit, PeakProcessMemoryUsed, PeakJobMemoryUsed;
}
public static class ScpOvpnJob {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr CreateJobObject(IntPtr a,string n);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool SetInformationJobObject(IntPtr h,int c,IntPtr i,uint s);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool AssignProcessToJobObject(IntPtr j,IntPtr p);
}
'@
      Add-Type -TypeDefinition $scpJobType -ErrorAction Stop
      $jobHandle = [ScpOvpnJob]::CreateJobObject([IntPtr]::Zero, $null)
      if ($jobHandle -eq [IntPtr]::Zero) { throw "CreateJobObject failed" }
      $jl = New-Object ScpJobExt
      $jl.BasicLimitInformation.LimitFlags = [UInt32]0x2000
      $sz = [System.Runtime.InteropServices.Marshal]::SizeOf([type][ScpJobExt])
      $pNative = [System.Runtime.InteropServices.Marshal]::AllocHGlobal($sz)
      try {
        [System.Runtime.InteropServices.Marshal]::StructureToPtr($jl, $pNative, $false)
        if (-not [ScpOvpnJob]::SetInformationJobObject($jobHandle, 9, $pNative, [uint32]$sz)) {
          $le = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
          throw "SetInformationJobObject failed (win32 $le)"
        }
      } finally {
        [System.Runtime.InteropServices.Marshal]::FreeHGlobal($pNative)
      }
    }

    $proc = Start-Process -FilePath $exe `
      -ArgumentList $args `
      -WindowStyle Hidden `
      -RedirectStandardOutput $LogPath `
      -RedirectStandardError $ErrPath `
      -PassThru

    if ($killWithHelper) {
      if (-not [ScpOvpnJob]::AssignProcessToJobObject($jobHandle, $proc.Handle)) {
        $e = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "AssignProcessToJobObject failed (win32 $e)"
      }
      $global:ScpOvpnJobKeepAlive = $jobHandle
    }

    $runMsg = if ($killWithHelper) { "OpenVPN running (stops when helper window closes)" } else { "OpenVPN started as Administrator (keeps running if helper closes)" }
    Write-Status "running" $runMsg

    while (-not $proc.HasExited) {
      if (Test-Path -LiteralPath $StopPath) {
        Write-Status "stopping" "Stop requested"
        try { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } catch {}
        break
      }
      Start-Sleep -Milliseconds 400
    }

    $proc.WaitForExit()
    $code = $proc.ExitCode
    if (Test-Path -LiteralPath $StopPath) {
      Write-Status "stopped" "Stopped by request (exit code $code)"
    } elseif ($code -eq 0) {
      Write-Status "stopped" "OpenVPN exited normally"
    } else {
      Write-Status "error" "OpenVPN exited with code $code"
    }
  } finally {
    Remove-Item -LiteralPath $authFile -ErrorAction SilentlyContinue
  }
} catch {
  $msg = $_.Exception.Message
  Write-Status "error" $msg
  Add-Content -LiteralPath $LogPath -Value "[helper-error] $msg"
}
"#;

    fs::write(&path, script).map_err(|e| format!("failed to write helper script: {e}"))?;
    Ok(path)
}

#[cfg(target_os = "windows")]
pub fn escape_ps_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(target_os = "windows")]
pub fn set_openvpn_admin_session(
    admin_store: &OpenVpnAdminStore,
    session: Option<OpenVpnAdminSession>,
) -> Result<(), String> {
    let mut slot = lock_err(&admin_store.session)?;
    *slot = session;
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn get_openvpn_admin_session(
    admin_store: &OpenVpnAdminStore,
) -> Result<Option<OpenVpnAdminSession>, String> {
    let slot = lock_err(&admin_store.session)?;
    Ok(slot.as_ref().map(|s| OpenVpnAdminSession {
        id: s.id.clone(),
        stop_path: s.stop_path.clone(),
    }))
}

#[cfg(target_os = "windows")]
pub fn emit_openvpn_log(app: &AppHandle, session_id: &str, function_id: &str, data: String) {
    let _ = app.emit(
        "openvpn-log",
        OpenVpnLogPayload {
            function_id: function_id.to_string(),
            session_id: session_id.to_string(),
            data,
        },
    );
}

pub fn prepare_openvpn_start(
    saved_config: &AppConfig,
    username: String,
    password: String,
    normalized_openvpn_config: fn(&OpenVpnConfig) -> OpenVpnConfig,
    generate_openvpn_password: fn(&str, &str) -> Result<String, String>,
) -> Result<(OpenVpnConfig, String, String), String> {
    let resolved_openvpn = normalized_openvpn_config(&saved_config.openvpn);
    let resolved_username = if username.trim().is_empty() {
        resolved_openvpn.username.clone()
    } else {
        username
    };
    let resolved_password = if password.is_empty() {
        resolved_openvpn.password.clone()
    } else {
        password
    };
    let resolved_otp_secret = resolved_openvpn.otp_secret.clone();

    if resolved_openvpn.executable_path.trim().is_empty() {
        return Err("openvpn executable path cannot be empty".to_string());
    }
    if resolved_openvpn.ovpn_path.trim().is_empty() {
        return Err("ovpn file path cannot be empty".to_string());
    }
    if resolved_username.trim().is_empty() {
        return Err("username cannot be empty".to_string());
    }
    if resolved_password.is_empty() {
        return Err("password cannot be empty".to_string());
    }

    let combined_password = generate_openvpn_password(&resolved_password, &resolved_otp_secret)?;

    Ok((resolved_openvpn, resolved_username, combined_password))
}

#[cfg(not(target_os = "windows"))]
#[allow(clippy::too_many_arguments)]
pub fn start_openvpn_pty_session(
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    saved_config: &AppConfig,
    resolved_openvpn: OpenVpnConfig,
    resolved_username: String,
    combined_password: String,
    openvpn_function_id: &str,
    lock_err_statuses: fn(
        &std::sync::Mutex<std::collections::HashMap<String, FunctionStatus>>,
    ) -> Result<
        std::sync::MutexGuard<'_, std::collections::HashMap<String, FunctionStatus>>,
        String,
    >,
    close_pty_internal: fn(&str, &SessionStore) -> Result<Option<PtyExitPayload>, String>,
    upsert_function_status: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        &str,
        &str,
        Option<String>,
    ) -> Result<(), String>,
    open_pty_internal: fn(u16, u16, &AppHandle, &SessionStore) -> Result<String, String>,
    build_openvpn_command_unix: fn(&OpenVpnConfig) -> Result<String, String>,
    run_openvpn_automation: fn(AppHandle, String, String, String, String, String),
) -> Result<String, String> {
    let current = lock_err_statuses(&function_store.statuses)?
        .get(openvpn_function_id)
        .cloned();
    if let Some(current) = current {
        if let Some(existing_session_id) = current.session_id {
            let _ = close_pty_internal(&existing_session_id, session_store);
        }
    }

    upsert_function_status(
        app,
        function_store,
        openvpn_function_id,
        "starting",
        "Opening PTY session",
        None,
    )?;

    let session_id = open_pty_internal(
        saved_config.app.default_terminal_cols,
        saved_config.app.default_terminal_rows,
        app,
        session_store,
    )?;

    upsert_function_status(
        app,
        function_store,
        openvpn_function_id,
        "starting",
        "PTY session opened",
        Some(session_id.clone()),
    )?;

    let automation_app = app.clone();
    let automation_session_id = session_id.clone();
    let command_line = build_openvpn_command_unix(&resolved_openvpn)?;
    thread::spawn(move || {
        run_openvpn_automation(
            automation_app,
            automation_session_id,
            command_line,
            resolved_username,
            combined_password,
            String::new(),
        );
    });

    Ok(session_id)
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
#[cfg(not(target_os = "windows"))]
#[allow(clippy::too_many_arguments)]
pub fn run_openvpn_automation(
    app: AppHandle,
    session_id: String,
    command: String,
    username: String,
    password: String,
    otp_secret: String,
    openvpn_function_id: &str,
    add_watcher: fn(&SessionStore, &str, Sender<String>) -> Result<(), String>,
    write_pty_internal: fn(&str, &str, &SessionStore) -> Result<(), String>,
    upsert_function_status: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        &str,
        &str,
        Option<String>,
    ) -> Result<(), String>,
    generate_openvpn_password: fn(&str, &str) -> Result<String, String>,
    mark_openvpn_connected: fn(&AppHandle),
    adapt_command_for_shell: fn(&str) -> String,
    lock_err_sessions: fn(
        &std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<crate::stores::PtySession>>>,
    ) -> Result<
        std::sync::MutexGuard<'_, std::collections::HashMap<String, std::sync::Arc<crate::stores::PtySession>>>,
        String,
    >,
) {
    let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();
    let session_state = app.state::<SessionStore>();
    if add_watcher(&session_state, &session_id, tx).is_err() {
        let function_state = app.state::<FunctionStore>();
        let _ = upsert_function_status(
            &app,
            &function_state,
            openvpn_function_id,
            "error",
            "Failed to watch PTY output",
            Some(session_id),
        );
        return;
    }

    let command_line = adapt_command_for_shell(&command);
    if write_pty_internal(&session_id, &format!("{command_line}\r"), &session_state).is_err() {
        let function_state = app.state::<FunctionStore>();
        let _ = upsert_function_status(
            &app,
            &function_state,
            openvpn_function_id,
            "error",
            "Failed to send OpenVPN command",
            Some(session_id),
        );
        return;
    }

    let function_state = app.state::<FunctionStore>();
    let _ = upsert_function_status(
        &app,
        &function_state,
        openvpn_function_id,
        "connecting",
        "Command sent, waiting for prompts",
        Some(session_id.clone()),
    );

    let mut username_sent = false;
    let mut password_sent = false;
    let mut seen_connected = false;
    let mut scan_buffer = String::new();

    loop {
        match rx.recv_timeout(Duration::from_millis(350)) {
            Ok(chunk) => {
                let lower_chunk = chunk.to_lowercase();
                scan_buffer.push_str(&lower_chunk);
                if scan_buffer.len() > 5000 {
                    let drain_to = scan_buffer.len() - 3000;
                    scan_buffer.drain(..drain_to);
                }

                if !username_sent
                    && (scan_buffer.contains("username")
                        || scan_buffer.contains("user name")
                        || scan_buffer.contains("enter auth username"))
                {
                    let state = app.state::<SessionStore>();
                    let _ = write_pty_internal(&session_id, &format!("{username}\r"), &state);
                    username_sent = true;
                    let function_state = app.state::<FunctionStore>();
                    let _ = upsert_function_status(
                        &app,
                        &function_state,
                        openvpn_function_id,
                        "authenticating",
                        "Username sent",
                        Some(session_id.clone()),
                    );
                }

                if !password_sent && scan_buffer.contains("password") {
                    let combined_password =
                        match generate_openvpn_password(&password, &otp_secret) {
                            Ok(value) => value,
                            Err(err) => {
                                let function_state = app.state::<FunctionStore>();
                                let _ = upsert_function_status(
                                    &app,
                                    &function_state,
                                    openvpn_function_id,
                                    "error",
                                    &format!("OTP generation failed: {err}"),
                                    Some(session_id.clone()),
                                );
                                continue;
                            }
                        };
                    let state = app.state::<SessionStore>();
                    let _ =
                        write_pty_internal(&session_id, &format!("{combined_password}\r"), &state);
                    password_sent = true;
                    let function_state = app.state::<FunctionStore>();
                    let _ = upsert_function_status(
                        &app,
                        &function_state,
                        openvpn_function_id,
                        "authenticating",
                        "Password sent (OTP + password)",
                        Some(session_id.clone()),
                    );
                }

                if scan_buffer.contains("authentication failed")
                    || scan_buffer.contains("auth failed")
                    || scan_buffer.contains("auth-failure")
                {
                    let function_state = app.state::<FunctionStore>();
                    let _ = upsert_function_status(
                        &app,
                        &function_state,
                        openvpn_function_id,
                        "error",
                        "Authentication failed",
                        Some(session_id.clone()),
                    );
                }

                if !seen_connected
                    && (scan_buffer.contains("initialization sequence completed")
                        || scan_buffer.contains("peer connection initiated"))
                {
                    seen_connected = true;
                    mark_openvpn_connected(&app);
                    let function_state = app.state::<FunctionStore>();
                    let _ = upsert_function_status(
                        &app,
                        &function_state,
                        openvpn_function_id,
                        "running",
                        "VPN tunnel is up",
                        Some(session_id.clone()),
                    );
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let state = app.state::<SessionStore>();
                let session_exists = lock_err_sessions(&state.sessions)
                    .ok()
                    .map(|sessions| sessions.contains_key(&session_id))
                    .unwrap_or(false);
                if !session_exists {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Waits for the outer PowerShell process that runs `Start-Process -Verb RunAs`. That call
/// blocks while the UAC dialog is open; if the user cancels or the launcher hangs, `output()`
/// would block indefinitely and stall the Tauri command — so we enforce a timeout and kill
/// the launcher process.
#[cfg(target_os = "windows")]
fn powershell_elevation_launcher_output(
    command: &str,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", command])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start elevated helper: {e}"))?;

    let started = Instant::now();
    loop {
        match child
            .try_wait()
            .map_err(|e| format!("elevated helper launcher wait error: {e}"))?
        {
            Some(status) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_end(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_end(&mut stderr);
                }
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            None => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "Timed out after {}s waiting for the UAC prompt or elevation launcher. \
                         If you cancelled the Administrator prompt, try Connect again.",
                        timeout.as_secs()
                    ));
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(clippy::too_many_arguments)]
pub fn start_openvpn_admin_worker(
    app: &AppHandle,
    function_store: &FunctionStore,
    admin_store: &OpenVpnAdminStore,
    executable_path: String,
    args: String,
    username: String,
    password: String,
    terminate_open_vpn_when_helper_exits: bool,
    openvpn_helper_script_name: &str,
    openvpn_function_id: &str,
    now_ms: fn() -> u64,
    is_windows_pid_running: fn(u32) -> bool,
    mark_openvpn_connected: fn(&AppHandle),
    handle_openvpn_terminal_state: fn(&AppHandle, &str, &str),
    upsert_function_status: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        &str,
        &str,
        Option<String>,
    ) -> Result<(), String>,
) -> Result<String, String> {
    let existing = get_openvpn_admin_session(admin_store)?;
    if let Some(existing) = existing {
        let _ = fs::write(&existing.stop_path, "stop");
    }

    let base_dir = openvpn_runtime_dir(app)?;
    let session_id = format!("openvpn-admin-{}", now_ms());
    let launch_path = base_dir.join(format!("{session_id}.launch.json"));
    let status_path = base_dir.join(format!("{session_id}.status.json"));
    let log_path = base_dir.join(format!("{session_id}.log.txt"));
    let err_path = base_dir.join(format!("{session_id}.err.txt"));
    let stop_path = base_dir.join(format!("{session_id}.stop"));

    let launch = OpenVpnLaunchConfig {
        executable_path,
        args,
        username,
        password,
        terminate_open_vpn_when_helper_exits,
    };
    let launch_payload = serde_json::to_string_pretty(&launch)
        .map_err(|e| format!("failed to encode launch config: {e}"))?;
    fs::write(&launch_path, launch_payload)
        .map_err(|e| format!("failed to write launch config: {e}"))?;
    fs::write(&log_path, "").map_err(|e| format!("failed to initialize log file: {e}"))?;
    fs::write(&err_path, "").map_err(|e| format!("failed to initialize err file: {e}"))?;
    if stop_path.exists() {
        let _ = fs::remove_file(&stop_path);
    }
    let helper_pid_path = base_dir.join(format!("{session_id}.helper.pid"));
    let _ = fs::remove_file(&helper_pid_path);

    let helper_path = write_openvpn_helper_script(app, openvpn_helper_script_name)?;
    let cmd = format!(
        "$p=Start-Process -Verb RunAs -FilePath 'powershell.exe' -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File','{}','-ConfigPath','{}','-StatusPath','{}','-LogPath','{}','-ErrPath','{}','-StopPath','{}','-HelperPidPath','{}') -PassThru; $p.Id",
        escape_ps_single_quoted(&helper_path.to_string_lossy()),
        escape_ps_single_quoted(&launch_path.to_string_lossy()),
        escape_ps_single_quoted(&status_path.to_string_lossy()),
        escape_ps_single_quoted(&log_path.to_string_lossy()),
        escape_ps_single_quoted(&err_path.to_string_lossy()),
        escape_ps_single_quoted(&stop_path.to_string_lossy()),
        escape_ps_single_quoted(&helper_pid_path.to_string_lossy())
    );

    let cleanup_session_files = || {
        let _ = fs::remove_file(&launch_path);
        let _ = fs::remove_file(&status_path);
        let _ = fs::remove_file(&log_path);
        let _ = fs::remove_file(&err_path);
        let _ = fs::remove_file(&stop_path);
        let _ = fs::remove_file(&helper_pid_path);
    };

    // UAC can keep the launcher blocked for a long time; still cap so the app never hangs forever.
    const ELEVATION_LAUNCH_TIMEOUT: Duration = Duration::from_secs(180);
    let output = match powershell_elevation_launcher_output(&cmd, ELEVATION_LAUNCH_TIMEOUT) {
        Ok(o) => o,
        Err(e) => {
            cleanup_session_files();
            return Err(e);
        }
    };

    if !output.status.success() {
        cleanup_session_files();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit code {}", output.status.code().unwrap_or(-1))
        };
        let hint = if detail.to_lowercase().contains("cancel")
            || detail.to_lowercase().contains("aborted")
            || detail.to_lowercase().contains("denied")
        {
            " (Elevation was cancelled or denied.)"
        } else {
            ""
        };
        return Err(format!("failed to elevate helper: {detail}{hint}"));
    }

    let helper_pid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let pid_ok = helper_pid
        .parse::<u32>()
        .ok()
        .filter(|&p| p > 0)
        .is_some();
    if !pid_ok {
        cleanup_session_files();
        return Err(
            "Elevation launcher returned no helper process id — the Administrator prompt may have been declined."
                .to_string(),
        );
    }
    upsert_function_status(
        app,
        function_store,
        openvpn_function_id,
        "starting",
        &format!("Elevated OpenVPN helper started (pid {helper_pid})"),
        Some(session_id.clone()),
    )?;

    set_openvpn_admin_session(
        admin_store,
        Some(OpenVpnAdminSession {
            id: session_id.clone(),
            stop_path: stop_path.clone(),
        }),
    )?;

    let status_app = app.clone();
    let status_session = session_id.clone();
    let helper_pid_path_watch = helper_pid_path.clone();
    let function_id = openvpn_function_id.to_string();
    thread::spawn(move || {
        let mut last_raw = String::new();
        let mut cached_helper_pid: Option<u32> = None;
        let watcher_started = Instant::now();
        const HELPER_STALL_TIMEOUT: Duration = Duration::from_secs(120);
        loop {
            thread::sleep(Duration::from_millis(500));

            if cached_helper_pid.is_none() {
                if let Ok(s) = fs::read_to_string(&helper_pid_path_watch) {
                    if let Ok(p) = s.trim().parse::<u32>() {
                        if p > 0 {
                            cached_helper_pid = Some(p);
                        }
                    }
                }
            }

            let raw = match fs::read_to_string(&status_path) {
                Ok(raw) => raw,
                Err(_) => {
                    let active = status_app
                        .state::<OpenVpnAdminStore>()
                        .session
                        .lock()
                        .ok()
                        .and_then(|s| s.as_ref().map(|v| v.id.clone()));
                    if active.as_deref() != Some(status_session.as_str()) {
                        break;
                    }
                    if cached_helper_pid.is_none() && watcher_started.elapsed() > HELPER_STALL_TIMEOUT {
                        let _ = fs::remove_file(&helper_pid_path_watch);
                        let msg = "Elevated OpenVPN helper never became ready (timeout waiting for status). Try Connect again.";
                        let _ = upsert_function_status(
                            &status_app,
                            &status_app.state::<FunctionStore>(),
                            &function_id,
                            "error",
                            msg,
                            None,
                        );
                        handle_openvpn_terminal_state(&status_app, "error", msg);
                        let _ =
                            set_openvpn_admin_session(&status_app.state::<OpenVpnAdminStore>(), None);
                        break;
                    }
                    if let Some(pid) = cached_helper_pid {
                        if !is_windows_pid_running(pid) {
                            let _ = fs::remove_file(&helper_pid_path_watch);
                            let _ = upsert_function_status(
                                &status_app,
                                &status_app.state::<FunctionStore>(),
                                &function_id,
                                "stopped",
                                "Elevated helper ended before status file was available",
                                None,
                            );
                            handle_openvpn_terminal_state(
                                &status_app,
                                "stopped",
                                "Elevated helper ended before status file was available",
                            );
                            let _ =
                                set_openvpn_admin_session(&status_app.state::<OpenVpnAdminStore>(), None);
                            break;
                        }
                    }
                    continue;
                }
            };
            if raw != last_raw {
                last_raw = raw.clone();
                if let Ok(parsed) = serde_json::from_str::<HelperStatusFile>(&raw) {
                    let state = parsed.state.to_lowercase();
                    let terminal = state == "stopped" || state == "error";
                    let _ = upsert_function_status(
                        &status_app,
                        &status_app.state::<FunctionStore>(),
                        &function_id,
                        &state,
                        &parsed.message,
                        if terminal {
                            None
                        } else {
                            Some(status_session.clone())
                        },
                    );
                    if terminal {
                        handle_openvpn_terminal_state(&status_app, &state, &parsed.message);
                        let _ = fs::remove_file(&helper_pid_path_watch);
                        let _ =
                            set_openvpn_admin_session(&status_app.state::<OpenVpnAdminStore>(), None);
                        break;
                    }
                    if state == "running" {
                        mark_openvpn_connected(&status_app);
                    }
                }
            }

            if let Some(pid) = cached_helper_pid {
                if !is_windows_pid_running(pid) {
                    let _ = fs::remove_file(&helper_pid_path_watch);
                    let _ = upsert_function_status(
                        &status_app,
                        &status_app.state::<FunctionStore>(),
                        &function_id,
                        "stopped",
                        "Elevated helper ended (window closed or process exited)",
                        None,
                    );
                    handle_openvpn_terminal_state(
                        &status_app,
                        "stopped",
                        "Elevated helper ended (window closed or process exited)",
                    );
                    let _ =
                        set_openvpn_admin_session(&status_app.state::<OpenVpnAdminStore>(), None);
                    break;
                }
            }
        }
    });

    let log_app = app.clone();
    let log_session = session_id.clone();
    let function_id = openvpn_function_id.to_string();
    thread::spawn(move || {
        let mut out_offset: usize = 0;
        let mut err_offset: usize = 0;
        loop {
            thread::sleep(Duration::from_millis(300));
            let active = log_app
                .state::<OpenVpnAdminStore>()
                .session
                .lock()
                .ok()
                .and_then(|s| s.as_ref().map(|v| v.id.clone()));
            if active.as_deref() != Some(log_session.as_str()) {
                break;
            }

            let out_bytes = match fs::read(&log_path) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            if out_bytes.len() > out_offset {
                let chunk = String::from_utf8_lossy(&out_bytes[out_offset..]).to_string();
                out_offset = out_bytes.len();
                emit_openvpn_log(&log_app, &log_session, &function_id, chunk);
            }

            let err_bytes = fs::read(&err_path).unwrap_or_default();
            if err_bytes.len() > err_offset {
                let chunk = String::from_utf8_lossy(&err_bytes[err_offset..]).to_string();
                err_offset = err_bytes.len();
                emit_openvpn_log(&log_app, &log_session, &function_id, format!("[stderr] {chunk}"));
            }
        }
    });

    Ok(session_id)
}

#[cfg(not(target_os = "windows"))]
#[allow(clippy::too_many_arguments)]
pub fn stop_openvpn_pty_session(
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    openvpn_function_id: &str,
    close_pty_internal: fn(&str, &SessionStore) -> Result<Option<PtyExitPayload>, String>,
    emit_pty_exit: fn(&AppHandle, PtyExitPayload),
    update_functions_for_session_exit: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        Option<i32>,
        &str,
    ) -> Result<(), String>,
    upsert_function_status: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        &str,
        &str,
        Option<String>,
    ) -> Result<(), String>,
    lock_err_statuses: fn(
        &std::sync::Mutex<std::collections::HashMap<String, FunctionStatus>>,
    ) -> Result<
        std::sync::MutexGuard<'_, std::collections::HashMap<String, FunctionStatus>>,
        String,
    >,
) -> Result<(), String> {
    let session_id = {
        let statuses = lock_err_statuses(&function_store.statuses)?;
        statuses
            .get(openvpn_function_id)
            .and_then(|status| status.session_id.clone())
    };

    if let Some(session_id) = session_id {
        upsert_function_status(
            app,
            function_store,
            openvpn_function_id,
            "stopping",
            "Stopping OpenVPN PTY session",
            Some(session_id.clone()),
        )?;

        let payload = close_pty_internal(&session_id, session_store)?;
        if let Some(payload) = payload {
            emit_pty_exit(app, payload.clone());
            let _ = update_functions_for_session_exit(
                app,
                function_store,
                &session_id,
                payload.exit_code,
                &payload.reason,
            );
        }
    } else {
        upsert_function_status(
            app,
            function_store,
            openvpn_function_id,
            "idle",
            "No running session",
            None,
        )?;
    }

    Ok(())
}
