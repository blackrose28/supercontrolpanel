#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::{
    collections::HashMap,
    env, fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    sync::mpsc::{self, Receiver, Sender},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use keyring::{Entry, Error as KeyringError};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ssh2::{FileStat, Session as SshSession};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use totp_rs::{Algorithm, Secret, TOTP};

mod models;
mod server_mgmt;
use models::{
    AppConfig, AppConfigForDisk, AppRuntimeConfig, BoundaryAuthKind, BoundaryConfig,
    BoundaryConfigForDisk, BoundaryOidcLoginMode, BoundaryTargetSummary, FunctionStatus,
    HelperStatusFile, OpenVpnConfig, OpenVpnConfigForDisk, OpenVpnLaunchConfig,
    OpenVpnLogPayload, PtyExitPayload, PtyOutputPayload, RemoteBrowseEntry,
    RemoteBrowseNodeInfo, RemoteBrowseSession, RemotePathKind, SshEndpoint,
    StartFunctionResult,
};
use server_mgmt::{load_servers, new_server_id, save_servers, ConnectionMethod, Server, ServerOs};

struct PtySession {
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send>>,
}

struct SessionStore {
    sessions: Mutex<HashMap<String, Arc<PtySession>>>,
    watchers: Mutex<HashMap<String, Vec<Sender<String>>>>,
    next_id: Mutex<u64>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            watchers: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
        }
    }
}

enum RemoteBrowserTransport {
    Sftp(SshSession),
    Cli(CliRemoteBrowserSession),
}

struct CliRemoteBrowserSession {
    io: Mutex<CliRemoteBrowserIo>,
}

struct CliRemoteBrowserIo {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
    next_marker_id: u64,
}

struct RemoteBrowserSessionState {
    server_id: String,
    root_path: String,
    transport: RemoteBrowserTransport,
}

struct RemoteBrowserStore {
    sessions: Mutex<HashMap<String, Arc<RemoteBrowserSessionState>>>,
    next_id: Mutex<u64>,
}

impl Default for RemoteBrowserStore {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
        }
    }
}

const OPENVPN_FUNCTION_ID: &str = "openvpn-connect";
const CADDY_FUNCTION_ID: &str = "caddy-run";
const CONFIG_FILE_NAME: &str = "config.json";
const SERVERS_FILE_NAME: &str = "servers.json";
const SYNC_SETTINGS_FILE_NAME: &str = "sync-local.json";
/// OS credential store: Windows Credential Manager, macOS Keychain, Linux Secret Service.
const KEYRING_SERVICE: &str = "com.example.supercontrolpanel";
const KEYRING_OPENVPN_PASSWORD: &str = "openvpn_password";
const KEYRING_OPENVPN_OTP_SECRET: &str = "openvpn_otp_secret";
const KEYRING_BOUNDARY_PASSWORD: &str = "boundary_password";
const KEYRING_BOUNDARY_OIDC_PASSWORD: &str = "boundary_oidc_password";
const KEYRING_BOUNDARY_OIDC_OTP_SECRET: &str = "boundary_oidc_otp_secret";
const KEYRING_BOUNDARY_APP_TOKEN: &str = "boundary_app_token";
const OPENVPN_HELPER_SCRIPT_NAME: &str = "openvpn-admin-helper.ps1";
const BOUNDARY_AUTH_FUNCTION_ID: &str = "boundary-auth";
const BOUNDARY_OIDC_WINDOW_LABEL: &str = "boundary-oidc-automation";

fn server_tunnel_function_id(server_id: &str) -> String {
    format!("server-tunnel-{server_id}")
}
fn server_ssh_function_id(server_id: &str) -> String {
    format!("server-ssh-{server_id}")
}
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Dev-only traces (visible in the terminal that runs `npm run tauri dev`).
#[cfg(debug_assertions)]
macro_rules! scp_debug {
    ($($arg:tt)*) => {
        eprintln!("[SCP-debug] {}", format!($($arg)*))
    };
}
#[cfg(not(debug_assertions))]
macro_rules! scp_debug {
    ($($arg:tt)*) => {};
}

struct FunctionStore {
    statuses: Mutex<HashMap<String, FunctionStatus>>,
}

struct OpenVpnAdminSession {
    id: String,
    stop_path: PathBuf,
}

struct BoundaryAutomationSession {
    session_id: String,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct OpenVpnAdminStore {
    session: Mutex<Option<OpenVpnAdminSession>>,
}

#[derive(Default)]
struct BoundaryAutomationStore {
    session: Mutex<Option<BoundaryAutomationSession>>,
}

impl Default for FunctionStore {
    fn default() -> Self {
        let mut statuses = HashMap::new();
        for (id, msg) in [
            (OPENVPN_FUNCTION_ID, "Ready"),
            (CADDY_FUNCTION_ID, "Ready"),
            (BOUNDARY_AUTH_FUNCTION_ID, "Not authenticated"),
        ] {
            statuses.insert(
                id.to_string(),
                FunctionStatus {
                    function_id: id.to_string(),
                    state: "idle".to_string(),
                    message: msg.to_string(),
                    boundary_token_source: None,
                    session_id: None,
                    updated_at_ms: now_ms(),
                },
            );
        }
        Self {
            statuses: Mutex::new(statuses),
        }
    }
}

fn lock_err<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> {
    mutex.lock().map_err(|_| "mutex poisoned".to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn default_app_config() -> AppConfig {
    AppConfig {
        app: AppRuntimeConfig {
            default_terminal_cols: 120,
            default_terminal_rows: 36,
        },
        openvpn: OpenVpnConfig {
            command: String::new(),
            executable_path: String::new(),
            ovpn_path: String::new(),
            extra_args: String::new(),
            username: String::new(),
            password: String::new(),
            otp_secret: String::new(),
            terminate_open_vpn_when_helper_exits: false,
        },
        boundary: BoundaryConfig::default(),
    }
}

fn config_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create config dir: {e}"))?;
    Ok(dir.join(CONFIG_FILE_NAME))
}

fn servers_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create config dir: {e}"))?;
    Ok(dir.join(SERVERS_FILE_NAME))
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SyncSettings {
    #[serde(default)]
    repo_root: String,
    #[serde(default)]
    openvpn_executable_path: String,
    #[serde(default)]
    openvpn_ovpn_path: String,
    #[serde(default)]
    boundary_cli_path: String,
    #[serde(default)]
    caddy_folder: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SyncSettingsPatch {
    #[serde(default)]
    repo_root: Option<String>,
    #[serde(default)]
    openvpn_executable_path: Option<String>,
    #[serde(default)]
    openvpn_ovpn_path: Option<String>,
    #[serde(default)]
    boundary_cli_path: Option<String>,
    #[serde(default)]
    caddy_folder: Option<String>,
}

fn sync_settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create config dir: {e}"))?;
    Ok(dir.join(SYNC_SETTINGS_FILE_NAME))
}

fn load_sync_settings_from_disk(app: &AppHandle) -> Result<SyncSettings, String> {
    let path = sync_settings_path(app)?;
    if !path.exists() {
        return Ok(SyncSettings::default());
    }
    let raw =
        fs::read_to_string(&path).map_err(|e| format!("failed to read sync settings: {e}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("invalid sync settings format: {e}"))
}

fn save_sync_settings_to_disk(app: &AppHandle, settings: &SyncSettings) -> Result<(), String> {
    let path = sync_settings_path(app)?;
    let encoded = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("failed to encode sync settings: {e}"))?;
    fs::write(path, encoded).map_err(|e| format!("failed to write sync settings: {e}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigPaths {
    app_config_path: String,
}

fn detect_openvpn_executable_path() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            r"C:\Program Files\OpenVPN\bin\openvpn.exe",
            r"C:\Program Files\OpenVPN Connect\bin\openvpn.exe",
            r"C:\Program Files (x86)\OpenVPN\bin\openvpn.exe",
        ];
        for candidate in candidates {
            if PathBuf::from(candidate).exists() {
                return Some(candidate.to_string());
            }
        }

        if let Ok(output) = Command::new("where").arg("openvpn").output() {
            if output.status.success() {
                let value = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = value.lines().map(str::trim).find(|line| !line.is_empty()) {
                    return Some(line.to_string());
                }
            }
        }
        None
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(output) = Command::new("which").arg("openvpn").output() {
            if output.status.success() {
                let value = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = value.lines().map(str::trim).find(|line| !line.is_empty()) {
                    return Some(line.to_string());
                }
            }
        }

        let candidates = [
            "/usr/sbin/openvpn",
            "/usr/local/sbin/openvpn",
            "/opt/homebrew/sbin/openvpn",
        ];
        for candidate in candidates {
            if PathBuf::from(candidate).exists() {
                return Some(candidate.to_string());
            }
        }
        None
    }
}

fn keyring_entry(account: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, account).map_err(|e| format!("keyring: {e}"))
}

fn keyring_get_optional(account: &str) -> Result<Option<String>, String> {
    let e = keyring_entry(account)?;
    match e.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(err) => Err(format!("keyring read ({account}): {err}")),
    }
}

fn keyring_set_or_delete(account: &str, value: &str) -> Result<(), String> {
    let e = keyring_entry(account)?;
    if value.is_empty() {
        match e.delete_credential() {
            Ok(()) => Ok(()),
            Err(KeyringError::NoEntry) => Ok(()),
            Err(err) => Err(format!("keyring delete ({account}): {err}")),
        }
    } else {
        e.set_password(value)
            .map_err(|err| format!("keyring write ({account}): {err}"))
    }
}

fn boundary_addr_base(addr: &str) -> String {
    addr.trim().trim_end_matches('/').to_string()
}

fn boundary_cli_path(cfg: &AppConfig) -> String {
    if cfg.boundary.cli_path.trim().is_empty() {
        "boundary".to_string()
    } else {
        cfg.boundary.cli_path.trim().to_string()
    }
}

fn boundary_app_token() -> Result<Option<String>, String> {
    keyring_get_optional(KEYRING_BOUNDARY_APP_TOKEN)
}

struct BoundaryTokenSummary {
    source: &'static str,
    message: String,
}

fn apply_boundary_env(cmd: &mut Command, cfg: &AppConfig) -> Result<(), String> {
    let addr = cfg.boundary.addr.trim();
    if !addr.is_empty() {
        cmd.env("BOUNDARY_ADDR", addr);
    }
    if let Some(token) = boundary_app_token()? {
        if !token.trim().is_empty() {
            cmd.env("BOUNDARY_TOKEN", token);
        }
    }
    Ok(())
}

fn apply_boundary_env_builder(cmd: &mut CommandBuilder, cfg: &AppConfig) -> Result<(), String> {
    let addr = cfg.boundary.addr.trim();
    if !addr.is_empty() {
        cmd.env("BOUNDARY_ADDR", addr);
    }
    if let Some(token) = boundary_app_token()? {
        if !token.trim().is_empty() {
            cmd.env("BOUNDARY_TOKEN", token);
        }
    }
    Ok(())
}

fn apply_boundary_token_arg_if_present(cmd: &mut Command) -> Result<(), String> {
    if let Some(token) = boundary_app_token()? {
        if !token.trim().is_empty() {
            cmd.arg("-token");
            cmd.arg("env://BOUNDARY_TOKEN");
        }
    }
    Ok(())
}

fn apply_boundary_token_arg_if_present_builder(cmd: &mut CommandBuilder) -> Result<(), String> {
    if let Some(token) = boundary_app_token()? {
        if !token.trim().is_empty() {
            cmd.arg("-token");
            cmd.arg("env://BOUNDARY_TOKEN");
        }
    }
    Ok(())
}

fn emit_pty_output_line(app: &AppHandle, session_id: &str, line: impl AsRef<str>) {
    let data = format!("{}\r\n", line.as_ref());
    let _ = app.emit(
        "pty-output",
        PtyOutputPayload {
            id: session_id.to_string(),
            data: data.clone(),
        },
    );
    let session_state = app.state::<SessionStore>();
    let _ = broadcast_watchers(&session_state, session_id, &data);
}

