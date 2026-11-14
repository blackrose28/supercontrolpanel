//! Standalone headless operation authorization, execution, schema catalog, and capability checks.
use crate::contracts::{
    authorize_standalone, map_core_error, redact_fields, standalone_unavailable_reason, ErrorCode,
    Handler, Operation, OperationError, OPERATIONS,
};
use crate::public_dto::{SafeAppConfig, SafeFunctionStatus, SafeSyncSettings, SafeTransfer};
use crate::validation::{
    conforms, input_schema, output_schema, protect_config_replacement, validate_arguments,
};
use scp_core::error::CoreError;
use scp_core::models::{
    AppConfigForDisk, ScpDownloadRequest, StdioPolicy, BOUNDARY_AUTH_FUNCTION_ID,
    OPENVPN_FUNCTION_ID,
};
use scp_core::runtime::{ProfileRuntime, MAX_SERVER_LIST_LIMIT};
use scp_core::secrets::{preflight_keyring, KeyringBackend, KeyringHealth};
use scp_core::supervisor::CancelToken;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

const DEFAULT_STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub fn permitted_standalone_operations(policy: &StdioPolicy) -> Vec<&'static Operation> {
    OPERATIONS
        .iter()
        .filter(|op| authorize_standalone(op.name, policy).is_ok())
        .collect()
}

pub fn execute_standalone<B: KeyringBackend + 'static>(
    runtime: &ProfileRuntime<B>,
    app_version: &str,
    name: &str,
    args: &Value,
) -> Result<Value, OperationError> {
    execute_standalone_cancellable(runtime, app_version, name, args, &CancelToken::new())
}

pub fn execute_standalone_cancellable<B: KeyringBackend + 'static>(
    runtime: &ProfileRuntime<B>,
    app_version: &str,
    name: &str,
    args: &Value,
    cancel: &CancelToken,
) -> Result<Value, OperationError> {
    let policy = runtime
        .reload_stdio_policy()
        .map_err(|e| map_core_error(&e))?;
    let op = authorize_standalone(name, &policy)?;
    validate_arguments(op, args)
        .map_err(|_| OperationError::standalone_new(ErrorCode::InvalidArguments))?;
    if matches!(op.handler, Handler::ConfigSave) {
        protect_standalone_config_replacement(runtime, &args["config"])?;
    }
    let mut data = dispatch_standalone(runtime, app_version, op.handler, args, cancel)
        .map_err(|e| map_core_error(&e))?;
    redact_fields(&mut data);
    if !conforms(&output_schema(op), &data) {
        return Err(OperationError::standalone_new(ErrorCode::OperationFailed));
    }
    Ok(data)
}

fn protect_standalone_config_replacement<B: KeyringBackend>(
    runtime: &ProfileRuntime<B>,
    proposed: &Value,
) -> Result<(), OperationError> {
    let current_cfg = runtime.get_app_config().map_err(|e| map_core_error(&e))?;
    let current_sync = runtime
        .get_sync_settings()
        .map_err(|e| map_core_error(&e))?;
    let mut current = serde_json::to_value(AppConfigForDisk::from(&current_cfg))
        .map_err(|_| OperationError::standalone_new(ErrorCode::OperationFailed))?;
    current["openvpn"]["executablePath"] = json!(current_sync.openvpn_executable_path);
    current["openvpn"]["ovpnPath"] = json!(current_sync.openvpn_ovpn_path);
    current["boundary"]["cliPath"] = json!(current_sync.boundary_cli_path);
    protect_config_replacement(&current, proposed)
        .map_err(|e| OperationError::for_host(e.code, true))
}

fn arg_str<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn optional_non_empty_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn arg_u16(args: &Value, key: &str) -> Option<u16> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u16::try_from(n).ok())
}

