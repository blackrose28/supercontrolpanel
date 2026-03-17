#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::{
    collections::HashMap,
    env,
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Receiver, Sender},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use keyring::{Entry, Error as KeyringError};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use totp_rs::{Algorithm, Secret, TOTP};

mod server_mgmt;
use server_mgmt::{load_servers, new_server_id, save_servers, ConnectionMethod, Server, ServerOs};

/// One target from `boundary targets list` for search results.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct BoundaryTargetSummary {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    target_type: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    address: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PtyOutputPayload {
    id: String,
    data: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PtyExitPayload {
    id: String,
    exit_code: Option<i32>,
    reason: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FunctionStatus {
    function_id: String,
    state: String,
    message: String,
    session_id: Option<String>,
    updated_at_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StartFunctionResult {
    function_id: String,
    session_id: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OpenVpnLogPayload {
    function_id: String,
    session_id: String,
    data: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AppConfig {
    app: AppRuntimeConfig,
    #[serde(default)]
    openvpn: OpenVpnConfig,
    #[serde(default)]
    boundary: BoundaryConfig,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
enum BoundaryAuthKind {
    #[default]
    Password,
    Oidc,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct BoundaryConfig {
    #[serde(default)]
    cli_path: String,
    #[serde(default)]
    addr: String,
    #[serde(default)]
    auth_method_id: String,
    #[serde(default)]
    login_name: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    auth_kind: BoundaryAuthKind,
    /// Passed to SSH when using boundary connect ssh (after --). e.g. "-o" "MACs=hmac-sha2-256" to fix MAC errors.
    #[serde(default)]
    connect_ssh_options: String,
}

impl Default for BoundaryConfig {
    fn default() -> Self {
        Self {
            cli_path: String::new(),
            addr: String::new(),
            auth_method_id: String::new(),
            login_name: String::new(),
            password: String::new(),
            auth_kind: BoundaryAuthKind::Oidc,
            connect_ssh_options: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct BoundaryConfigForDisk {
    #[serde(default)]
    addr: String,
    #[serde(default)]
    auth_method_id: String,
    #[serde(default)]
    login_name: String,
    #[serde(default)]
    auth_kind: BoundaryAuthKind,
    #[serde(default)]
    connect_ssh_options: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AppRuntimeConfig {
    default_terminal_cols: u16,
    default_terminal_rows: u16,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct OpenVpnConfig {
    #[serde(default)]
    command: String,
    #[serde(default)]
    executable_path: String,
    #[serde(default)]
    ovpn_path: String,
    #[serde(default)]
    extra_args: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    otp_secret: String,
    /// Windows: if true, closing/killing the elevated helper also terminates OpenVPN (job object).
    /// Field name uses `open_vpn` so camelCase matches UI (`terminateOpenVpnWhenHelperExits`).
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    terminate_open_vpn_when_helper_exits: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenVpnLaunchConfig {
    executable_path: String,
    args: String,
    username: String,
    password: String,
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    terminate_open_vpn_when_helper_exits: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelperStatusFile {
    state: String,
    message: String,
    updated_at_ms: Option<u64>,
}

/// Written to `config.json` only (no secrets).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenVpnConfigForDisk {
    command: String,
    extra_args: String,
    username: String,
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    terminate_open_vpn_when_helper_exits: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppConfigForDisk {
    app: AppRuntimeConfig,
    openvpn: OpenVpnConfigForDisk,
    #[serde(default)]
    boundary: BoundaryConfigForDisk,
}

impl From<&AppConfig> for AppConfigForDisk {
    fn from(c: &AppConfig) -> Self {
        Self {
            app: c.app.clone(),
            openvpn: OpenVpnConfigForDisk {
                command: String::new(),
                extra_args: c.openvpn.extra_args.clone(),
                username: c.openvpn.username.clone(),
                terminate_open_vpn_when_helper_exits: c
                    .openvpn
                    .terminate_open_vpn_when_helper_exits,
            },
            boundary: BoundaryConfigForDisk {
                addr: c.boundary.addr.clone(),
                auth_method_id: c.boundary.auth_method_id.clone(),
                login_name: c.boundary.login_name.clone(),
                auth_kind: c.boundary.auth_kind,
                connect_ssh_options: c.boundary.connect_ssh_options.clone(),
            },
        }
    }
}

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

const OPENVPN_FUNCTION_ID: &str = "openvpn-connect";
const CONFIG_FILE_NAME: &str = "config.json";
const SERVERS_FILE_NAME: &str = "servers.json";
const SYNC_SETTINGS_FILE_NAME: &str = "sync-local.json";
/// OS credential store: Windows Credential Manager, macOS Keychain, Linux Secret Service.
const KEYRING_SERVICE: &str = "com.example.supercontrolpanel";
const KEYRING_OPENVPN_PASSWORD: &str = "openvpn_password";
const KEYRING_OPENVPN_OTP_SECRET: &str = "openvpn_otp_secret";
const KEYRING_BOUNDARY_PASSWORD: &str = "boundary_password";
const OPENVPN_HELPER_SCRIPT_NAME: &str = "openvpn-admin-helper.ps1";
const BOUNDARY_AUTH_FUNCTION_ID: &str = "boundary-auth";

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

#[derive(Default)]
struct OpenVpnAdminStore {
    session: Mutex<Option<OpenVpnAdminSession>>,
}

impl Default for FunctionStore {
    fn default() -> Self {
        let mut statuses = HashMap::new();
        for (id, msg) in [
            (OPENVPN_FUNCTION_ID, "Ready"),
            (BOUNDARY_AUTH_FUNCTION_ID, "Not authenticated"),
        ] {
            statuses.insert(
                id.to_string(),
                FunctionStatus {
                    function_id: id.to_string(),
                    state: "idle".to_string(),
                    message: msg.to_string(),
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
                let (state, message) = match boundary_cli_token_summary(app) {
                    Ok(msg) => ("authenticated".to_string(), msg),
                    Err(_) => {
                        if exit_code == Some(0) {
                            (
                                "authenticated".to_string(),
                                "Token saved to CLI keyring (could not re-read token; try Status refresh)."
                                    .to_string(),
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
                            )
                        }
                    }
                };
                status.state = state;
                status.message = message;
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

/// Uses the same token the Boundary CLI stores in the OS keyring after `boundary authenticate`.
/// Probes via `boundary config get-token` (HashiCorp-supported; `token read` is not in all CLI versions).
fn boundary_cli_token_summary(app: &AppHandle) -> Result<String, String> {
    let cfg = load_app_config_from_disk(app)?;
    let exe = if cfg.boundary.cli_path.trim().is_empty() {
        "boundary".to_string()
    } else {
        cfg.boundary.cli_path.trim().to_string()
    };
    let addr = cfg.boundary.addr.trim();
    scp_debug!(
        "boundary_cli_token_summary: exe={exe:?} BOUNDARY_ADDR={}",
        if addr.is_empty() { "unset" } else { addr }
    );
    if exe != "boundary" && !PathBuf::from(&exe).exists() {
        return Err(format!("Boundary CLI not found: {exe}"));
    }
    let mut cmd = Command::new(&exe);
    cmd.arg("config");
    cmd.arg("get-token");
    if !addr.is_empty() {
        cmd.env("BOUNDARY_ADDR", addr);
    }
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd
        .output()
        .map_err(|e| format!("failed to run `{exe} config get-token`: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let code = out.status.code().unwrap_or(-1);
        scp_debug!(
            "boundary config get-token failed: exit_code={code} stderr_len={} stderr={:?}",
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
    Ok("Authenticated — token in CLI keyring (boundary config get-token)".to_string())
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

fn generate_openvpn_password(password: &str, otp_secret: &str) -> Result<String, String> {
    let secret = otp_secret.trim();
    if secret.is_empty() {
        return Ok(password.to_string());
    }

    let secret_bytes = Secret::Encoded(secret.to_string())
        .to_bytes()
        .map_err(|e| format!("invalid OTP secret: {e}"))?;

    let totp = TOTP::new(Algorithm::SHA1, 6, 1, 30, secret_bytes)
        .map_err(|e| format!("failed to initialize OTP generator: {e}"))?;

    let otp = totp
        .generate_current()
        .map_err(|e| format!("failed to generate OTP: {e}"))?;
    Ok(format!("{otp}{password}"))
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
            continue;
        }
        match boundary_cli_token_summary(&app) {
            Ok(msg) => {
                s.state = "authenticated".to_string();
                s.message = msg;
            }
            Err(e) => {
                s.state = "unauthenticated".to_string();
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
                    "No active Boundary CLI token — open Boundary tab and log in. [{err_short}]{addr_hint}"
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
fn save_sync_settings(app: AppHandle, settings: SyncSettings) -> Result<(), String> {
    let mut merged = load_sync_settings_from_disk(&app)?;
    merged.repo_root = settings.repo_root;
    if !settings.openvpn_executable_path.is_empty() {
        merged.openvpn_executable_path = settings.openvpn_executable_path;
    }
    if !settings.openvpn_ovpn_path.is_empty() {
        merged.openvpn_ovpn_path = settings.openvpn_ovpn_path;
    }
    if !settings.boundary_cli_path.is_empty() {
        merged.boundary_cli_path = settings.boundary_cli_path;
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
        Ok(if combined.is_empty() { "OK".into() } else { combined })
    } else {
        let code = out.status.code().map(|c| c.to_string()).unwrap_or_else(|| "unknown".into());
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
    let incoming_pw_len = config.openvpn.password.len();
    let incoming_otp_len = config.openvpn.otp_secret.len();
    // Never wipe keyring when the UI sends empty secrets (WebViews often omit password field values).
    if !config.openvpn.password.is_empty() {
        scp_debug!("save_app_config: writing password to keyring (len={incoming_pw_len})");
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
        scp_debug!("save_app_config: writing OTP secret to keyring (len={incoming_otp_len})");
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
fn list_servers(app: AppHandle) -> Result<Vec<Server>, String> {
    load_servers(&app)
}

#[tauri::command]
fn upsert_server(app: AppHandle, mut server: Server) -> Result<Vec<Server>, String> {
    let mut list = load_servers(&app)?;
    if server.id.is_empty() {
        server.id = new_server_id();
        list.push(server);
    } else if let Some(i) = list.iter().position(|s| s.id == server.id) {
        list[i] = server;
    } else {
        list.push(server);
    }
    save_servers(&app, &list)?;
    Ok(list)
}

#[tauri::command]
fn delete_server(app: AppHandle, id: String) -> Result<Vec<Server>, String> {
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
    let exe = if cfg.boundary.cli_path.trim().is_empty() {
        "boundary".to_string()
    } else {
        cfg.boundary.cli_path.trim().to_string()
    };
    if exe != "boundary" && !PathBuf::from(&exe).exists() {
        return Err("Boundary CLI not found in config".to_string());
    }
    let mut cmd = Command::new(&exe);
    cmd.arg("targets");
    cmd.arg("list");
    cmd.arg("-recursive");
    cmd.arg("-format");
    cmd.arg("json");
    if !cfg.boundary.addr.trim().is_empty() {
        cmd.env("BOUNDARY_ADDR", cfg.boundary.addr.trim());
    }
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
) -> Result<StartFunctionResult, String> {
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
    let exe = if cfg.boundary.cli_path.trim().is_empty() {
        "boundary"
    } else {
        cfg.boundary.cli_path.trim()
    };
    let mut c = CommandBuilder::new(exe);
    if !cfg.boundary.addr.trim().is_empty() {
        c.env("BOUNDARY_ADDR", cfg.boundary.addr.trim());
    }
    c.arg("connect");
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
) -> Result<(), String> {
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

#[tauri::command]
fn scp_upload(
    app: AppHandle,
    server_id: String,
    local_path: String,
    remote_path: String,
) -> Result<String, String> {
    let servers = load_servers(&app)?;
    let server = servers
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| "server not found".to_string())?;
    let (port, host) = match &server.connection_method {
        ConnectionMethod::Ssh => (server.ssh_port, server.host.clone()),
        ConnectionMethod::Boundary => {
            let port = if server.boundary_listen_port == 0 {
                6222u16
            } else {
                server.boundary_listen_port
            };
            (port, "127.0.0.1".to_string())
        }
    };
    if server.ssh_user.trim().is_empty() {
        return Err("SSH user is required for SCP".to_string());
    }
    let remote_spec = format!("{}@{}:{}", server.ssh_user.trim(), host, remote_path);
    let mut scp = Command::new("scp");
    scp.arg("-P")
        .arg(port.to_string())
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("MACs=hmac-sha2-512");
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

#[tauri::command]
fn scp_download(
    app: AppHandle,
    server_id: String,
    remote_path: String,
    local_path: String,
) -> Result<String, String> {
    let servers = load_servers(&app)?;
    let server = servers
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| "server not found".to_string())?;
    let (port, host) = match &server.connection_method {
        ConnectionMethod::Ssh => (server.ssh_port, server.host.clone()),
        ConnectionMethod::Boundary => {
            let port = if server.boundary_listen_port == 0 {
                6222u16
            } else {
                server.boundary_listen_port
            };
            (port, "127.0.0.1".to_string())
        }
    };
    if server.ssh_user.trim().is_empty() {
        return Err("SSH user is required for SCP".to_string());
    }
    let remote_spec = format!("{}@{}:{}", server.ssh_user.trim(), host, remote_path);
    let mut scp = Command::new("scp");
    scp.arg("-P")
        .arg(port.to_string())
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("MACs=hmac-sha2-512");
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

#[tauri::command]
fn start_boundary_auth(
    cols: u16,
    rows: u16,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
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
    if let Some(prev) = lock_err(&function_store.statuses)?
        .get(BOUNDARY_AUTH_FUNCTION_ID)
        .and_then(|s| s.session_id.clone())
    {
        let _ = close_pty_internal(&prev, &session_store);
    }
    let exe = if cfg.boundary.cli_path.trim().is_empty() {
        "boundary"
    } else {
        cfg.boundary.cli_path.trim()
    };
    let mut cmd = CommandBuilder::new(exe);
    if !addr.is_empty() {
        cmd.env("BOUNDARY_ADDR", addr);
    }
    cmd.arg("authenticate");
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
    let session_id = open_pty_with_command_builder(cols, rows, cmd, &app, &session_store)?;
    let status_label = match cfg.boundary.auth_kind {
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
    if cfg.boundary.auth_kind == BoundaryAuthKind::Password {
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
) -> Result<(), String> {
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
        let (state, msg) = match boundary_cli_token_summary(&app) {
            Ok(m) => ("authenticated", m),
            Err(_) => (
                "unauthenticated",
                "Login was stopped; no token in CLI keyring.".to_string(),
            ),
        };
        upsert_function_status(
            &app,
            &function_store,
            BOUNDARY_AUTH_FUNCTION_ID,
            state,
            &msg,
            None,
        )?;
    }
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(SessionStore::default())
        .manage(FunctionStore::default())
        .manage(OpenVpnAdminStore::default())
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
            list_servers,
            upsert_server,
            delete_server,
            search_boundary_targets,
            start_server_tunnel,
            stop_server_tunnel,
            start_server_ssh,
            stop_server_ssh,
            scp_upload,
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