fn passive_pty_command() -> CommandBuilder {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = CommandBuilder::new("powershell.exe");
        cmd.arg("-NoLogo");
        cmd.arg("-NoProfile");
        cmd.arg("-Command");
        cmd.arg("while ($true) { Start-Sleep -Seconds 3600 }");
        cmd
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = CommandBuilder::new("sh");
        cmd.arg("-c");
        cmd.arg("while true; do sleep 3600; done");
        cmd
    }
}

fn load_app_config_from_disk(app: &AppHandle) -> Result<AppConfig, String> {
    let path = config_file_path(app)?;
    let mut config = if !path.exists() {
        default_app_config()
    } else {
        let raw = fs::read_to_string(&path).map_err(|e| format!("failed to read config: {e}"))?;
        serde_json::from_str::<AppConfig>(&raw)
            .map_err(|e| format!("invalid config format: {e}"))?
    };

    let mut needs_disk_rewrite = false;
    let mut sync_settings = load_sync_settings_from_disk(app)?;
    let mut needs_sync_settings_rewrite = false;

    if sync_settings.openvpn_executable_path.trim().is_empty()
        && !config.openvpn.executable_path.trim().is_empty()
    {
        sync_settings.openvpn_executable_path = config.openvpn.executable_path.clone();
        needs_sync_settings_rewrite = true;
        needs_disk_rewrite = true;
    }
    if sync_settings.openvpn_ovpn_path.trim().is_empty()
        && !config.openvpn.ovpn_path.trim().is_empty()
    {
        sync_settings.openvpn_ovpn_path = config.openvpn.ovpn_path.clone();
        needs_sync_settings_rewrite = true;
        needs_disk_rewrite = true;
    }
    if sync_settings.boundary_cli_path.trim().is_empty()
        && !config.boundary.cli_path.trim().is_empty()
    {
        sync_settings.boundary_cli_path = config.boundary.cli_path.clone();
        needs_sync_settings_rewrite = true;
        needs_disk_rewrite = true;
    }

    config.openvpn.executable_path = sync_settings.openvpn_executable_path.clone();
    config.openvpn.ovpn_path = sync_settings.openvpn_ovpn_path.clone();
    config.boundary.cli_path = sync_settings.boundary_cli_path.clone();
    if config.openvpn.command.trim().is_empty() && !config.openvpn.executable_path.trim().is_empty()
    {
        config.openvpn.command = config.openvpn.executable_path.clone();
    }

    match keyring_get_optional(KEYRING_OPENVPN_PASSWORD)? {
        Some(p) => config.openvpn.password = p,
        None => {
            if !config.openvpn.password.is_empty() {
                keyring_set_or_delete(KEYRING_OPENVPN_PASSWORD, &config.openvpn.password)?;
                needs_disk_rewrite = true;
            }
        }
    }

    match keyring_get_optional(KEYRING_OPENVPN_OTP_SECRET)? {
        Some(s) => config.openvpn.otp_secret = s,
        None => {
            if !config.openvpn.otp_secret.is_empty() {
                keyring_set_or_delete(KEYRING_OPENVPN_OTP_SECRET, &config.openvpn.otp_secret)?;
                needs_disk_rewrite = true;
            }
        }
    }

    match keyring_get_optional(KEYRING_BOUNDARY_PASSWORD)? {
        Some(p) => config.boundary.password = p,
        None => {
            if !config.boundary.password.is_empty() {
                keyring_set_or_delete(KEYRING_BOUNDARY_PASSWORD, &config.boundary.password)?;
                needs_disk_rewrite = true;
            }
        }
    }

    match keyring_get_optional(KEYRING_BOUNDARY_OIDC_PASSWORD)? {
        Some(p) => config.boundary.oidc_password = p,
        None => {
            if !config.boundary.oidc_password.is_empty() {
                keyring_set_or_delete(
                    KEYRING_BOUNDARY_OIDC_PASSWORD,
                    &config.boundary.oidc_password,
                )?;
                needs_disk_rewrite = true;
            }
        }
    }

    match keyring_get_optional(KEYRING_BOUNDARY_OIDC_OTP_SECRET)? {
        Some(s) => config.boundary.oidc_otp_secret = s,
        None => {
            if !config.boundary.oidc_otp_secret.is_empty() {
                keyring_set_or_delete(
                    KEYRING_BOUNDARY_OIDC_OTP_SECRET,
                    &config.boundary.oidc_otp_secret,
                )?;
                needs_disk_rewrite = true;
            }
        }
    }

    if needs_disk_rewrite {
        save_app_config_to_disk(app, &config)?;
    }
    if needs_sync_settings_rewrite {
        save_sync_settings_to_disk(app, &sync_settings)?;
    }

    Ok(config)
}

fn save_app_config_to_disk(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let path = config_file_path(app)?;
    let disk = AppConfigForDisk::from(config);
    let serialized =
        serde_json::to_string_pretty(&disk).map_err(|e| format!("failed to encode config: {e}"))?;
    fs::write(path, serialized).map_err(|e| format!("failed to write config: {e}"))
}

fn save_local_app_settings(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let mut sync_settings = load_sync_settings_from_disk(app)?;
    sync_settings.openvpn_executable_path = config.openvpn.executable_path.trim().to_string();
    sync_settings.openvpn_ovpn_path = config.openvpn.ovpn_path.trim().to_string();
    sync_settings.boundary_cli_path = config.boundary.cli_path.trim().to_string();
    save_sync_settings_to_disk(app, &sync_settings)
}

fn runtime_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config dir: {e}"))?
        .join("runtime");
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create runtime dir: {e}"))?;
    Ok(dir)
}

fn openvpn_runtime_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = runtime_dir(app)?.join("openvpn");
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create openvpn runtime dir: {e}"))?;
    Ok(dir)
}

fn write_openvpn_helper_script(app: &AppHandle) -> Result<PathBuf, String> {
    let path = runtime_dir(app)?.join(OPENVPN_HELPER_SCRIPT_NAME);
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

fn escape_ps_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

fn set_openvpn_admin_session(
    admin_store: &OpenVpnAdminStore,
    session: Option<OpenVpnAdminSession>,
) -> Result<(), String> {
    let mut slot = lock_err(&admin_store.session)?;
    *slot = session;
    Ok(())
}

fn get_openvpn_admin_session(
    admin_store: &OpenVpnAdminStore,
) -> Result<Option<OpenVpnAdminSession>, String> {
    let slot = lock_err(&admin_store.session)?;
    Ok(slot.as_ref().map(|s| OpenVpnAdminSession {
        id: s.id.clone(),
        stop_path: s.stop_path.clone(),
    }))
}

#[cfg(target_os = "windows")]
fn is_windows_pid_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code);
        let _ = CloseHandle(h);
        if ok == 0 {
            return false;
        }
        code == STILL_ACTIVE
    }
}