fn dispatch_standalone<B: KeyringBackend + 'static>(
    runtime: &ProfileRuntime<B>,
    app_version: &str,
    handler: Handler,
    args: &Value,
    cancel: &CancelToken,
) -> Result<Value, CoreError> {
    match handler {
        Handler::BoundarySearchTargets => {
            let filter = args.get("filter").and_then(Value::as_str);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            let targets = runtime.search_boundary_targets(filter, limit)?;
            Ok(Value::Array(
                targets
                    .into_iter()
                    .map(|t| {
                        json!({
                            "id": t.id,
                            "name": t.name,
                            "targetType": t.target_type,
                            "description": t.description,
                            "address": t.address,
                        })
                    })
                    .collect(),
            ))
        }
        Handler::DetectOpenvpnExecutable => {
            Ok(json!({ "path": runtime.detect_openvpn()?.unwrap_or_default() }))
        }
        Handler::ServerSshPasswordSet => {
            runtime
                .set_server_ssh_password(arg_str(args, "serverId"), arg_str(args, "password"))?;
            Ok(json!({}))
        }
        Handler::ServerSshPasswordClear => {
            runtime.clear_server_ssh_password(arg_str(args, "serverId"))?;
            Ok(json!({}))
        }
        Handler::AppGetOverview => {
            let statuses = runtime.list_function_statuses()?;
            let active = statuses.iter().filter(|s| s.session_id.is_some()).count();
            Ok(json!({
                "appVersion": app_version,
                "activeFunctions": active,
                "knownFunctionCount": statuses.len(),
            }))
        }
        Handler::FunctionStatusList => {
            let statuses = runtime.list_function_statuses()?;
            Ok(json!(statuses
                .iter()
                .map(SafeFunctionStatus::from)
                .collect::<Vec<_>>()))
        }
        Handler::FunctionStatusGet => {
            let id = arg_str(args, "functionId");
            let status = runtime.get_function_status(id)?;
            Ok(json!(SafeFunctionStatus::from(&status)))
        }
        Handler::OpenvpnGetStatus => {
            let status = runtime.get_function_status(OPENVPN_FUNCTION_ID).ok();
            Ok(json!(status.as_ref().map(SafeFunctionStatus::from)))
        }
        Handler::BoundaryGetStatus => {
            let status = runtime.get_function_status(BOUNDARY_AUTH_FUNCTION_ID).ok();
            Ok(json!(status.as_ref().map(SafeFunctionStatus::from)))
        }
        Handler::ServersList => {
            let page = runtime.list_servers(Some(MAX_SERVER_LIST_LIMIT), None)?;
            Ok(json!(page.servers))
        }
        Handler::ServerGet => {
            let id = arg_str(args, "serverId");
            Ok(json!(runtime.get_server(id)?))
        }
        Handler::ConfigGetPaths => Ok(json!(runtime.get_config_paths()?)),
        Handler::OpenvpnStart => {
            let username = optional_non_empty_str(args, "username");
            let password = optional_non_empty_str(args, "password");
            Ok(json!(runtime.start_openvpn_with_overrides(
                username, password, cancel
            )?))
        }
        Handler::OpenvpnStop => {
            runtime.stop_openvpn(DEFAULT_STOP_TIMEOUT)?;
            Ok(json!({}))
        }
        Handler::BoundaryAuthStart => Ok(json!(runtime.start_boundary_auth_with_size(
            arg_u16(args, "cols"),
            arg_u16(args, "rows"),
            cancel,
        )?)),
        Handler::BoundaryAuthStop => {
            runtime.stop_boundary_auth(DEFAULT_STOP_TIMEOUT)?;
            Ok(json!({}))
        }
        Handler::ServerTunnelStart => Ok(json!(
            runtime.start_server_tunnel(arg_str(args, "serverId"), cancel)?
        )),
        Handler::ServerTunnelStop => {
            runtime.stop_server_tunnel(arg_str(args, "serverId"), DEFAULT_STOP_TIMEOUT)?;
            Ok(json!({}))
        }
        Handler::ServerSshStart => {
            let defaults = runtime.get_app_config()?.app;
            let cols = arg_u16(args, "cols").unwrap_or(defaults.default_terminal_cols);
            let rows = arg_u16(args, "rows").unwrap_or(defaults.default_terminal_rows);
            Ok(json!(runtime.start_server_ssh(
                arg_str(args, "serverId"),
                cols,
                rows,
                cancel,
            )?))
        }
        Handler::ServerSshStop => {
            runtime.stop_server_ssh(arg_str(args, "serverId"), DEFAULT_STOP_TIMEOUT)?;
            Ok(json!({}))
        }
        Handler::CaddyStart => Ok(json!(runtime.start_caddy_with_size(
            Some(arg_str(args, "folder")),
            arg_u16(args, "cols"),
            arg_u16(args, "rows"),
            cancel,
        )?)),
        Handler::CaddyStop => {
            runtime.stop_caddy(DEFAULT_STOP_TIMEOUT)?;
            Ok(json!({}))
        }
        Handler::ServerUpsert => {
            runtime.upsert_server(&args["server"], false, None)?;
            Ok(json!(
                runtime
                    .list_servers(Some(MAX_SERVER_LIST_LIMIT), None)?
                    .servers
            ))
        }
        Handler::ServerDelete => {
            runtime.delete_server(arg_str(args, "id"), false, None)?;
            Ok(json!(
                runtime
                    .list_servers(Some(MAX_SERVER_LIST_LIMIT), None)?
                    .servers
            ))
        }
        Handler::ConfigSave => {
            let saved = runtime.save_app_config_mcp(&args["config"])?;
            Ok(json!(SafeAppConfig::from(&saved)))
        }
        Handler::ConfigSavePartial => {
            let patched = runtime.patch_app_config_mcp(&args["patch"])?;
            Ok(json!(SafeAppConfig::from(&patched)))
        }
        Handler::SyncSettingsGet => {
            Ok(json!(SafeSyncSettings::from(&runtime.get_sync_settings()?)))
        }
        Handler::SyncSettingsSavePartial => {
            runtime.patch_sync_settings(&args["patch"], false, None)?;
            Ok(json!(SafeSyncSettings::from(&runtime.get_sync_settings()?)))
        }
        Handler::RemoteBrowserOpen => Ok(json!(runtime.open_remote_browser(
            arg_str(args, "serverId"),
            None,
            cancel,
        )?)),
        Handler::RemoteBrowserClose => {
            runtime.close_remote_browser(arg_str(args, "browserId"))?;
            Ok(json!({}))
        }
        Handler::RemoteBrowserListChildren => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or("/");
            Ok(json!(
                runtime.list_remote_children(arg_str(args, "browserId"), path)?
            ))
        }
        Handler::RemoteBrowserStatPath => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or("/");
            Ok(json!(
                runtime.stat_remote_path(arg_str(args, "browserId"), path)?
            ))
        }
        Handler::RemoteBrowserCreateDirectory => {
            runtime.create_remote_directory(arg_str(args, "browserId"), arg_str(args, "path"))?;
            Ok(json!({}))
        }
        Handler::RemoteBrowserDeletePath => {
            let recursive = args
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            runtime.delete_remote_path(
                arg_str(args, "browserId"),
                arg_str(args, "path"),
                recursive,
            )?;
            Ok(json!({}))
        }
        Handler::ScpUpload => {
            let recursive = args
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            runtime.scp_upload(
                arg_str(args, "serverId"),
                arg_str(args, "localPath"),
                arg_str(args, "remotePath"),
                recursive,
                cancel,
            )?;
            Ok(json!({ "result": "Transfer completed" }))
        }
        Handler::ScpDownload => {
            let recursive = args
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let local_target = arg_str(args, "localPath");
            let overwrite = Path::new(local_target).is_dir();
            runtime.scp_download(
                arg_str(args, "serverId"),
                arg_str(args, "remotePath"),
                local_target,
                recursive,
                overwrite,
                cancel,
            )?;
            Ok(json!({ "result": "Transfer completed" }))
        }
        Handler::ScpStartUpload => {
            let recursive = args
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let started = runtime.start_scp_upload(
                arg_str(args, "serverId"),
                arg_str(args, "localPath"),
                arg_str(args, "remotePath"),
                recursive,
                cancel,
            )?;
            Ok(json!({ "sessionId": started.session_id }))
        }
        Handler::ScpStartDownload => {
            let recursive = args
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let local_target = arg_str(args, "localPath");
            let overwrite = Path::new(local_target).is_dir();
            let started = runtime.start_scp_download(
                ScpDownloadRequest {
                    server_id: arg_str(args, "serverId"),
                    remote_path: arg_str(args, "remotePath"),
                    local_target,
                    recursive,
                    overwrite,
                },
                cancel,
            )?;
            Ok(json!({ "sessionId": started.session_id }))
        }
        Handler::ScpTransferList => {
            let list = runtime.list_transfers()?;
            Ok(json!(list
                .iter()
                .map(SafeTransfer::from)
                .collect::<Vec<_>>()))
        }
        Handler::ScpTransferGet => {
            let id = arg_str(args, "transferId");
            let found = runtime
                .list_transfers()?
                .into_iter()
                .find(|t| t.id == id)
                .ok_or_else(|| {
                    CoreError::not_found(
                        "transfer_not_found",
                        format!("Unknown transferId: {id}"),
                        "inspect_status",
                    )
                })?;
            Ok(json!(SafeTransfer::from(&found)))
        }
        Handler::ConfigSyncPush => {
            runtime.push_config_sync(Some(arg_str(args, "repoRoot")), false, None)?;
            Ok(json!({}))
        }
        Handler::ServerRdpStart => Err(CoreError::capability_unavailable(
            "interactive_host_required",
            "Operation is unavailable on the standalone stdio host",
            "open_desktop",
        )),
        Handler::ConfigSyncPull => Err(CoreError::capability_unavailable(
            "protected_import",
            "Operation requires interactive protected import review",
            "use_cli_import",
        )),
        Handler::ConfigGitPull | Handler::ConfigGitPush => Err(CoreError::capability_unavailable(
            "reviewed_git",
            "Operation requires interactive git review",
            "use_cli_git",
        )),
    }
}