fn emit_openvpn_log(app: &AppHandle, session_id: &str, data: String) {
    let _ = app.emit(
        "openvpn-log",
        OpenVpnLogPayload {
            function_id: OPENVPN_FUNCTION_ID.to_string(),
            session_id: session_id.to_string(),
            data,
        },
    );
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn quote_posix_arg(raw: &str) -> String {
    if raw.is_empty() {
        "''".to_string()
    } else if raw
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_./:".contains(&b))
    {
        raw.to_string()
    } else {
        format!("'{}'", raw.replace('\'', "'\"'\"'"))
    }
}

fn quote_windows_arg(raw: &str) -> String {
    if raw.contains(' ') || raw.contains('\t') || raw.contains('"') {
        format!("\"{}\"", raw.replace('"', "\\\""))
    } else {
        raw.to_string()
    }
}

fn normalized_openvpn_config(config: &OpenVpnConfig) -> OpenVpnConfig {
    let mut cfg = config.clone();
    if cfg.executable_path.trim().is_empty() && !cfg.command.trim().is_empty() {
        cfg.executable_path = cfg.command.trim().to_string();
    }
    cfg
}

fn build_openvpn_args_windows(config: &OpenVpnConfig) -> Result<String, String> {
    if config.ovpn_path.trim().is_empty() {
        return Err("ovpn file path cannot be empty".to_string());
    }
    let mut args = format!("--config {}", quote_windows_arg(config.ovpn_path.trim()));
    if !config.extra_args.trim().is_empty() {
        args.push(' ');
        args.push_str(config.extra_args.trim());
    }
    Ok(args)
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn build_openvpn_command_unix(config: &OpenVpnConfig) -> Result<String, String> {
    if config.executable_path.trim().is_empty() {
        return Err("openvpn executable path cannot be empty".to_string());
    }
    if config.ovpn_path.trim().is_empty() {
        return Err("ovpn file path cannot be empty".to_string());
    }
    let mut command = format!(
        "sudo {} --config {}",
        quote_posix_arg(config.executable_path.trim()),
        quote_posix_arg(config.ovpn_path.trim())
    );
    if !config.extra_args.trim().is_empty() {
        command.push(' ');
        command.push_str(config.extra_args.trim());
    }
    Ok(command)
}

fn next_session_id(store: &SessionStore) -> Result<String, String> {
    let mut next = lock_err(&store.next_id)?;
    let id = *next;
    *next += 1;
    Ok(format!("pty-{id}"))
}

fn remove_session(store: &SessionStore, id: &str) -> Result<Option<Arc<PtySession>>, String> {
    {
        let mut watchers = lock_err(&store.watchers)?;
        watchers.remove(id);
    }
    let mut sessions = lock_err(&store.sessions)?;
    Ok(sessions.remove(id))
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn add_watcher(store: &SessionStore, id: &str, sender: Sender<String>) -> Result<(), String> {
    let mut watchers = lock_err(&store.watchers)?;
    watchers.entry(id.to_string()).or_default().push(sender);
    Ok(())
}

fn broadcast_watchers(store: &SessionStore, id: &str, data: &str) -> Result<(), String> {
    let mut watchers = lock_err(&store.watchers)?;
    if let Some(list) = watchers.get_mut(id) {
        list.retain(|sender| sender.send(data.to_string()).is_ok());
    }
    Ok(())
}

fn emit_pty_exit(app: &AppHandle, payload: PtyExitPayload) {
    let _ = app.emit("pty-exit", payload);
}

fn upsert_function_status(
    app: &AppHandle,
    function_store: &FunctionStore,
    function_id: &str,
    state: &str,
    message: &str,
    session_id: Option<String>,
) -> Result<(), String> {
    let status = FunctionStatus {
        function_id: function_id.to_string(),
        state: state.to_string(),
        message: message.to_string(),
        boundary_token_source: None,
        session_id,
        updated_at_ms: now_ms(),
    };

    {
        let mut statuses = lock_err(&function_store.statuses)?;
        statuses.insert(function_id.to_string(), status.clone());
    }

    let _ = app.emit("function-status", status);
    Ok(())
}

fn upsert_boundary_status(
    app: &AppHandle,
    function_store: &FunctionStore,
    state: &str,
    message: &str,
    boundary_token_source: Option<&str>,
    session_id: Option<String>,
) -> Result<(), String> {
    let status = FunctionStatus {
        function_id: BOUNDARY_AUTH_FUNCTION_ID.to_string(),
        state: state.to_string(),
        message: message.to_string(),
        boundary_token_source: boundary_token_source.map(|s| s.to_string()),
        session_id,
        updated_at_ms: now_ms(),
    };

    {
        let mut statuses = lock_err(&function_store.statuses)?;
        statuses.insert(BOUNDARY_AUTH_FUNCTION_ID.to_string(), status.clone());
    }

    let _ = app.emit("function-status", status);
    Ok(())
}

fn update_functions_for_session_exit(
    app: &AppHandle,
    function_store: &FunctionStore,
    session_id: &str,
    exit_code: Option<i32>,
    reason: &str,
) -> Result<(), String> {
    let mut changed = Vec::<FunctionStatus>::new();
    {
        let mut statuses = lock_err(&function_store.statuses)?;
        for status in statuses.values_mut() {
            if status.session_id.as_deref() != Some(session_id) {
                continue;
            }
            if status.function_id == BOUNDARY_AUTH_FUNCTION_ID {
                status.session_id = None;
                status.updated_at_ms = now_ms();
                let (state, message, source) = match boundary_token_summary(app) {
                    Ok(summary) => (
                        "authenticated".to_string(),
                        summary.message,
                        Some(summary.source.to_string()),
                    ),
                    Err(_) => {
                        if exit_code == Some(0) {
                            (
                                "authenticated".to_string(),
                                "Login completed, but the saved Boundary token could not be verified yet. Refresh Status or retry login."
                                    .to_string(),
                                None,
                            )
                        } else {
                            (
                                "unauthenticated".to_string(),
                                format!(
                                    "Not logged in or login interrupted (exit {}). {}",
                                    exit_code
                                        .map(|v| v.to_string())
                                        .unwrap_or_else(|| "?".to_string()),
                                    if reason.len() > 80 {
                                        format!("{}…", &reason[..80])
                                    } else {
                                        reason.to_string()
                                    }
                                ),
                                None,
                            )
                        }
                    }
                };
                status.state = state;
                status.message = message;
                status.boundary_token_source = source;
                changed.push(status.clone());
                continue;
            }
            status.state = if exit_code == Some(0) {
                "stopped".to_string()
            } else {
                "error".to_string()
            };
            status.message = format!(
                "PTY exited (code={}, reason={reason})",
                exit_code
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            );
            status.session_id = None;
            status.updated_at_ms = now_ms();
            changed.push(status.clone());
        }
    }

    for status in changed {
        let _ = app.emit("function-status", status);
    }
    Ok(())
}

/// Probes the currently active Boundary token via `boundary config get-token`.
/// This supports both the CLI-managed keyring token and the app-managed token injected through `BOUNDARY_TOKEN`.
fn boundary_token_summary(app: &AppHandle) -> Result<BoundaryTokenSummary, String> {
    let cfg = load_app_config_from_disk(app)?;
    let exe = boundary_cli_path(&cfg);
    let _addr = cfg.boundary.addr.trim();
    let using_app_token = boundary_app_token()?
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false);
    scp_debug!(
        "boundary_token_summary: exe={exe:?} BOUNDARY_ADDR={}",
        if _addr.is_empty() { "unset" } else { _addr }
    );
    if exe != "boundary" && !PathBuf::from(&exe).exists() {
        return Err(format!("Boundary CLI not found: {exe}"));
    }
    let mut cmd = Command::new(&exe);
    cmd.arg("config");
    cmd.arg("get-token");
    apply_boundary_env(&mut cmd, &cfg)?;
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd
        .output()
        .map_err(|e| format!("failed to run `{exe} config get-token`: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let _code = out.status.code().unwrap_or(-1);
        scp_debug!(
            "boundary config get-token failed: exit_code={_code} stderr_len={} stderr={:?}",
            out.stderr.len(),
            if stderr.len() > 400 {
                format!("{}…", &stderr[..400])
            } else {
                stderr.clone()
            }
        );
        if stderr.is_empty() {
            return Err("no token".into());
        }
        return Err(stderr);
    }
    let token_trimmed = String::from_utf8_lossy(&out.stdout).trim().to_string();
    scp_debug!(
        "boundary config get-token ok: token_len={}",
        token_trimmed.len()
    );
    if token_trimmed.is_empty() {
        return Err("no token".into());
    }
    Ok(BoundaryTokenSummary {
        source: if using_app_token { "app" } else { "cli" },
        message: if using_app_token {
            "Authenticated via hidden OIDC window. Token is stored by SuperControlPanel and injected into Boundary commands."
                .to_string()
        } else {
            "Authenticated via Boundary CLI. Token is stored in the CLI keyring.".to_string()
        },
    })
}

fn open_pty_internal(
    cols: u16,
    rows: u16,
    app: &AppHandle,
    store: &SessionStore,
) -> Result<String, String> {
    open_pty_with_command_builder(cols, rows, default_shell_command(), app, store)
}

fn open_pty_with_command_builder(
    cols: u16,
    rows: u16,
    cmd: CommandBuilder,
    app: &AppHandle,
    store: &SessionStore,
) -> Result<String, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("failed to open pty: {e}"))?;

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("failed to spawn process in pty: {e}"))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("failed to clone pty reader: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("failed to get pty writer: {e}"))?;

    let session_id = next_session_id(store)?;
    let session = Arc::new(PtySession {
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        child: Mutex::new(child),
    });

    {
        let mut sessions = lock_err(&store.sessions)?;
        sessions.insert(session_id.clone(), session);
    }

    let output_session_id = session_id.clone();
    let output_app = app.clone();
    thread::spawn(move || {
        let mut buf = [0_u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = output_app.emit(
                        "pty-output",
                        PtyOutputPayload {
                            id: output_session_id.clone(),
                            data: data.clone(),
                        },
                    );
                    let session_state = output_app.state::<SessionStore>();
                    let _ = broadcast_watchers(&session_state, &output_session_id, &data);
                }
                Err(_) => break,
            }
        }
    });

    let monitor_session_id = session_id.clone();
    let monitor_app = app.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(100));

        let session = {
            let state = monitor_app.state::<SessionStore>();
            let lookup = match lock_err(&state.sessions) {
                Ok(sessions) => sessions.get(&monitor_session_id).cloned(),
                Err(_) => None,
            };
            lookup
        };

        let Some(session) = session else {
            break;
        };

        let exit_payload = {
            let mut child = match lock_err(&session.child) {
                Ok(child) => child,
                Err(_) => break,
            };
            match child.try_wait() {
                Ok(Some(status)) => Some(PtyExitPayload {
                    id: monitor_session_id.clone(),
                    exit_code: i32::try_from(status.exit_code()).ok(),
                    reason: status.to_string(),
                }),
                Ok(None) => None,
                Err(err) => Some(PtyExitPayload {
                    id: monitor_session_id.clone(),
                    exit_code: None,
                    reason: format!("wait_error: {err}"),
                }),
            }
        };

        if let Some(payload) = exit_payload {
            let removed = {
                let state = monitor_app.state::<SessionStore>();
                remove_session(&state, &monitor_session_id)
                    .ok()
                    .flatten()
                    .is_some()
            };

            if removed {
                emit_pty_exit(&monitor_app, payload.clone());
                let function_state = monitor_app.state::<FunctionStore>();
                let _ = update_functions_for_session_exit(
                    &monitor_app,
                    &function_state,
                    &monitor_session_id,
                    payload.exit_code,
                    &payload.reason,
                );
            }
            break;
        }
    });

    Ok(session_id)
}

fn write_pty_internal(id: &str, data: &str, store: &SessionStore) -> Result<(), String> {
    let session = {
        let sessions = lock_err(&store.sessions)?;
        sessions
            .get(id)
            .cloned()
            .ok_or_else(|| format!("session not found: {id}"))?
    };

    let mut writer = lock_err(&session.writer)?;
    writer
        .write_all(data.as_bytes())
        .map_err(|e| format!("failed to write pty input: {e}"))?;
    writer
        .flush()
        .map_err(|e| format!("failed to flush pty input: {e}"))?;

    Ok(())
}

fn close_pty_internal(id: &str, store: &SessionStore) -> Result<Option<PtyExitPayload>, String> {
    let session = remove_session(store, id)?;
    if let Some(session) = session {
        let mut child = lock_err(&session.child)?;
        let _ = child.kill();
        let exit_status = child
            .wait()
            .map_err(|e| format!("failed to wait child: {e}"))?;
        return Ok(Some(PtyExitPayload {
            id: id.to_string(),
            exit_code: i32::try_from(exit_status.exit_code()).ok(),
            reason: "closed_by_command".to_string(),
        }));
    }
    Ok(None)
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn adapt_command_for_shell(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    #[cfg(target_os = "windows")]
    {
        // PowerShell requires '&' when invoking a quoted executable path.
        if trimmed.starts_with('&') {
            trimmed.to_string()
        } else if trimmed.starts_with('"') || trimmed.starts_with('\'') {
            format!("& {trimmed}")
        } else {
            trimmed.to_string()
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        trimmed.to_string()
    }
}

/// Cached clock offset (seconds) = remote_epoch − local_epoch.
/// Positive means local clock is behind; negative means ahead.
static NTP_OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();

/// Fetch clock offset by comparing local time with a remote HTTP Date header.
fn fetch_time_offset() -> i64 {
    *NTP_OFFSET.get_or_init(|| {
        let offset = (|| -> Result<i64, String> {
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|e| format!("{e}"))?;
            let resp = client
                .head("https://www.google.com")
                .send()
                .map_err(|e| format!("{e}"))?;
            let date_str = resp
                .headers()
                .get("date")
                .and_then(|v| v.to_str().ok())
                .ok_or("no Date header")?;
            // Parse HTTP date: "Wed, 26 Mar 2026 16:45:00 GMT"
            let remote = httpdate::parse_http_date(date_str).map_err(|e| format!("{e}"))?;
            let remote_epoch = remote
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| format!("{e}"))?
                .as_secs() as i64;
            let local_epoch = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| format!("{e}"))?
                .as_secs() as i64;
            Ok(remote_epoch - local_epoch)
        })();
        match offset {
            Ok(d) => d,
            Err(_) => 0,
        }
    })
}

fn generate_totp_code(otp_secret: &str) -> Result<String, String> {
    let secret = otp_secret.trim();
    if secret.is_empty() {
        return Err("OTP secret is empty".to_string());
    }
    let secret_bytes = Secret::Encoded(secret.to_string())
        .to_bytes()
        .map_err(|e| format!("invalid OTP secret: {e}"))?;
    let local_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("system time error: {e}"))?
        .as_secs();
    let offset = fetch_time_offset();
    let corrected = (local_epoch as i64 + offset) as u64;
    let totp = TOTP::new(Algorithm::SHA1, 6, 1, 30, secret_bytes)
        .map_err(|e| format!("failed to initialize OTP generator: {e}"))?;
    Ok(totp.generate(corrected))
}