fn cli_command_schemas() -> Vec<Value> {
    let mut commands = slice1_discovery_schemas();
    commands.extend(slice1_profile_schemas());
    commands.extend(slice2_tunnel_vpn_schemas());
    commands.extend(slice2_ssh_scp_schemas());
    commands.extend(slice3_config_backup_schemas());
    commands
}

fn slice1_discovery_schemas() -> Vec<Value> {
    vec![
        cmd_schema(
            "schema",
            "Print machine-readable command and MCP schemas",
            false,
            false,
            json!({"command": {"type": "string"}, "json": {"type": "boolean"}}),
        ),
        cmd_schema(
            "capabilities",
            "Inspect standalone host capabilities and profile readiness",
            false,
            false,
            json!({"noProfile": {"type": "boolean"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        cmd_schema(
            "status",
            "Inspect process-local runtime and function statuses",
            true,
            false,
            json!({"functionId": {"type": "string"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        cmd_schema(
            "mcp.serve",
            "Run MCP server over newline-delimited JSON-RPC stdio",
            true,
            false,
            json!({"stdio": {"type": "boolean"}}),
        ),
    ]
}

fn slice1_profile_schemas() -> Vec<Value> {
    vec![
        cmd_schema(
            "server.list",
            "List configured servers with bounded pagination",
            true,
            false,
            json!({
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_SERVER_LIST_LIMIT},
                "after": {"type": "string"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}
            }),
        ),
        cmd_schema(
            "server.get",
            "Fetch a single server definition by exact ID",
            true,
            false,
            json!({"serverId": {"type": "string", "minLength": 1}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        cmd_schema(
            "config.paths",
            "Show resolved profile config file paths",
            true,
            false,
            json!({"json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        cmd_schema(
            "settings.show",
            "Show redacted app config and local sync settings",
            true,
            false,
            json!({"json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
    ]
}

fn slice2_tunnel_vpn_schemas() -> Vec<Value> {
    vec![
        cmd_schema(
            "openvpn.detect",
            "Detect OpenVPN binary on the local host",
            false,
            false,
            json!({"json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        cmd_schema(
            "openvpn.start",
            "Run OpenVPN foreground session until stopped",
            true,
            true,
            json!({"foreground": {"type": "boolean"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        cmd_schema(
            "boundary.targets.search",
            "Search Boundary targets from configured CLI",
            true,
            false,
            json!({"filter": {"type": "string"}, "limit": {"type": "integer", "minimum": 1}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        cmd_schema(
            "boundary.auth",
            "Authenticate against Boundary using password or interactive browser flow",
            true,
            true,
            json!({"interactive": {"type": "boolean"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        cmd_schema(
            "caddy.start",
            "Run Caddy foreground file server until stopped",
            true,
            true,
            json!({"foreground": {"type": "boolean"}, "folder": {"type": "string"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
    ]
}

fn slice2_ssh_scp_schemas() -> Vec<Value> {
    vec![
        cmd_schema(
            "server.tunnel.start",
            "Run Boundary tunnel for a server in the foreground until stopped",
            true,
            true,
            json!({"serverId": {"type": "string", "minLength": 1}, "foreground": {"type": "boolean"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        cmd_schema(
            "server.ssh",
            "Open an interactive PTY SSH session to a server",
            true,
            true,
            json!({"serverId": {"type": "string", "minLength": 1}, "interactive": {"type": "boolean"}}),
        ),
        cmd_schema(
            "scp.upload",
            "Upload a local file or directory to a server via SFTP/SCP",
            true,
            true,
            json!({"serverId": {"type": "string", "minLength": 1}, "local": {"type": "string", "minLength": 1}, "remote": {"type": "string", "minLength": 1}, "recursive": {"type": "boolean"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        cmd_schema(
            "scp.download",
            "Download a remote file or directory from a server via SFTP/SCP",
            true,
            true,
            json!({"serverId": {"type": "string", "minLength": 1}, "remote": {"type": "string", "minLength": 1}, "local": {"type": "string", "minLength": 1}, "recursive": {"type": "boolean"}, "overwrite": {"type": "boolean"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
    ]
}

fn slice3_config_backup_schemas() -> Vec<Value> {
    let json_args = json!({
        "file": {"type": "string"},
        "stdin": {"type": "boolean"},
        "preview": {"type": "boolean"},
        "expectedRevision": {"type": "string"},
        "json": {"type": "boolean"},
        "nonInteractive": {"type": "boolean"}
    });
    let specs: &[(&str, &str, bool, bool, bool, bool, Value)] = &[
        (
            "completion",
            "Generate shell completion script for scpctl without opening a profile",
            false,
            false,
            false,
            true,
            json!({"shell": {"type": "string", "enum": ["bash", "zsh", "fish", "powershell"]}}),
        ),
        ("server.upsert", "Create or update a server record in the active CLI profile", true, false, true, true, json_args.clone()),
        (
            "server.delete",
            "Delete a server record and its scoped credentials from the active CLI profile",
            true,
            true,
            true,
            false,
            json!({"serverId": {"type": "string", "minLength": 1}, "preview": {"type": "boolean"}, "expectedRevision": {"type": "string"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        (
            "server.credentials.set",
            "Store a server SSH password in the profile-scoped keyring via file, stdin, or TTY",
            true,
            false,
            false,
            true,
            json!({"serverId": {"type": "string", "minLength": 1}, "secretFile": {"type": "string"}, "secretStdin": {"type": "boolean"}, "interactive": {"type": "boolean"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        (
            "server.credentials.clear",
            "Clear a server SSH password from the profile-scoped keyring",
            true,
            true,
            false,
            true,
            json!({"serverId": {"type": "string", "minLength": 1}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        (
            "server.rdp",
            "Launch the native OS RDP client for a Windows server with an interactive GUI session",
            true,
            false,
            false,
            false,
            json!({"serverId": {"type": "string", "minLength": 1}, "interactive": {"type": "boolean"}}),
        ),
        ("config.save", "Replace full shared app config from JSON file or stdin", true, false, true, true, json_args.clone()),
        ("config.patch", "Merge patch into shared app config from JSON file or stdin", true, false, true, true, json_args.clone()),
        (
            "config.secret.set",
            "Store a profile-scoped config secret in the OS keyring via file, stdin, or TTY",
            true,
            false,
            false,
            true,
            json!({"key": {"type": "string", "minLength": 1}, "secretFile": {"type": "string"}, "secretStdin": {"type": "boolean"}, "interactive": {"type": "boolean"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        (
            "config.secret.clear",
            "Clear a profile-scoped config secret from the OS keyring",
            true,
            true,
            false,
            true,
            json!({"key": {"type": "string", "minLength": 1}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        (
            "config.sync.push",
            "Write redacted app-config JSON files into the configured Git repo app-config directory",
            true,
            false,
            true,
            true,
            json!({"repoRoot": {"type": "string"}, "preview": {"type": "boolean"}, "expectedRevision": {"type": "string"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        ("settings.patch", "Patch local non-privileged CLI profile settings", true, false, true, true, json_args),
        (
            "settings.export",
            "Export an encrypted .scpbak archive using passphrase file, stdin, or interactive TTY",
            true,
            false,
            false,
            false,
            json!({"output": {"type": "string", "minLength": 1}, "overwrite": {"type": "boolean"}, "passphraseFile": {"type": "string"}, "passphraseStdin": {"type": "boolean"}, "interactive": {"type": "boolean"}, "json": {"type": "boolean"}, "nonInteractive": {"type": "boolean"}}),
        ),
        (
            "settings.import",
            "Preview or interactively apply an encrypted .scpbak archive into the active CLI profile",
            true,
            true,
            true,
            false,
            json!({
                "input": {"type": "string", "minLength": 1},
                "preview": {"type": "boolean"},
                "apply": {"type": "boolean"},
                "interactive": {"type": "boolean"},
                "passphraseFile": {"type": "string"},
                "passphraseStdin": {"type": "boolean"},
                "noShared": {"type": "boolean"},
                "noServers": {"type": "boolean"},
                "noLocal": {"type": "boolean"},
                "noSecrets": {"type": "boolean"},
                "useSourcePaths": {"type": "boolean"},
                "remap": {"type": "array", "items": {"type": "string"}},
                "json": {"type": "boolean"},
                "nonInteractive": {"type": "boolean"}
            }),
        ),
    ];
    specs
        .iter()
        .map(|(cmd, desc, prof, destr, prev, idem, args)| {
            let mut entry = cmd_schema(cmd, desc, *prof, *destr, args.clone());
            entry["previewSupported"] = json!(*prev);
            entry["idempotent"] = json!(*idem);
            entry
        })
        .collect()
}

fn cmd_schema(
    command: &str,
    description: &str,
    profile_required: bool,
    destructive: bool,
    args: Value,
) -> Value {
    json!({
        "command": command,
        "description": description,
        "profileRequired": profile_required,
        "available": true,
        "destructive": destructive,
        "previewSupported": false,
        "idempotent": !destructive,
        "arguments": args
    })
}

fn unavailability_reason(op: &Operation) -> Option<&'static str> {
    standalone_unavailable_reason(op)
}

pub fn standalone_schema_catalog(command_filter: Option<&str>) -> Result<Value, CoreError> {
    let mut commands = cli_command_schemas();
    if let Some(filter) = command_filter {
        commands.retain(|c| c["command"].as_str() == Some(filter));
        if commands.is_empty() {
            return Err(CoreError::invalid_arguments(
                "unknown_command",
                format!("Unknown command schema target: {filter}"),
            ));
        }
    }

    let mut operations = OPERATIONS.iter().collect::<Vec<_>>();
    operations.sort_by_key(|op| op.name);
    let tools: Vec<Value> = operations
        .iter()
        .map(|op| {
            let reason = unavailability_reason(op);
            json!({
                "name": op.name,
                "description": op.title,
                "category": op.category,
                "readOnly": op.read_only,
                "inputSchema": input_schema(op),
                "outputSchema": output_schema(op),
                "available": reason.is_none(),
                "reason": reason
            })
        })
        .collect();
    let mut resources: Vec<Value> = operations
        .iter()
        .filter_map(|op| {
            op.resource.map(|(uri, title, category)| {
                let reason = unavailability_reason(op);
                json!({
                    "uri": uri,
                    "name": title,
                    "category": category,
                    "operation": op.name,
                    "available": reason.is_none(),
                    "reason": reason
                })
            })
        })
        .collect();
    resources.sort_by(|a, b| a["uri"].as_str().cmp(&b["uri"].as_str()));

    Ok(json!({
        "commands": commands,
        "mcpOperations": tools,
        "mcpResources": resources
    }))
}

pub fn capabilities_report<B: KeyringBackend>(
    runtime: Option<&ProfileRuntime<B>>,
) -> Result<Value, CoreError> {
    let health = match runtime {
        Some(rt) => rt.keyring_health(),
        None => preflight_keyring(),
    };
    let (keyring_state, keyring_detail) = match health {
        KeyringHealth::Ready => ("ready", Value::Null),
        KeyringHealth::Locked => ("locked", Value::String("OS keyring is locked".into())),
        KeyringHealth::Unavailable(msg) => ("unavailable", Value::String(msg)),
    };

    let (profile_open, profile_info, stdio_policy, permitted) = match runtime {
        Some(rt) => {
            let paths = rt.profile_paths();
            let info = json!({
                "runtimeId": rt.runtime_id(),
                "profileName": paths.profile_name,
                "profileDir": paths.profile_dir,
                "credentialNamespace": rt.credential_namespace(),
            });
            match rt.reload_stdio_policy() {
                Ok(policy) => {
                    let names: Vec<&'static str> = permitted_standalone_operations(&policy)
                        .into_iter()
                        .map(|op| op.name)
                        .collect();
                    (true, info, json!(policy), names)
                }
                Err(err) if err.code == "POLICY_DENIED" => (
                    true,
                    info,
                    json!({
                        "sensitiveActionPolicy": "invalid",
                        "allowedToolCategories": [],
                    }),
                    Vec::new(),
                ),
                Err(err) => return Err(err),
            }
        }
        None => {
            let default_policy = StdioPolicy::default();
            let names: Vec<&'static str> = permitted_standalone_operations(&default_policy)
                .into_iter()
                .map(|op| op.name)
                .collect();
            (false, Value::Null, json!(default_policy), names)
        }
    };

    Ok(json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "profileOpen": profile_open,
        "profile": profile_info,
        "keyring": {
            "state": keyring_state,
            "detail": keyring_detail,
        },
        "stdioPolicy": stdio_policy,
        "availableOperations": permitted,
        "runtimeFamilies": {
            "openvpn": "cli_foreground",
            "boundary": "cli_foreground_and_query",
            "caddy": "cli_foreground",
            "ssh": "cli_interactive_pty",
            "scp": "cli_one_shot",
            "sftp": "stdio_persistent_browser",
            "config": "cli_local",
            "backup": "cli_local",
            "rdp": "cli_interactive_gui"
        },
        "unavailableFamilies": {}
    }))
}