fn generate_openvpn_password(password: &str, otp_secret: &str) -> Result<String, String> {
    let secret = otp_secret.trim();
    if secret.is_empty() {
        return Ok(password.to_string());
    }
    let otp = generate_totp_code(secret)?;
    Ok(format!("{otp}{password}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct BoundaryApiAuthenticateResponse {
    #[serde(default)]
    attributes: BoundaryApiAuthenticateAttributes,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
struct BoundaryApiAuthenticateAttributes {
    #[serde(default)]
    auth_url: String,
    #[serde(default)]
    token_id: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    expiration_time: String,
}

fn boundary_http_client() -> Result<Client, String> {
    Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::limited(20))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}

fn boundary_oidc_init_script(cfg: &BoundaryConfig) -> String {
    let username =
        serde_json::to_string(cfg.oidc_username.trim()).unwrap_or_else(|_| "\"\"".into());
    let password = serde_json::to_string(&cfg.oidc_password).unwrap_or_else(|_| "\"\"".into());
    let otp = match generate_totp_code(&cfg.oidc_otp_secret) {
        Ok(code) => serde_json::to_string(&code).unwrap_or_else(|_| "\"\"".into()),
        Err(_) => "\"\"".into(),
    };
    format!(
        r#"
(() => {{
  const cfg = {{ username: {username}, password: {password}, otp: {otp} }};
  const visible = (el) => !!el && !el.disabled && el.type !== "hidden" && el.offsetParent !== null;
  const findFirst = (selectors) => {{
    for (const selector of selectors) {{
      const el = document.querySelector(selector);
      if (visible(el)) return el;
    }}
    return null;
  }};
  const setValue = (el, value) => {{
    if (!el || !value) return false;
    el.focus();
    el.value = value;
    el.dispatchEvent(new Event("input", {{ bubbles: true }}));
    el.dispatchEvent(new Event("change", {{ bubbles: true }}));
    return true;
  }};
  const submit = (el) => {{
    if (!el) return false;
    const form = el.closest("form");
    if (form) {{
      if (typeof form.requestSubmit === "function") form.requestSubmit();
      else form.submit();
      return true;
    }}
    const btn = findFirst([
      "button[type=submit]",
      "input[type=submit]",
      "button[id*=login i]",
      "button[name*=login i]",
      "button[id*=verify i]",
      "button[name*=verify i]",
      "button"
    ]);
    if (btn) {{
      btn.click();
      return true;
    }}
    return false;
  }};
  const tick = () => {{
    const pwd = findFirst(["input[type=password]", "input[name*=pass i]", "input[id*=pass i]"]);
    const otpField = findFirst([
      "input[autocomplete='one-time-code']",
      "input[name*=otp i]",
      "input[id*=otp i]",
      "input[name*=totp i]",
      "input[id*=totp i]",
      "input[name*=code i]",
      "input[id*=code i]"
    ]);
    if (otpField && cfg.otp && !otpField.value) {{
      setValue(otpField, cfg.otp);
      submit(otpField);
      return;
    }}
    const user = findFirst([
      "input[autocomplete=username]",
      "input[type=email]",
      "input[name*=user i]",
      "input[id*=user i]",
      "input[name*=email i]",
      "input[id*=email i]",
      "input[type=text]"
    ]);
    if (user && cfg.username && !user.value) setValue(user, cfg.username);
    if (pwd && cfg.password && !pwd.value) {{
      setValue(pwd, cfg.password);
      submit(pwd);
      return;
    }}
    if (user && cfg.username && !pwd) {{
      const next = findFirst([
        "button[type=submit]",
        "input[type=submit]",
        "button[id*=next i]",
        "button[name*=next i]"
      ]);
      if (next) next.click();
    }}
  }};
  tick();
  setInterval(tick, 900);
}})();
"#
    )
}

#[cfg(target_os = "windows")]
fn start_openvpn_admin_worker(
    app: &AppHandle,
    function_store: &FunctionStore,
    admin_store: &OpenVpnAdminStore,
    executable_path: String,
    args: String,
    username: String,
    password: String,
    terminate_open_vpn_when_helper_exits: bool,
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

    let helper_path = write_openvpn_helper_script(app)?;
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

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &cmd])
        .output()
        .map_err(|e| format!("failed to start elevated helper: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        scp_debug!("start_openvpn_admin_worker: elevate failed stderr={stderr:?}");
        return Err(format!("failed to elevate helper: {stderr}"));
    }

    let helper_pid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    scp_debug!("start_openvpn_admin_worker: elevated helper pid={helper_pid}");
    upsert_function_status(
        app,
        function_store,
        OPENVPN_FUNCTION_ID,
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
    thread::spawn(move || {
        let mut last_raw = String::new();
        let mut cached_helper_pid: Option<u32> = None;
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
                    if let Some(pid) = cached_helper_pid {
                        if !is_windows_pid_running(pid) {
                            let _ = fs::remove_file(&helper_pid_path_watch);
                            let _ = upsert_function_status(
                                &status_app,
                                &status_app.state::<FunctionStore>(),
                                OPENVPN_FUNCTION_ID,
                                "stopped",
                                "Elevated helper ended before status file was available",
                                None,
                            );
                            let _ = set_openvpn_admin_session(
                                &status_app.state::<OpenVpnAdminStore>(),
                                None,
                            );
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
                        OPENVPN_FUNCTION_ID,
                        &state,
                        &parsed.message,
                        if terminal {
                            None
                        } else {
                            Some(status_session.clone())
                        },
                    );
                    if terminal {
                        let _ = fs::remove_file(&helper_pid_path_watch);
                        let _ = set_openvpn_admin_session(
                            &status_app.state::<OpenVpnAdminStore>(),
                            None,
                        );
                        break;
                    }
                }
            }

            if let Some(pid) = cached_helper_pid {
                if !is_windows_pid_running(pid) {
                    let _ = fs::remove_file(&helper_pid_path_watch);
                    let _ = upsert_function_status(
                        &status_app,
                        &status_app.state::<FunctionStore>(),
                        OPENVPN_FUNCTION_ID,
                        "stopped",
                        "Elevated helper ended (window closed or process exited)",
                        None,
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
                emit_openvpn_log(&log_app, &log_session, chunk);
            }

            let err_bytes = fs::read(&err_path).unwrap_or_default();
            if err_bytes.len() > err_offset {
                let chunk = String::from_utf8_lossy(&err_bytes[err_offset..]).to_string();
                err_offset = err_bytes.len();
                emit_openvpn_log(&log_app, &log_session, format!("[stderr] {chunk}"));
            }
        }
    });

    Ok(session_id)
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn run_openvpn_automation(
    app: AppHandle,
    session_id: String,
    command: String,
    username: String,
    password: String,
    otp_secret: String,
) {
    let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();
    let session_state = app.state::<SessionStore>();
    if add_watcher(&session_state, &session_id, tx).is_err() {
        let function_state = app.state::<FunctionStore>();
        let _ = upsert_function_status(
            &app,
            &function_state,
            OPENVPN_FUNCTION_ID,
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
            OPENVPN_FUNCTION_ID,
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
        OPENVPN_FUNCTION_ID,
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
                        OPENVPN_FUNCTION_ID,
                        "authenticating",
                        "Username sent",
                        Some(session_id.clone()),
                    );
                }

                if !password_sent && scan_buffer.contains("password") {
                    let combined_password = match generate_openvpn_password(&password, &otp_secret)
                    {
                        Ok(value) => value,
                        Err(err) => {
                            let function_state = app.state::<FunctionStore>();
                            let _ = upsert_function_status(
                                &app,
                                &function_state,
                                OPENVPN_FUNCTION_ID,
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
                        OPENVPN_FUNCTION_ID,
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
                        OPENVPN_FUNCTION_ID,
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
                    let function_state = app.state::<FunctionStore>();
                    let _ = upsert_function_status(
                        &app,
                        &function_state,
                        OPENVPN_FUNCTION_ID,
                        "running",
                        "VPN tunnel is up",
                        Some(session_id.clone()),
                    );
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let state = app.state::<SessionStore>();
                let session_exists = lock_err(&state.sessions)
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

fn default_shell_command() -> CommandBuilder {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = CommandBuilder::new("powershell.exe");
        cmd.arg("-NoLogo");
        cmd.arg("-NoProfile");
        cmd
    }

    #[cfg(not(target_os = "windows"))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let mut cmd = CommandBuilder::new(shell);
        cmd.arg("-i");
        cmd
    }
}

#[tauri::command]
fn open_pty(
    cols: u16,
    rows: u16,
    app: AppHandle,
    store: State<'_, SessionStore>,
) -> Result<String, String> {
    open_pty_internal(cols, rows, &app, &store)
}

#[tauri::command]
fn write_pty(id: String, data: String, store: State<'_, SessionStore>) -> Result<(), String> {
    write_pty_internal(&id, &data, &store)
}

#[tauri::command]
fn resize_pty(
    id: String,
    cols: u16,
    rows: u16,
    store: State<'_, SessionStore>,
) -> Result<(), String> {
    let session = {
        let sessions = lock_err(&store.sessions)?;
        sessions
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("session not found: {id}"))?
    };

    let master = lock_err(&session.master)?;
    master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("failed to resize pty: {e}"))?;

    Ok(())
}

#[tauri::command]
fn close_pty(
    id: String,
    app: AppHandle,
    store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<(), String> {
    let payload = close_pty_internal(&id, &store)?;
    if let Some(payload) = payload {
        emit_pty_exit(&app, payload.clone());
        let _ = update_functions_for_session_exit(
            &app,
            &function_store,
            &id,
            payload.exit_code,
            &payload.reason,
        );
    }
    Ok(())
}

#[tauri::command]
fn list_function_statuses(
    app: AppHandle,
    function_store: State<'_, FunctionStore>,
) -> Result<Vec<FunctionStatus>, String> {
    let mut values = lock_err(&function_store.statuses)?
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for s in &mut values {
        if s.function_id != BOUNDARY_AUTH_FUNCTION_ID {
            continue;
        }
        if s.session_id.is_some() {
            s.state = "authenticating".to_string();
            s.message =
                "Logging in — finish in the Boundary tab (browser or terminal).".to_string();
            s.boundary_token_source = None;
            continue;
        }
        match boundary_token_summary(&app) {
            Ok(summary) => {
                s.state = "authenticated".to_string();
                s.message = summary.message;
                s.boundary_token_source = Some(summary.source.to_string());
            }
            Err(e) => {
                s.state = "unauthenticated".to_string();
                s.boundary_token_source = None;
                let addr_hint = load_app_config_from_disk(&app)
                    .map(|c| {
                        if c.boundary.addr.trim().is_empty() {
                            " Set Controller URL in Config so the CLI targets the right keyring entry."
                        } else {
                            ""
                        }
                    })
                    .unwrap_or("");
                let err_short = if e.len() > 120 {
                    format!("{}…", &e[..117])
                } else {
                    e
                };
                s.message = format!(
                    "No active Boundary token — open Boundary tab and log in. [{err_short}]{addr_hint}"
                );
            }
        }
    }
    values.sort_by(|a, b| a.function_id.cmp(&b.function_id));
    Ok(values)
}

#[tauri::command]
fn get_app_config(app: AppHandle) -> Result<AppConfig, String> {
    let config = load_app_config_from_disk(&app)?;
    scp_debug!(
        "get_app_config: exe_len={} ovpn_len={} user_len={} password_in_memory={} otp_configured={}",
        config.openvpn.executable_path.len(),
        config.openvpn.ovpn_path.len(),
        config.openvpn.username.len(),
        !config.openvpn.password.is_empty(),
        !config.openvpn.otp_secret.is_empty()
    );
    save_app_config_to_disk(&app, &config)?;
    Ok(config)
}

#[tauri::command]
fn get_config_paths(app: AppHandle) -> Result<ConfigPaths, String> {
    let path = config_file_path(&app)?;
    Ok(ConfigPaths {
        app_config_path: path.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
fn get_sync_settings(app: AppHandle) -> Result<SyncSettings, String> {
    load_sync_settings_from_disk(&app)
}

#[tauri::command]
fn save_sync_settings(app: AppHandle, settings: SyncSettingsPatch) -> Result<(), String> {
    let mut merged = load_sync_settings_from_disk(&app)?;
    if let Some(repo_root) = settings.repo_root {
        merged.repo_root = repo_root;
    }
    if let Some(openvpn_executable_path) = settings.openvpn_executable_path {
        if !openvpn_executable_path.is_empty() {
            merged.openvpn_executable_path = openvpn_executable_path;
        }
    }
    if let Some(openvpn_ovpn_path) = settings.openvpn_ovpn_path {
        if !openvpn_ovpn_path.is_empty() {
            merged.openvpn_ovpn_path = openvpn_ovpn_path;
        }
    }
    if let Some(boundary_cli_path) = settings.boundary_cli_path {
        if !boundary_cli_path.is_empty() {
            merged.boundary_cli_path = boundary_cli_path;
        }
    }
    if let Some(caddy_folder) = settings.caddy_folder {
        merged.caddy_folder = caddy_folder;
    }
    save_sync_settings_to_disk(&app, &merged)
}

#[tauri::command]
fn sync_config_pull(app: AppHandle, repo_root: String) -> Result<(), String> {
    let repo_root = repo_root.trim();
    if repo_root.is_empty() {
        return Err("repoRoot cannot be empty".into());
    }
    let app_path = config_file_path(&app)?;
    let servers_path = servers_file_path(&app)?;
    let repo_dir = PathBuf::from(repo_root).join("app-config");
    let repo_config_path = repo_dir.join(CONFIG_FILE_NAME);
    if !repo_config_path.exists() {
        return Err(format!(
            "Repo config not found at {}. Commit app-config/config.json first.",
            repo_config_path.to_string_lossy()
        ));
    }
    if let Some(parent) = app_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create app config dir: {e}"))?;
    }
    fs::copy(&repo_config_path, &app_path)
        .map_err(|e| format!("failed to copy from repo to app config: {e}"))?;

    let repo_servers_path = repo_dir.join(SERVERS_FILE_NAME);
    if repo_servers_path.exists() {
        if let Some(parent) = servers_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create servers config dir: {e}"))?;
        }
        fs::copy(&repo_servers_path, &servers_path)
            .map_err(|e| format!("failed to copy from repo to servers.json: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
fn sync_config_push(app: AppHandle, repo_root: String) -> Result<(), String> {
    let repo_root = repo_root.trim();
    if repo_root.is_empty() {
        return Err("repoRoot cannot be empty".into());
    }
    let app_path = config_file_path(&app)?;
    if !app_path.exists() {
        return Err(format!(
            "App config not found at {}. Save config once in the UI first.",
            app_path.to_string_lossy()
        ));
    }
    let servers_path = servers_file_path(&app)?;

    let repo_dir = PathBuf::from(repo_root).join("app-config");
    let repo_config_path = repo_dir.join(CONFIG_FILE_NAME);
    let repo_servers_path = repo_dir.join(SERVERS_FILE_NAME);

    if let Some(parent) = repo_config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create repo app-config directory: {e}"))?;
    }
    fs::copy(&app_path, &repo_config_path)
        .map_err(|e| format!("failed to copy from app config to repo: {e}"))?;

    if servers_path.exists() {
        if let Some(parent) = repo_servers_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create repo app-config directory: {e}"))?;
        }
        fs::copy(&servers_path, &repo_servers_path)
            .map_err(|e| format!("failed to copy from servers.json to repo: {e}"))?;
    }
    Ok(())
}

fn config_git_computer_name() -> String {
    if cfg!(target_os = "windows") {
        if let Ok(v) = env::var("COMPUTERNAME") {
            let t = v.trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    if let Ok(v) = env::var("HOSTNAME") {
        let t = v.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    "unknown-computer".to_string()
}

fn run_git(repo_root: &str, args: &[&str]) -> Result<String, String> {
    let root = repo_root.trim();
    if root.is_empty() {
        return Err("repoRoot cannot be empty".into());
    }
    let git_dir = PathBuf::from(root).join(".git");
    if !git_dir.exists() {
        return Err(format!(
            "Not a git repo: {} (missing .git)",
            PathBuf::from(root).to_string_lossy()
        ));
    }

    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        // Avoid hanging on credential prompts in a GUI app.
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("failed to run git {:?}: {e}", args))?;

    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&out.stdout));
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    let combined = combined.trim().to_string();

    if out.status.success() {
        Ok(if combined.is_empty() {
            "OK".into()
        } else {
            combined
        })
    } else {
        let code = out
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unknown".into());
        Err(if combined.is_empty() {
            format!("git {:?} failed (exit code {code})", args)
        } else {
            format!("git {:?} failed (exit code {code}):\n{combined}", args)
        })
    }
}

#[tauri::command]
fn config_git_pull(repo_root: String) -> Result<String, String> {
    run_git(repo_root.trim(), &["pull"])
}

#[tauri::command]
fn config_git_push(repo_root: String) -> Result<String, String> {
    let repo_root = repo_root.trim().to_string();
    if repo_root.is_empty() {
        return Err("repoRoot cannot be empty".into());
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push(run_git(&repo_root, &["add", "."])?);

    let status = run_git(&repo_root, &["status", "--porcelain"])?;
    if status.trim().is_empty() || status.trim() == "OK" {
        return Ok("No changes to commit.".into());
    }

    let computer = config_git_computer_name();
    let msg = format!("Change config from {computer}");
    lines.push(run_git(&repo_root, &["commit", "-m", &msg])?);
    lines.push(run_git(&repo_root, &["push"])?);
    Ok(lines.join("\n"))
}

#[tauri::command]
fn save_app_config(app: AppHandle, mut config: AppConfig) -> Result<AppConfig, String> {
    let _incoming_pw_len = config.openvpn.password.len();
    let _incoming_otp_len = config.openvpn.otp_secret.len();
    // Never wipe keyring when the UI sends empty secrets (WebViews often omit password field values).
    if !config.openvpn.password.is_empty() {
        scp_debug!("save_app_config: writing password to keyring (len={_incoming_pw_len})");
        keyring_set_or_delete(KEYRING_OPENVPN_PASSWORD, &config.openvpn.password)?;
    } else if let Some(p) = keyring_get_optional(KEYRING_OPENVPN_PASSWORD)? {
        scp_debug!(
            "save_app_config: password empty from UI, restored from keyring (len={})",
            p.len()
        );
        config.openvpn.password = p;
    } else {
        scp_debug!("save_app_config: password empty from UI, keyring has no entry");
    }
    if !config.openvpn.otp_secret.is_empty() {
        scp_debug!("save_app_config: writing OTP secret to keyring (len={_incoming_otp_len})");
        keyring_set_or_delete(KEYRING_OPENVPN_OTP_SECRET, &config.openvpn.otp_secret)?;
    } else if let Some(s) = keyring_get_optional(KEYRING_OPENVPN_OTP_SECRET)? {
        scp_debug!(
            "save_app_config: OTP empty from UI, restored from keyring (len={})",
            s.len()
        );
        config.openvpn.otp_secret = s;
    } else {
        scp_debug!("save_app_config: OTP empty from UI, keyring has no entry");
    }
    if !config.boundary.password.is_empty() {
        keyring_set_or_delete(KEYRING_BOUNDARY_PASSWORD, &config.boundary.password)?;
    } else if let Some(p) = keyring_get_optional(KEYRING_BOUNDARY_PASSWORD)? {
        config.boundary.password = p;
    }
    if !config.boundary.oidc_password.is_empty() {
        keyring_set_or_delete(
            KEYRING_BOUNDARY_OIDC_PASSWORD,
            &config.boundary.oidc_password,
        )?;
    } else if let Some(p) = keyring_get_optional(KEYRING_BOUNDARY_OIDC_PASSWORD)? {
        config.boundary.oidc_password = p;
    }
    if !config.boundary.oidc_otp_secret.is_empty() {
        keyring_set_or_delete(
            KEYRING_BOUNDARY_OIDC_OTP_SECRET,
            &config.boundary.oidc_otp_secret,
        )?;
    } else if let Some(s) = keyring_get_optional(KEYRING_BOUNDARY_OIDC_OTP_SECRET)? {
        config.boundary.oidc_otp_secret = s;
    }
    save_local_app_settings(&app, &config)?;
    save_app_config_to_disk(&app, &config)?;
    scp_debug!(
        "save_app_config: done; password_present={} otp_present={}",
        !config.openvpn.password.is_empty(),
        !config.openvpn.otp_secret.is_empty()
    );
    Ok(config)
}

#[tauri::command]
fn detect_openvpn_executable() -> Result<String, String> {
    detect_openvpn_executable_path()
        .ok_or_else(|| "OpenVPN executable not found in PATH/common locations".to_string())
}

#[tauri::command]
fn start_openvpn_function(
    _command: String,
    username: String,
    password: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<StartFunctionResult, String> {
    scp_debug!("start_openvpn_function: invoked");
    let saved_config = load_app_config_from_disk(&app)?;
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

    scp_debug!(
        "start_openvpn_function: after load — exe_nonempty={} ovpn_nonempty={} user_nonempty={} password_nonempty={}",
        !resolved_openvpn.executable_path.trim().is_empty(),
        !resolved_openvpn.ovpn_path.trim().is_empty(),
        !resolved_username.trim().is_empty(),
        !resolved_password.is_empty()
    );

    if resolved_openvpn.executable_path.trim().is_empty() {
        scp_debug!("start_openvpn_function: ERR empty executable_path");
        return Err("openvpn executable path cannot be empty".to_string());
    }
    if resolved_openvpn.ovpn_path.trim().is_empty() {
        scp_debug!("start_openvpn_function: ERR empty ovpn_path");
        return Err("ovpn file path cannot be empty".to_string());
    }
    if resolved_username.trim().is_empty() {
        scp_debug!("start_openvpn_function: ERR empty username");
        return Err("username cannot be empty".to_string());
    }
    if resolved_password.is_empty() {
        scp_debug!("start_openvpn_function: ERR empty password (keyring/disk missing?)");
        return Err("password cannot be empty".to_string());
    }

    let combined_password =
        match generate_openvpn_password(&resolved_password, &resolved_otp_secret) {
            Ok(s) => s,
            Err(e) => {
                scp_debug!("start_openvpn_function: ERR generate_openvpn_password: {e}");
                return Err(e);
            }
        };

    #[cfg(target_os = "windows")]
    {
        let _ = &session_store;
        let openvpn_args = match build_openvpn_args_windows(&resolved_openvpn) {
            Ok(a) => a,
            Err(e) => {
                scp_debug!("start_openvpn_function: ERR build_openvpn_args_windows: {e}");
                return Err(e);
            }
        };
        scp_debug!(
            "start_openvpn_function: Windows path — launching elevated helper; exe={}",
            resolved_openvpn.executable_path
        );
        if let Some(active) = get_openvpn_admin_session(&admin_store)? {
            let _ = fs::write(&active.stop_path, "stop");
        }
        let session_id = start_openvpn_admin_worker(
            &app,
            &function_store,
            &admin_store,
            resolved_openvpn.executable_path.clone(),
            openvpn_args,
            resolved_username,
            combined_password,
            resolved_openvpn.terminate_open_vpn_when_helper_exits,
        )?;
        scp_debug!("start_openvpn_function: Windows helper session_id={session_id}");

        return Ok(StartFunctionResult {
            function_id: OPENVPN_FUNCTION_ID.to_string(),
            session_id,
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let current = lock_err(&function_store.statuses)?
            .get(OPENVPN_FUNCTION_ID)
            .cloned();
        if let Some(current) = current {
            if let Some(existing_session_id) = current.session_id {
                let _ = close_pty_internal(&existing_session_id, &session_store);
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        upsert_function_status(
            &app,
            &function_store,
            OPENVPN_FUNCTION_ID,
            "starting",
            "Opening PTY session",
            None,
        )?;

        let session_id = open_pty_internal(
            saved_config.app.default_terminal_cols,
            saved_config.app.default_terminal_rows,
            &app,
            &session_store,
        )?;

        upsert_function_status(
            &app,
            &function_store,
            OPENVPN_FUNCTION_ID,
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

        return Ok(StartFunctionResult {
            function_id: OPENVPN_FUNCTION_ID.to_string(),
            session_id,
        });
    }
}

#[tauri::command]
fn stop_openvpn_function(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = &session_store;
        let active = get_openvpn_admin_session(&admin_store)?;
        if let Some(active) = active {
            let helper_pid_path =
                openvpn_runtime_dir(&app)?.join(format!("{}.helper.pid", active.id));
            if let Ok(s) = fs::read_to_string(&helper_pid_path) {
                if let Ok(pid) = s.trim().parse::<u32>() {
                    if pid > 0 && !is_windows_pid_running(pid) {
                        let _ = fs::remove_file(&helper_pid_path);
                        upsert_function_status(
                            &app,
                            &function_store,
                            OPENVPN_FUNCTION_ID,
                            "stopped",
                            "Helper already exited",
                            None,
                        )?;
                        set_openvpn_admin_session(&admin_store, None)?;
                        return Ok(());
                    }
                }
            }
            upsert_function_status(
                &app,
                &function_store,
                OPENVPN_FUNCTION_ID,
                "stopping",
                "Requesting elevated OpenVPN stop",
                Some(active.id.clone()),
            )?;
            fs::write(&active.stop_path, "stop")
                .map_err(|e| format!("failed to signal openvpn stop: {e}"))?;
        } else {
            upsert_function_status(
                &app,
                &function_store,
                OPENVPN_FUNCTION_ID,
                "idle",
                "No running admin session",
                None,
            )?;
        }

        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let session_id = {
            let statuses = lock_err(&function_store.statuses)?;
            statuses
                .get(OPENVPN_FUNCTION_ID)
                .and_then(|status| status.session_id.clone())
        };

        if let Some(session_id) = session_id {
            upsert_function_status(
                &app,
                &function_store,
                OPENVPN_FUNCTION_ID,
                "stopping",
                "Stopping OpenVPN PTY session",
                Some(session_id.clone()),
            )?;

            let payload = close_pty_internal(&session_id, &session_store)?;
            if let Some(payload) = payload {
                emit_pty_exit(&app, payload.clone());
                let _ = update_functions_for_session_exit(
                    &app,
                    &function_store,
                    &session_id,
                    payload.exit_code,
                    &payload.reason,
                );
            }
        } else {
            upsert_function_status(
                &app,
                &function_store,
                OPENVPN_FUNCTION_ID,
                "idle",
                "No running session",
                None,
            )?;
        }

        return Ok(());
    }
}

#[tauri::command]
fn start_caddy_run(
    cols: u16,
    rows: u16,
    folder: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<StartFunctionResult, String> {
    let folder_trim = folder.trim();
    if folder_trim.is_empty() {
        return Err("folder cannot be empty".to_string());
    }
    let folder_path = PathBuf::from(folder_trim);
    if !folder_path.exists() {
        return Err(format!(
            "folder does not exist: {}",
            folder_path.to_string_lossy()
        ));
    }
    let caddyfile = folder_path.join("Caddyfile");
    if !caddyfile.exists() {
        return Err(format!(
            "Caddyfile not found in folder: {}",
            folder_path.to_string_lossy()
        ));
    }

    if let Some(prev) = lock_err(&function_store.statuses)?
        .get(CADDY_FUNCTION_ID)
        .and_then(|s| s.session_id.clone())
    {
        if let Some(p) = close_pty_internal(&prev, &session_store)? {
            emit_pty_exit(&app, p);
        }
        let _ = update_functions_for_session_exit(&app, &function_store, &prev, None, "replaced");
    }

    let mut cmd = CommandBuilder::new("caddy");
    cmd.arg("run");
    cmd.arg("--config");
    cmd.arg("Caddyfile");
    cmd.cwd(folder_path.clone());

    let session_id = open_pty_with_command_builder(cols, rows, cmd, &app, &session_store)?;
    upsert_function_status(
        &app,
        &function_store,
        CADDY_FUNCTION_ID,
        "running",
        &format!("Caddy running in {}", folder_path.to_string_lossy()),
        Some(session_id.clone()),
    )?;

    Ok(StartFunctionResult {
        function_id: CADDY_FUNCTION_ID.to_string(),
        session_id,
    })
}

#[tauri::command]
fn stop_caddy_run(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<(), String> {
    let sid = match lock_err(&function_store.statuses)?
        .get(CADDY_FUNCTION_ID)
        .and_then(|s| s.session_id.clone())
    {
        Some(s) => s,
        None => {
            upsert_function_status(
                &app,
                &function_store,
                CADDY_FUNCTION_ID,
                "idle",
                "No running session",
                None,
            )?;
            return Ok(());
        }
    };
    if let Some(p) = close_pty_internal(&sid, &session_store)? {
        emit_pty_exit(&app, p);
    }
    upsert_function_status(
        &app,
        &function_store,
        CADDY_FUNCTION_ID,
        "idle",
        "Stopped",
        None,
    )?;
    Ok(())
}

#[tauri::command]
fn list_servers(app: AppHandle) -> Result<Vec<Server>, String> {
    load_servers(&app)
}

fn shutdown_remote_browser_session_state(state: &RemoteBrowserSessionState) {
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

fn close_remote_browser_session_internal(
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

fn close_remote_browser_sessions_for_server(
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

fn close_all_remote_browser_sessions(browser_store: &RemoteBrowserStore) -> Result<(), String> {
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

#[tauri::command]
fn upsert_server(
    app: AppHandle,
    mut server: Server,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<Server>, String> {
    let mut list = load_servers(&app)?;
    if server.id.is_empty() {
        server.id = new_server_id();
        list.push(server);
    } else if let Some(i) = list.iter().position(|s| s.id == server.id) {
        close_remote_browser_sessions_for_server(&server.id, &browser_store)?;
        list[i] = server;
    } else {
        list.push(server);
    }
    save_servers(&app, &list)?;
    Ok(list)
}

#[tauri::command]
fn delete_server(
    app: AppHandle,
    id: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<Server>, String> {
    close_remote_browser_sessions_for_server(&id, &browser_store)?;
    let mut list = load_servers(&app)?;
    list.retain(|s| s.id != id);
    save_servers(&app, &list)?;
    Ok(list)
}

/// Search Boundary targets (uses CLI token). Optional text filter; empty = list all.
#[tauri::command]
fn search_boundary_targets(
    app: AppHandle,
    filter: Option<String>,
) -> Result<Vec<BoundaryTargetSummary>, String> {
    let cfg = load_app_config_from_disk(&app)?;
    let exe = boundary_cli_path(&cfg);
    if exe != "boundary" && !PathBuf::from(&exe).exists() {
        return Err("Boundary CLI not found in config".to_string());
    }
    let mut cmd = Command::new(&exe);
    cmd.arg("targets");
    cmd.arg("list");
    apply_boundary_token_arg_if_present(&mut cmd)?;
    cmd.arg("-recursive");
    cmd.arg("-format");
    cmd.arg("json");
    apply_boundary_env(&mut cmd, &cfg)?;
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd
        .output()
        .map_err(|e| format!("boundary targets list failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "boundary targets list failed (check Boundary auth in Status)".to_string()
        } else {
            stderr
        });
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("invalid JSON: {e}"))?;
    let items = v
        .get("items")
        .and_then(|a| a.as_array())
        .ok_or_else(|| "expected items array in boundary output".to_string())?;
    let filter_lower = filter
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let mut results: Vec<BoundaryTargetSummary> = Vec::new();
    for item in items {
        let obj = match item.as_object() {
            Some(o) => o,
            None => continue,
        };
        let id = obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let target_type = obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let description = obj
            .get("description")
            .or_else(|| obj.get("attributes").and_then(|a| a.get("description")))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let address = obj
            .get("address")
            .or_else(|| obj.get("attributes").and_then(|a| a.get("address")))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if let Some(ref f) = filter_lower {
            if !name.to_lowercase().contains(f)
                && !id.to_lowercase().contains(f)
                && !description.to_lowercase().contains(f)
            {
                continue;
            }
        }
        results.push(BoundaryTargetSummary {
            id,
            name,
            target_type,
            description,
            address,
        });
    }
    Ok(results)
}

#[tauri::command]
fn start_server_tunnel(
    server_id: String,
    cols: u16,
    rows: u16,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<StartFunctionResult, String> {
    close_remote_browser_sessions_for_server(&server_id, &browser_store)?;
    let servers = load_servers(&app)?;
    let server = servers
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| "server not found".to_string())?;
    if server.connection_method != ConnectionMethod::Boundary {
        return Err("Tunnel is only for Boundary servers".to_string());
    }
    if server.boundary_target_id.trim().is_empty() {
        return Err("Boundary target ID is required".to_string());
    }
    let fid = server_tunnel_function_id(&server_id);
    if let Some(prev) = lock_err(&function_store.statuses)?
        .get(&fid)
        .and_then(|s| s.session_id.clone())
    {
        if let Some(p) = close_pty_internal(&prev, &session_store)? {
            emit_pty_exit(&app, p);
        }
        let _ = update_functions_for_session_exit(&app, &function_store, &prev, None, "replaced");
    }
    let port = if server.boundary_listen_port == 0 {
        6222u16
    } else {
        server.boundary_listen_port
    };
    let cfg = load_app_config_from_disk(&app)?;
    let exe = boundary_cli_path(&cfg);
    let mut c = CommandBuilder::new(exe);
    c.arg("connect");
    apply_boundary_token_arg_if_present_builder(&mut c)?;
    apply_boundary_env_builder(&mut c, &cfg)?;
    c.arg("-target-id");
    c.arg(server.boundary_target_id.trim());
    c.arg("-listen-port");
    c.arg(port.to_string());
    let session_id = open_pty_with_command_builder(cols, rows, c, &app, &session_store)?;
    upsert_function_status(
        &app,
        &function_store,
        &fid,
        "running",
        &format!("Tunnel: {} (port {})", server.name, port),
        Some(session_id.clone()),
    )?;
    Ok(StartFunctionResult {
        function_id: fid,
        session_id,
    })
}

#[tauri::command]
fn stop_server_tunnel(
    server_id: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    close_remote_browser_sessions_for_server(&server_id, &browser_store)?;
    let fid = server_tunnel_function_id(&server_id);
    let sid = match lock_err(&function_store.statuses)?
        .get(&fid)
        .and_then(|s| s.session_id.clone())
    {
        Some(s) => s,
        None => return Ok(()),
    };
    if let Some(p) = close_pty_internal(&sid, &session_store)? {
        emit_pty_exit(&app, p);
    }
    let message = load_servers(&app)
        .ok()
        .and_then(|servers| servers.into_iter().find(|s| s.id == server_id))
        .map(|s| {
            let port = if s.boundary_listen_port == 0 {
                6222
            } else {
                s.boundary_listen_port
            };
            format!("Tunnel: {} (port {}) — stopped", s.name, port)
        })
        .unwrap_or_else(|| "Stopped".to_string());
    upsert_function_status(&app, &function_store, &fid, "idle", &message, None)?;
    Ok(())
}

#[tauri::command]
fn start_server_ssh(
    server_id: String,
    cols: u16,
    rows: u16,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<StartFunctionResult, String> {
    let servers = load_servers(&app)?;
    let server = servers
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| "server not found".to_string())?;
    let fid = server_ssh_function_id(&server_id);
    if let Some(prev) = lock_err(&function_store.statuses)?
        .get(&fid)
        .and_then(|s| s.session_id.clone())
    {
        if let Some(p) = close_pty_internal(&prev, &session_store)? {
            emit_pty_exit(&app, p);
        }
        let _ = update_functions_for_session_exit(&app, &function_store, &prev, None, "replaced");
    }
    let cmd = match &server.connection_method {
        ConnectionMethod::Ssh => {
            if server.ssh_user.trim().is_empty() || server.host.trim().is_empty() {
                return Err("host and SSH user are required for direct SSH".to_string());
            }
            let mut c = CommandBuilder::new("ssh");
            c.arg("-p");
            c.arg(server.ssh_port.to_string());
            c.arg("-o");
            c.arg("StrictHostKeyChecking=accept-new");
            c.arg("-o");
            c.arg("MACs=hmac-sha2-512");
            c.arg(format!("{}@{}", server.ssh_user.trim(), server.host.trim()));
            c
        }
        ConnectionMethod::Boundary => {
            let port = if server.boundary_listen_port == 0 {
                6222u16
            } else {
                server.boundary_listen_port
            };
            let user = if server.ssh_user.trim().is_empty() {
                return Err("SSH username required for Boundary (connect to localhost)".to_string());
            } else {
                server.ssh_user.trim()
            };
            let mut c = CommandBuilder::new("ssh");
            c.arg("-p");
            c.arg(port.to_string());
            c.arg("-o");
            c.arg("StrictHostKeyChecking=accept-new");
            c.arg("-o");
            c.arg("MACs=hmac-sha2-512");
            c.arg(format!("{}@127.0.0.1", user));
            c
        }
    };
    let session_id = open_pty_with_command_builder(cols, rows, cmd, &app, &session_store)?;
    upsert_function_status(
        &app,
        &function_store,
        &fid,
        "running",
        &format!("SSH: {}", server.name),
        Some(session_id.clone()),
    )?;
    Ok(StartFunctionResult {
        function_id: fid,
        session_id,
    })
}

#[tauri::command]
fn stop_server_ssh(
    server_id: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<(), String> {
    let fid = server_ssh_function_id(&server_id);
    let sid = match lock_err(&function_store.statuses)?
        .get(&fid)
        .and_then(|s| s.session_id.clone())
    {
        Some(s) => s,
        None => return Ok(()),
    };
    if let Some(p) = close_pty_internal(&sid, &session_store)? {
        emit_pty_exit(&app, p);
    }
    let message = load_servers(&app)
        .ok()
        .and_then(|servers| servers.into_iter().find(|s| s.id == server_id))
        .map(|s| format!("SSH: {} — stopped", s.name))
        .unwrap_or_else(|| "Stopped".to_string());
    upsert_function_status(&app, &function_store, &fid, "idle", &message, None)?;
    Ok(())
}

fn load_server_by_id(app: &AppHandle, server_id: &str) -> Result<Server, String> {
    load_servers(app)?
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| "server not found".to_string())
}

fn resolve_server_ssh_endpoint(server: &Server, operation: &str) -> Result<SshEndpoint, String> {
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
                6222
            } else {
                server.boundary_listen_port
            },
        }),
    }
}

fn add_common_ssh_options(cmd: &mut Command, port_flag: &str, port: u16) {
    cmd.arg(port_flag)
        .arg(port.to_string())
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("MACs=hmac-sha2-512");
}

fn ssh_target(endpoint: &SshEndpoint) -> String {
    format!("{}@{}", endpoint.user, endpoint.host)
}

fn shell_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "'\"'\"'"))
}

fn ensure_boundary_tunnel_running(
    function_store: &FunctionStore,
    server: &Server,
) -> Result<(), String> {
    if server.connection_method != ConnectionMethod::Boundary {
        return Ok(());
    }
    let tunnel_id = server_tunnel_function_id(&server.id);
    let statuses = lock_err(&function_store.statuses)?;
    let running = statuses
        .get(&tunnel_id)
        .map(|status| status.state == "running" && status.session_id.is_some())
        .unwrap_or(false);
    if running {
        Ok(())
    } else {
        Err("Boundary tunnel is not running for this server.".to_string())
    }
}

fn run_ssh_command(endpoint: &SshEndpoint, remote_script: &str) -> Result<String, String> {
    let mut cmd = Command::new("ssh");
    add_common_ssh_options(&mut cmd, "-p", endpoint.port);
    cmd.arg(ssh_target(endpoint)).arg(remote_script);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd
        .output()
        .map_err(|e| format!("ssh failed to run: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "ssh command failed".to_string()
        } else {
            stderr
        });
    }
    String::from_utf8(out.stdout).map_err(|e| format!("ssh output was not valid UTF-8: {e}"))
}

fn next_remote_browser_id(store: &RemoteBrowserStore) -> Result<String, String> {
    let mut next = lock_err(&store.next_id)?;
    let id = format!("browser-{}", *next);
    *next += 1;
    Ok(id)
}

fn remote_browser_state(
    browser_id: &str,
    store: &RemoteBrowserStore,
) -> Result<Arc<RemoteBrowserSessionState>, String> {
    lock_err(&store.sessions)?
        .get(browser_id)
        .cloned()
        .ok_or_else(|| "remote browser session not found".to_string())
}

fn remote_kind_from_file_stat(stat: &FileStat) -> RemotePathKind {
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

fn path_buf_to_remote_string(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn normalize_remote_browser_path(path: &str, fallback: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else if trimmed == "/" {
        "/".to_string()
    } else {
        trimmed.trim_end_matches('/').to_string()
    }
}

fn parse_remote_browser_ls_entry(base_path: &str, raw: &str) -> Option<RemoteBrowseEntry> {
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

fn join_remote_path(base: &str, name: &str) -> String {
    let clean_name = name.trim_start_matches('/');
    if base == "/" {
        format!("/{clean_name}")
    } else {
        format!("{}/{}", base.trim_end_matches('/'), clean_name)
    }
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

fn spawn_cli_remote_browser_session(
    endpoint: &SshEndpoint,
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
    cmd.creation_flags(CREATE_NO_WINDOW);
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

fn remote_browser_root_path_via_cli_session(
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

fn list_remote_browser_children_via_cli_session(
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

fn stat_remote_browser_path_via_cli_session(
    session: &CliRemoteBrowserSession,
    path: &str,
) -> Result<RemoteBrowseNodeInfo, String> {
    let quoted = shell_quote(path);
    let script = format!(
        "p={quoted}; if [ -d \"$p\" ]; then printf 'directory'; \
         elif [ -f \"$p\" ]; then printf 'file'; \
         elif [ -L \"$p\" ]; then printf 'symlink'; \
         elif [ -e \"$p\" ]; then printf 'other'; \
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

fn create_remote_browser_directory_via_cli_session(
    session: &CliRemoteBrowserSession,
    path: &str,
) -> Result<(), String> {
    let script = format!("mkdir -p -- {}", shell_quote(path));
    run_cli_remote_browser_command(session, &script).map(|_| ())
}

fn delete_remote_browser_path_via_cli_session(
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

fn remove_remote_sftp_path_recursive(sftp: &ssh2::Sftp, path: &std::path::Path) -> Result<(), String> {
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

fn open_remote_sftp_session(
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

#[tauri::command]
fn open_remote_browser(
    app: AppHandle,
    server_id: String,
    browser_store: State<'_, RemoteBrowserStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<RemoteBrowseSession, String> {
    close_all_remote_browser_sessions(&browser_store)?;
    let server = load_server_by_id(&app, &server_id)?;
    ensure_boundary_tunnel_running(&function_store, &server)?;
    let endpoint = resolve_server_ssh_endpoint(&server, "remote browser")?;
    #[cfg(target_os = "windows")]
    let (root_path, transport) = {
        let session = spawn_cli_remote_browser_session(&endpoint)?;
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
    let browser_id = next_remote_browser_id(&browser_store)?;
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

#[tauri::command]
fn close_remote_browser(
    browser_id: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    close_remote_browser_session_internal(&browser_id, &browser_store)
}

#[tauri::command]
fn list_remote_browser_children(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<RemoteBrowseEntry>, String> {
    let browser = remote_browser_state(&browser_id, &browser_store)?;
    let target_path = normalize_remote_browser_path(&path, &browser.root_path);
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

#[tauri::command]
fn stat_remote_browser_path(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<RemoteBrowseNodeInfo, String> {
    let browser = remote_browser_state(&browser_id, &browser_store)?;
    let target_path = normalize_remote_browser_path(&path, &browser.root_path);
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

#[tauri::command]
fn create_remote_browser_directory(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    let browser = remote_browser_state(&browser_id, &browser_store)?;
    let target_path = normalize_remote_browser_path(&path, &browser.root_path);
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

#[tauri::command]
fn delete_remote_browser_path(
    browser_id: String,
    path: String,
    recursive: Option<bool>,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    let browser = remote_browser_state(&browser_id, &browser_store)?;
    let target_path = normalize_remote_browser_path(&path, &browser.root_path);
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

#[tauri::command]
fn scp_upload(
    app: AppHandle,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
) -> Result<String, String> {
    let server = load_server_by_id(&app, &server_id)?;
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

fn build_scp_command(
    endpoint: &SshEndpoint,
    local_path: &str,
    remote_path: &str,
    recursive: bool,
    download: bool,
) -> CommandBuilder {
    let remote_spec = format!("{}:{}", ssh_target(endpoint), remote_path);
    let mut cmd = CommandBuilder::new("scp");
    cmd.arg("-P");
    cmd.arg(endpoint.port.to_string());
    cmd.arg("-o");
    cmd.arg("StrictHostKeyChecking=accept-new");
    cmd.arg("-o");
    cmd.arg("MACs=hmac-sha2-512");
    if recursive {
        cmd.arg("-r");
    }
    if download {
        cmd.arg(remote_spec);
        cmd.arg(local_path);
    } else {
        cmd.arg(local_path);
        cmd.arg(remote_spec);
    }
    cmd
}

#[tauri::command]
fn start_scp_upload(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
) -> Result<String, String> {
    let server = load_server_by_id(&app, &server_id)?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let cmd = build_scp_command(
        &endpoint,
        &local_path,
        &remote_path,
        recursive.unwrap_or(false),
        false,
    );
    open_pty_with_command_builder(120, 24, cmd, &app, &session_store)
}

#[tauri::command]
fn scp_download(
    app: AppHandle,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
) -> Result<String, String> {
    let server = load_server_by_id(&app, &server_id)?;
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

#[tauri::command]
fn start_scp_download(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
) -> Result<String, String> {
    let server = load_server_by_id(&app, &server_id)?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let cmd = build_scp_command(
        &endpoint,
        &local_path,
        &remote_path,
        recursive.unwrap_or(false),
        true,
    );
    open_pty_with_command_builder(120, 24, cmd, &app, &session_store)
}

#[tauri::command]
fn start_server_rdp(app: AppHandle, server_id: String) -> Result<(), String> {
    let servers = load_servers(&app)?;
    let server = servers
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| "server not found".to_string())?;
    if server.os != ServerOs::Windows {
        return Err(
            "RDP is only available for Windows servers. Edit the server and set OS to Windows."
                .to_string(),
        );
    }
    let host = server.host.trim();
    if host.is_empty() {
        return Err(
            "Host is required for RDP — set a DNS name or IP address on the server.".to_string(),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("mstsc.exe");
        cmd.arg(format!("/v:{host}"));
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.spawn()
            .map_err(|e| format!("failed to launch mstsc.exe: {e}"))?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let url = format!("rdp://{host}");
        let status = Command::new("open")
            .arg(&url)
            .status()
            .map_err(|e| format!("failed to launch RDP URL via `open`: {e}"))?;
        if !status.success() {
            return Err("failed to launch RDP client via `open`".to_string());
        }
        Ok(())
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let url = format!("rdp://{host}");
        let status = Command::new("xdg-open")
            .arg(&url)
            .status()
            .map_err(|e| format!("failed to launch RDP URL via `xdg-open`: {e}"))?;
        if !status.success() {
            return Err("failed to launch RDP client via `xdg-open`".to_string());
        }
        Ok(())
    }
}

fn stop_boundary_automation_session(
    app: &AppHandle,
    automation_store: &BoundaryAutomationStore,
    session_store: &SessionStore,
    function_store: &FunctionStore,
) -> Result<(), String> {
    let prior = {
        let mut guard = lock_err(&automation_store.session)?;
        guard.take()
    };
    if let Some(prior) = prior {
        prior.cancel.store(true, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window(BOUNDARY_OIDC_WINDOW_LABEL) {
            let _ = window.close();
        }
        if let Some(payload) = close_pty_internal(&prior.session_id, session_store)? {
            emit_pty_exit(app, payload.clone());
            let _ = update_functions_for_session_exit(
                app,
                function_store,
                &prior.session_id,
                payload.exit_code,
                &payload.reason,
            );
        }
    }
    Ok(())
}

fn complete_boundary_automation_session(
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    session_id: &str,
    exit_code: Option<i32>,
    reason: &str,
) {
    if let Ok(Some(payload)) = close_pty_internal(session_id, session_store) {
        emit_pty_exit(app, payload.clone());
        let _ = update_functions_for_session_exit(
            app,
            function_store,
            session_id,
            payload.exit_code,
            &payload.reason,
        );
    } else {
        let _ =
            update_functions_for_session_exit(app, function_store, session_id, exit_code, reason);
    }
}

fn run_boundary_oidc_automation(
    app: AppHandle,
    session_id: String,
    cfg: BoundaryConfig,
    cancel: Arc<AtomicBool>,
) {
    let function_store = app.state::<FunctionStore>();
    let session_store = app.state::<SessionStore>();
    let automation_store = app.state::<BoundaryAutomationStore>();

    let finish = |exit_code: Option<i32>, reason: &str| {
        if let Ok(mut guard) = automation_store.session.lock() {
            guard.take();
        }
        if let Some(window) = app.get_webview_window(BOUNDARY_OIDC_WINDOW_LABEL) {
            let _ = window.close();
        }
        complete_boundary_automation_session(
            &app,
            &session_store,
            &function_store,
            &session_id,
            exit_code,
            reason,
        );
    };

    let addr = boundary_addr_base(&cfg.addr);
    let auth_endpoint = format!(
        "{addr}/v1/auth-methods/{}:authenticate",
        cfg.auth_method_id.trim()
    );
    emit_pty_output_line(
        &app,
        &session_id,
        format!("Starting automated OIDC login against {addr}"),
    );

    let client = match boundary_http_client() {
        Ok(client) => client,
        Err(err) => {
            emit_pty_output_line(&app, &session_id, format!("HTTP client error: {err}"));
            finish(Some(1), &err);
            return;
        }
    };

    let login_resp = match client
        .post(&auth_endpoint)
        .json(&json!({
            "command": "start"
        }))
        .send()
    {
        Ok(resp) => resp,
        Err(err) => {
            let msg = format!("Boundary login request failed: {err}");
            emit_pty_output_line(&app, &session_id, &msg);
            finish(Some(1), &msg);
            return;
        }
    };
    if !login_resp.status().is_success() {
        let status = login_resp.status();
        let body = login_resp.text().unwrap_or_default();
        let msg = format!(
            "Boundary login request failed: HTTP {status} {}",
            body.trim()
        );
        emit_pty_output_line(&app, &session_id, &msg);
        finish(Some(1), &msg);
        return;
    }
    let login_data: BoundaryApiAuthenticateResponse = match login_resp.json() {
        Ok(data) => data,
        Err(err) => {
            let msg = format!("Failed to decode Boundary login response: {err}");
            emit_pty_output_line(&app, &session_id, &msg);
            finish(Some(1), &msg);
            return;
        }
    };
    if login_data.attributes.auth_url.trim().is_empty()
        || login_data.attributes.token_id.trim().is_empty()
    {
        let msg = "Boundary login response did not include auth_url/token_id".to_string();
        emit_pty_output_line(&app, &session_id, &msg);
        finish(Some(1), &msg);
        return;
    }

    let parsed_auth_url = match login_data.attributes.auth_url.parse() {
        Ok(url) => url,
        Err(err) => {
            let msg = format!("Invalid OIDC auth URL: {err}");
            emit_pty_output_line(&app, &session_id, &msg);
            finish(Some(1), &msg);
            return;
        }
    };
    let builder = WebviewWindowBuilder::new(
        &app,
        BOUNDARY_OIDC_WINDOW_LABEL,
        WebviewUrl::External(parsed_auth_url),
    )
    .visible(false)
    .title("Boundary OIDC Automation")
    .initialization_script_for_all_frames(boundary_oidc_init_script(&cfg));
    if let Err(err) = builder.build() {
        let msg = format!("Failed to open automation webview: {err}");
        emit_pty_output_line(&app, &session_id, &msg);
        finish(Some(1), &msg);
        return;
    }

    emit_pty_output_line(
        &app,
        &session_id,
        "Hidden OIDC window launched. Waiting for callback and token issuance.",
    );
    let token_id = login_data.attributes.token_id;
    let mut attempts: u32 = 0;
    loop {
        if cancel.load(Ordering::SeqCst) {
            emit_pty_output_line(&app, &session_id, "OIDC automation cancelled.");
            finish(Some(1), "cancelled");
            return;
        }
        thread::sleep(Duration::from_secs(1));
        attempts += 1;
        let token_resp = match client
            .post(&auth_endpoint)
            .json(&json!({
                "command": "token",
                "token_id": token_id,
                "attributes": { "token_id": token_id }
            }))
            .send()
        {
            Ok(resp) => resp,
            Err(err) => {
                let msg = format!("Boundary token polling failed: {err}");
                emit_pty_output_line(&app, &session_id, &msg);
                finish(Some(1), &msg);
                return;
            }
        };
        if token_resp.status().is_success() {
            let token_data: BoundaryApiAuthenticateResponse = match token_resp.json() {
                Ok(data) => data,
                Err(err) => {
                    let msg = format!("Failed to decode token response: {err}");
                    emit_pty_output_line(&app, &session_id, &msg);
                    finish(Some(1), &msg);
                    return;
                }
            };
            if !token_data.attributes.token.trim().is_empty() {
                if let Err(err) = keyring_set_or_delete(
                    KEYRING_BOUNDARY_APP_TOKEN,
                    token_data.attributes.token.trim(),
                ) {
                    let msg = format!("Failed to store Boundary token: {err}");
                    emit_pty_output_line(&app, &session_id, &msg);
                    finish(Some(1), &msg);
                    return;
                }
                emit_pty_output_line(
                    &app,
                    &session_id,
                    if token_data.attributes.expiration_time.trim().is_empty() {
                        "OIDC login succeeded. Token stored in the app secure store.".to_string()
                    } else {
                        format!(
                            "OIDC login succeeded. Token expires at {}.",
                            token_data.attributes.expiration_time.trim()
                        )
                    },
                );
                finish(Some(0), "automated OIDC login complete");
                return;
            }
        } else if !matches!(token_resp.status().as_u16(), 400 | 404) {
            let status = token_resp.status();
            let body = token_resp.text().unwrap_or_default();
            let msg = format!(
                "Boundary token polling failed: HTTP {status} {}",
                body.trim()
            );
            emit_pty_output_line(&app, &session_id, &msg);
            finish(Some(1), &msg);
            return;
        } else if attempts % 5 == 0 {
            emit_pty_output_line(
                &app,
                &session_id,
                "Still waiting for the IdP to complete login and callback.",
            );
        }
    }
}

#[tauri::command]
fn start_boundary_auth(
    cols: u16,
    rows: u16,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    automation_store: State<'_, BoundaryAutomationStore>,
) -> Result<StartFunctionResult, String> {
    let cfg = load_app_config_from_disk(&app)?;
    let am = cfg.boundary.auth_method_id.trim();
    let addr = cfg.boundary.addr.trim();
    match cfg.boundary.auth_kind {
        BoundaryAuthKind::Oidc => {
            if am.is_empty() || addr.is_empty() {
                return Err(
                    "OIDC: set Controller URL and OIDC auth method ID (amoidc_…) in Config, then Save."
                        .into(),
                );
            }
            if cfg.boundary.oidc_login_mode == BoundaryOidcLoginMode::Automated
                && (cfg.boundary.oidc_username.trim().is_empty()
                    || cfg.boundary.oidc_password.is_empty())
            {
                return Err(
                    "OIDC automation: set OIDC username and password in Config, then Save.".into(),
                );
            }
        }
        BoundaryAuthKind::Password => {
            if am.is_empty()
                || cfg.boundary.login_name.trim().is_empty()
                || cfg.boundary.password.is_empty()
            {
                return Err(
                    "Password: set auth method ID (ampw_…), login name, and password in Config, then Save."
                        .into(),
                );
            }
        }
    }
    keyring_set_or_delete(KEYRING_BOUNDARY_APP_TOKEN, "")?;
    stop_boundary_automation_session(&app, &automation_store, &session_store, &function_store)?;
    if let Some(prev) = lock_err(&function_store.statuses)?
        .get(BOUNDARY_AUTH_FUNCTION_ID)
        .and_then(|s| s.session_id.clone())
    {
        let _ = close_pty_internal(&prev, &session_store);
    }
    let is_oidc_automation = cfg.boundary.auth_kind == BoundaryAuthKind::Oidc
        && cfg.boundary.oidc_login_mode == BoundaryOidcLoginMode::Automated;
    let session_id = if is_oidc_automation {
        open_pty_with_command_builder(cols, rows, passive_pty_command(), &app, &session_store)?
    } else {
        let exe = boundary_cli_path(&cfg);
        let mut cmd = CommandBuilder::new(exe);
        cmd.arg("authenticate");
        apply_boundary_token_arg_if_present_builder(&mut cmd)?;
        apply_boundary_env_builder(&mut cmd, &cfg)?;
        match cfg.boundary.auth_kind {
            BoundaryAuthKind::Oidc => {
                cmd.arg("oidc");
                cmd.arg("-auth-method-id");
                cmd.arg(am);
            }
            BoundaryAuthKind::Password => {
                cmd.arg("password");
                cmd.arg("-auth-method-id");
                cmd.arg(am);
                cmd.arg("-login-name");
                cmd.arg(cfg.boundary.login_name.trim());
            }
        }
        open_pty_with_command_builder(cols, rows, cmd, &app, &session_store)?
    };
    let status_label = match cfg.boundary.auth_kind {
        BoundaryAuthKind::Oidc if is_oidc_automation => "Boundary OIDC (automated)",
        BoundaryAuthKind::Oidc => "Boundary OIDC (browser)",
        BoundaryAuthKind::Password => "Boundary password login",
    };
    upsert_function_status(
        &app,
        &function_store,
        BOUNDARY_AUTH_FUNCTION_ID,
        "authenticating",
        status_label,
        Some(session_id.clone()),
    )?;
    if is_oidc_automation {
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut guard = lock_err(&automation_store.session)?;
            *guard = Some(BoundaryAutomationSession {
                session_id: session_id.clone(),
                cancel: cancel.clone(),
            });
        }
        let app2 = app.clone();
        let cfg2 = cfg.boundary.clone();
        let sid = session_id.clone();
        thread::spawn(move || run_boundary_oidc_automation(app2, sid, cfg2, cancel));
    } else if cfg.boundary.auth_kind == BoundaryAuthKind::Password {
        let pass = cfg.boundary.password.clone();
        let sid = session_id.clone();
        let app2 = app.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(1200));
            let st = app2.state::<SessionStore>();
            let _ = write_pty_internal(&sid, &format!("{pass}\r"), &st);
        });
    }
    Ok(StartFunctionResult {
        function_id: BOUNDARY_AUTH_FUNCTION_ID.to_string(),
        session_id,
    })
}

#[tauri::command]
fn stop_boundary_auth(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    automation_store: State<'_, BoundaryAutomationStore>,
) -> Result<(), String> {
    stop_boundary_automation_session(&app, &automation_store, &session_store, &function_store)?;
    let sid = match lock_err(&function_store.statuses)?
        .get(BOUNDARY_AUTH_FUNCTION_ID)
        .and_then(|s| s.session_id.clone())
    {
        Some(s) => s,
        None => return Ok(()),
    };
    if let Some(p) = close_pty_internal(&sid, &session_store)? {
        emit_pty_exit(&app, p.clone());
        let _ =
            update_functions_for_session_exit(&app, &function_store, &sid, p.exit_code, &p.reason);
    } else {
        let (state, msg, source) = match boundary_token_summary(&app) {
            Ok(summary) => ("authenticated", summary.message, Some(summary.source)),
            Err(_) => (
                "unauthenticated",
                "Login was stopped; no Boundary token is available.".to_string(),
                None,
            ),
        };
        upsert_boundary_status(&app, &function_store, state, &msg, source, None)?;
    }
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(SessionStore::default())
        .manage(FunctionStore::default())
        .manage(OpenVpnAdminStore::default())
        .manage(BoundaryAutomationStore::default())
        .manage(RemoteBrowserStore::default())
        .invoke_handler(tauri::generate_handler![
            open_pty,
            write_pty,
            resize_pty,
            close_pty,
            list_function_statuses,
            get_app_config,
            get_config_paths,
            get_sync_settings,
            save_sync_settings,
            save_app_config,
            detect_openvpn_executable,
            start_openvpn_function,
            stop_openvpn_function,
            start_caddy_run,
            stop_caddy_run,
            list_servers,
            upsert_server,
            delete_server,
            search_boundary_targets,
            start_server_tunnel,
            stop_server_tunnel,
            start_server_ssh,
            stop_server_ssh,
            open_remote_browser,
            close_remote_browser,
            list_remote_browser_children,
            stat_remote_browser_path,
            create_remote_browser_directory,
            delete_remote_browser_path,
            start_scp_upload,
            scp_upload,
            start_scp_download,
            scp_download,
            start_server_rdp,
            start_boundary_auth,
            stop_boundary_auth,
            sync_config_pull,
            sync_config_push,
            config_git_pull,
            config_git_push
        ])
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
