use super::validation::validate_arguments;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    PolicyDenied,
    InvalidArguments,
    CapabilityUnavailable,
    NotFound,
    Conflict,
    Cancelled,
    Timeout,
    OperationFailed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationError {
    pub code: ErrorCode,
    pub message: &'static str,
    pub retryable: bool,
}
impl OperationError {
    pub fn for_host(code: ErrorCode, standalone: bool) -> Self {
        let message = match (code, standalone) {
            (ErrorCode::PolicyDenied, false) => "MCP policy denied this operation. Use local desktop settings or controls.",
            (ErrorCode::PolicyDenied, true) => "MCP policy denied this operation. Configure local stdio policy to allow permitted operations.",
            (ErrorCode::InvalidArguments, _) => "Invalid MCP arguments or protected configuration fields.",
            (ErrorCode::CapabilityUnavailable, false) => "This MCP operation is unavailable on this host or requires local setup/review. Check the local desktop, platform/client support and server configuration.",
            (ErrorCode::CapabilityUnavailable, true) => "This MCP operation is unavailable on this standalone host or requires local setup.",
            (ErrorCode::NotFound, _) => "Requested item was not found.",
            (ErrorCode::Conflict, _) => "Operation conflicts with current state. Query state before retrying.",
            (ErrorCode::Cancelled, _) => "Operation was cancelled. Query state before retrying.",
            (ErrorCode::Timeout, _) => "Operation timed out. Query state before retrying.",
            (ErrorCode::OperationFailed, false) => "Operation failed. Inspect the local desktop for details; query state before retrying.",
            (ErrorCode::OperationFailed, true) => "Operation failed on this standalone host; query state before retrying.",
        };
        Self {
            code,
            message,
            retryable: standalone && matches!(code, ErrorCode::Timeout),
        }
    }

    pub fn new(code: ErrorCode) -> Self {
        Self::for_host(code, false)
    }

    pub fn standalone_new(code: ErrorCode) -> Self {
        Self::for_host(code, true)
    }

    pub fn legacy_message(&self) -> String {
        format!(
            "{}: {}",
            serde_json::to_value(self.code).unwrap().as_str().unwrap(),
            self.message
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Runtime,
    InteractiveHost,
    ProtectedImport,
    ReviewedGit,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime {
    Query,
    Request,
    HostOwned,
    Stop,
    Mutation,
}
#[derive(Debug, Clone, Copy)]
pub enum Handler {
    BoundarySearchTargets,
    DetectOpenvpnExecutable,
    ServerRdpStart,
    ServerSshPasswordSet,
    ServerSshPasswordClear,
    AppGetOverview,
    FunctionStatusList,
    FunctionStatusGet,
    OpenvpnGetStatus,
    BoundaryGetStatus,
    ServersList,
    ServerGet,
    ConfigGetPaths,
    OpenvpnStart,
    OpenvpnStop,
    BoundaryAuthStart,
    BoundaryAuthStop,
    ServerTunnelStart,
    ServerTunnelStop,
    ServerSshStart,
    ServerSshStop,
    CaddyStart,
    CaddyStop,
    ServerUpsert,
    ServerDelete,
    ConfigSave,
    ConfigSavePartial,
    SyncSettingsGet,
    SyncSettingsSavePartial,
    RemoteBrowserOpen,
    RemoteBrowserClose,
    RemoteBrowserListChildren,
    RemoteBrowserStatPath,
    RemoteBrowserCreateDirectory,
    RemoteBrowserDeletePath,
    ScpUpload,
    ScpDownload,
    ScpStartUpload,
    ScpStartDownload,
    ScpTransferList,
    ScpTransferGet,
    ConfigSyncPull,
    ConfigSyncPush,
    ConfigGitPull,
    ConfigGitPush,
}

#[derive(Debug)]
pub struct Operation {
    pub handler: Handler,
    pub name: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub read_only: bool,
    pub capability: Capability,
    #[allow(dead_code)] // Transport ownership enforcement is implemented in the lifecycle slice.
    pub lifetime: Lifetime,
    // URI, legacy resource title/category. Authorization always uses the operation category.
    pub resource: Option<(&'static str, &'static str, &'static str)>,
}
pub static OPERATIONS: &[Operation] = &[
    Operation {
        handler: Handler::BoundarySearchTargets,
        name: "boundary_search_targets",
        title: "Search Boundary targets (check local Boundary authentication on failure)",
        category: "read-only",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: None,
    },
    Operation {
        handler: Handler::DetectOpenvpnExecutable,
        name: "detect_openvpn_executable",
        title: "Detect OpenVPN executable",
        category: "read-only",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: None,
    },
    Operation {
        handler: Handler::ServerRdpStart,
        name: "server_rdp_start",
        title: "Launch RDP on the interactive desktop (requires an installed client)",
        category: "runtime-control",
        read_only: false,
        capability: Capability::InteractiveHost,
        lifetime: Lifetime::HostOwned,
        resource: None,
    },
    Operation {
        handler: Handler::ServerSshPasswordSet,
        name: "server_ssh_password_set",
        title: "Set SSH password in OS keyring (write-only)",
        category: "config-mutation",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Mutation,
        resource: None,
    },
    Operation {
        handler: Handler::ServerSshPasswordClear,
        name: "server_ssh_password_clear",
        title: "Clear SSH password from OS keyring",
        category: "config-mutation",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Mutation,
        resource: None,
    },
    Operation {
        handler: Handler::AppGetOverview,
        name: "app_get_overview",
        title: "App overview",
        category: "read-only",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: None,
    },
    Operation {
        handler: Handler::FunctionStatusList,
        name: "function_status_list",
        title: "List function statuses",
        category: "read-only",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: Some(("scp://status/functions", "Function statuses", "status")),
    },
    Operation {
        handler: Handler::FunctionStatusGet,
        name: "function_status_get",
        title: "Get one function status",
        category: "read-only",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: None,
    },
    Operation {
        handler: Handler::OpenvpnGetStatus,
        name: "openvpn_get_status",
        title: "OpenVPN status",
        category: "read-only",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: Some(("scp://status/openvpn", "OpenVPN status", "status")),
    },
    Operation {
        handler: Handler::BoundaryGetStatus,
        name: "boundary_get_status",
        title: "Boundary status",
        category: "read-only",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: Some(("scp://status/boundary", "Boundary status", "status")),
    },
    Operation {
        handler: Handler::ServersList,
        name: "servers_list",
        title: "List servers",
        category: "read-only",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: Some(("scp://servers/list", "Servers list", "inventory")),
    },
    Operation {
        handler: Handler::ServerGet,
        name: "server_get",
        title: "Get server",
        category: "read-only",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: None,
    },
    Operation {
        handler: Handler::ConfigGetPaths,
        name: "config_get_paths",
        title: "Get config paths",
        category: "read-only",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: Some(("scp://config/paths", "Config paths", "config")),
    },
    Operation {
        handler: Handler::OpenvpnStart,
        name: "openvpn_start",
        title: "Start OpenVPN",
        category: "runtime-control",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::HostOwned,
        resource: None,
    },
    Operation {
        handler: Handler::OpenvpnStop,
        name: "openvpn_stop",
        title: "Stop OpenVPN",
        category: "runtime-control",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Stop,
        resource: None,
    },
    Operation {
        handler: Handler::BoundaryAuthStart,
        name: "boundary_auth_start",
        title: "Start Boundary auth",
        category: "runtime-control",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::HostOwned,
        resource: None,
    },
    Operation {
        handler: Handler::BoundaryAuthStop,
        name: "boundary_auth_stop",
        title: "Stop Boundary auth",
        category: "runtime-control",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Stop,
        resource: None,
    },
    Operation {
        handler: Handler::ServerTunnelStart,
        name: "server_tunnel_start",
        title: "Start server tunnel",
        category: "runtime-control",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::HostOwned,
        resource: None,
    },
    Operation {
        handler: Handler::ServerTunnelStop,
        name: "server_tunnel_stop",
        title: "Stop server tunnel",
        category: "runtime-control",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Stop,
        resource: None,
    },
    Operation {
        handler: Handler::ServerSshStart,
        name: "server_ssh_start",
        title: "Start server SSH",
        category: "runtime-control",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::HostOwned,
        resource: None,
    },
    Operation {
        handler: Handler::ServerSshStop,
        name: "server_ssh_stop",
        title: "Stop server SSH",
        category: "runtime-control",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Stop,
        resource: None,
    },
    Operation {
        handler: Handler::CaddyStart,
        name: "caddy_start",
        title: "Start Caddy",
        category: "runtime-control",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::HostOwned,
        resource: None,
    },
    Operation {
        handler: Handler::CaddyStop,
        name: "caddy_stop",
        title: "Stop Caddy",
        category: "runtime-control",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Stop,
        resource: None,
    },
    Operation {
        handler: Handler::ServerUpsert,
        name: "server_upsert",
        title: "Create or update server",
        category: "config-mutation",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Mutation,
        resource: None,
    },
    Operation {
        handler: Handler::ServerDelete,
        name: "server_delete",
        title: "Delete server",
        category: "config-mutation",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Mutation,
        resource: None,
    },
    Operation {
        handler: Handler::ConfigSave,
        name: "config_save",
        title: "Save full app config",
        category: "config-mutation",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Mutation,
        resource: None,
    },
    Operation {
        handler: Handler::ConfigSavePartial,
        name: "config_save_partial",
        title: "Patch app config",
        category: "config-mutation",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Mutation,
        resource: None,
    },
    Operation {
        handler: Handler::SyncSettingsGet,
        name: "sync_settings_get",
        title: "Get device-local sync settings",
        category: "read-only",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: None,
    },
    Operation {
        handler: Handler::SyncSettingsSavePartial,
        name: "sync_settings_save_partial",
        title: "Patch device-local sync settings",
        category: "config-mutation",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Mutation,
        resource: None,
    },
    Operation {
        handler: Handler::RemoteBrowserOpen,
        name: "remote_browser_open",
        title: "Open remote browser session",
        category: "file-transfer",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::HostOwned,
        resource: None,
    },
    Operation {
        handler: Handler::RemoteBrowserClose,
        name: "remote_browser_close",
        title: "Close remote browser session",
        category: "file-transfer",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Stop,
        resource: None,
    },
    Operation {
        handler: Handler::RemoteBrowserListChildren,
        name: "remote_browser_list_children",
        title: "List remote browser children",
        category: "file-transfer",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: None,
    },
    Operation {
        handler: Handler::RemoteBrowserStatPath,
        name: "remote_browser_stat_path",
        title: "Stat remote browser path",
        category: "file-transfer",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: None,
    },
    Operation {
        handler: Handler::RemoteBrowserCreateDirectory,
        name: "remote_browser_create_directory",
        title: "Create remote directory",
        category: "file-transfer",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Request,
        resource: None,
    },
    Operation {
        handler: Handler::RemoteBrowserDeletePath,
        name: "remote_browser_delete_path",
        title: "Delete remote path",
        category: "file-transfer",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Request,
        resource: None,
    },
    Operation {
        handler: Handler::ScpUpload,
        name: "scp_upload",
        title: "Run SCP upload",
        category: "file-transfer",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Request,
        resource: None,
    },
    Operation {
        handler: Handler::ScpDownload,
        name: "scp_download",
        title: "Run SCP download",
        category: "file-transfer",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Request,
        resource: None,
    },
    Operation {
        handler: Handler::ScpStartUpload,
        name: "scp_start_upload",
        title: "Start PTY-backed SCP upload",
        category: "file-transfer",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::HostOwned,
        resource: None,
    },
    Operation {
        handler: Handler::ScpStartDownload,
        name: "scp_start_download",
        title: "Start PTY-backed SCP download",
        category: "file-transfer",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::HostOwned,
        resource: None,
    },
    Operation {
        handler: Handler::ScpTransferList,
        name: "scp_transfer_list",
        title: "List SCP transfers",
        category: "file-transfer",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: Some((
            "scp://status/transfers",
            "SCP transfer status",
            "file-transfer",
        )),
    },
    Operation {
        handler: Handler::ScpTransferGet,
        name: "scp_transfer_get",
        title: "Get one SCP transfer",
        category: "file-transfer",
        read_only: true,
        capability: Capability::Runtime,
        lifetime: Lifetime::Query,
        resource: None,
    },
    Operation {
        handler: Handler::ConfigSyncPull,
        name: "config_sync_pull",
        title: "Pull synced config from repo",
        category: "config-mutation",
        read_only: false,
        capability: Capability::ProtectedImport,
        lifetime: Lifetime::Mutation,
        resource: None,
    },
    Operation {
        handler: Handler::ConfigSyncPush,
        name: "config_sync_push",
        title: "Push synced config to repo",
        category: "config-mutation",
        read_only: false,
        capability: Capability::Runtime,
        lifetime: Lifetime::Mutation,
        resource: None,
    },
    Operation {
        handler: Handler::ConfigGitPull,
        name: "config_git_pull",
        title: "Run git pull for config repo",
        category: "config-mutation",
        read_only: false,
        capability: Capability::ReviewedGit,
        lifetime: Lifetime::Mutation,
        resource: None,
    },
    Operation {
        handler: Handler::ConfigGitPush,
        name: "config_git_push",
        title: "Run git add/commit/push for config repo",
        category: "config-mutation",
        read_only: false,
        capability: Capability::ReviewedGit,
        lifetime: Lifetime::Mutation,
        resource: None,
    },
];
pub fn operation(name: &str) -> Option<&'static Operation> {
    OPERATIONS.iter().find(|op| op.name == name)
}
pub fn resource_operation(uri: &str) -> Option<&'static Operation> {
    OPERATIONS
        .iter()
        .find(|op| op.resource.is_some_and(|resource| resource.0 == uri))
}
pub fn is_valid_category(category: &str) -> bool {
    matches!(
        category,
        "read-only" | "runtime-control" | "config-mutation" | "file-transfer"
    )
}

/// Returns whether `op` is available on the standalone CLI stdio MCP host.
pub fn is_standalone_stdio_available(op: &Operation) -> bool {
    matches!(op.capability, Capability::Runtime)
}

/// Returns the machine-readable unavailability reason on the standalone host, if any.
pub fn standalone_unavailable_reason(op: &Operation) -> Option<&'static str> {
    match op.capability {
        Capability::Runtime => None,
        Capability::InteractiveHost => Some("interactive_host_required"),
        Capability::ProtectedImport => Some("protected_import"),
        Capability::ReviewedGit => Some("reviewed_git"),
    }
}

pub fn map_core_error(err: &scp_core::error::CoreError) -> OperationError {
    let code = match err.code {
        "POLICY_DENIED" => ErrorCode::PolicyDenied,
        "INVALID_ARGUMENTS" => ErrorCode::InvalidArguments,
        "CAPABILITY_UNAVAILABLE" => ErrorCode::CapabilityUnavailable,
        "NOT_FOUND" => ErrorCode::NotFound,
        "CONFLICT" => ErrorCode::Conflict,
        "TIMEOUT" => ErrorCode::Timeout,
        "INTERRUPTED" => ErrorCode::Cancelled,
        _ => ErrorCode::OperationFailed,
    };
    OperationError::standalone_new(code)
}

fn authorize_checked(
    name: &str,
    read_only_only: bool,
    valid_policy: bool,
    categories: &[&str],
    standalone: bool,
) -> Result<&'static Operation, OperationError> {
    let mk_err = |code| OperationError::for_host(code, standalone);
    let op = operation(name).ok_or_else(|| mk_err(ErrorCode::NotFound))?;
    if !valid_policy
        || categories.iter().any(|c| !is_valid_category(c))
        || !categories.contains(&op.category)
        || (read_only_only && !op.read_only)
    {
        return Err(mk_err(ErrorCode::PolicyDenied));
    }
    let available = match op.capability {
        Capability::Runtime => true,
        Capability::InteractiveHost if !standalone => cfg!(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "linux"
        )),
        Capability::InteractiveHost | Capability::ProtectedImport | Capability::ReviewedGit => {
            false
        }
    };
    if !available {
        return Err(mk_err(ErrorCode::CapabilityUnavailable));
    }
    Ok(op)
}

pub fn authorize(
    name: &str,
    policy: &str,
    categories: &[&str],
) -> Result<&'static Operation, OperationError> {
    authorize_checked(
        name,
        policy == "require-ui-approval",
        matches!(policy, "allow-with-token" | "require-ui-approval"),
        categories,
        false,
    )
}

pub fn authorize_standalone(
    name: &str,
    policy: &scp_core::models::StdioPolicy,
) -> Result<&'static Operation, OperationError> {
    let cats: Vec<&str> = policy
        .allowed_tool_categories
        .iter()
        .map(String::as_str)
        .collect();
    authorize_checked(
        name,
        policy.sensitive_action_policy == "read-only",
        matches!(
            policy.sensitive_action_policy.as_str(),
            "read-only" | "allow-local-policy"
        ),
        &cats,
        true,
    )
}

/// Authorize and validate before the trusted host can touch stores or invoke actions.
pub fn execute<T>(
    name: &str,
    policy: &str,
    categories: &[&str],
    args: &Value,
    host: impl FnOnce(&Operation) -> Result<T, String>,
) -> Result<T, OperationError> {
    let op = authorize(name, policy, categories)?;
    validate_arguments(op, args)?;
    host(op).map_err(classify_host_error)
}

pub fn classify_host_error(message: String) -> OperationError {
    for code in [
        ErrorCode::PolicyDenied,
        ErrorCode::InvalidArguments,
        ErrorCode::CapabilityUnavailable,
        ErrorCode::NotFound,
        ErrorCode::Conflict,
        ErrorCode::Cancelled,
        ErrorCode::Timeout,
    ] {
        if message == OperationError::new(code).legacy_message() {
            return OperationError::new(code);
        }
    }
    let code = match message.as_str() {
        "function status not found" | "server not found" | "transfer not found" => {
            ErrorCode::NotFound
        }
        _ => ErrorCode::OperationFailed,
    };
    OperationError::new(code)
}

/// Audit metadata only: even innocuous-looking argument values can carry credentials.
pub fn audit_summary(kind: &str, name: &str, _args: &Value) -> String {
    let name = operation(name)
        .map(|op| op.name)
        .or_else(|| resource_operation(name).map(|op| op.name))
        .unwrap_or("unknown");
    let kind = if kind == "resource.read" {
        "resource.read"
    } else {
        "tool.call"
    };
    format!("[MCP audit] kind={kind} name={name}")
}

pub fn redact_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let key: String = key
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect();
                if key != "boundarytokensource"
                    && [
                        "password",
                        "secret",
                        "token",
                        "authorization",
                        "cookie",
                        "credential",
                    ]
                    .iter()
                    .any(|part| key.contains(part))
                {
                    *value = json!("[redacted]");
                } else {
                    redact_fields(value);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_fields(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CATEGORIES: &[&str] = &[
        "read-only",
        "runtime-control",
        "config-mutation",
        "file-transfer",
    ];

    #[test]
    fn policy_fails_closed_and_resources_share_authorization() {
        for policy in ["", "ALLOW", "allow-with-tokne"] {
            assert_eq!(
                authorize("function_status_list", policy, CATEGORIES)
                    .unwrap_err()
                    .code,
                ErrorCode::PolicyDenied
            );
        }
        assert!(authorize(
            "function_status_list",
            "allow-with-token",
            &["bogus", "read-only"]
        )
        .is_err());
        assert!(authorize("unknown", "allow-with-token", CATEGORIES).is_err());
        assert!(authorize("servers_list", "allow-with-token", &[]).is_err());
        for op in OPERATIONS {
            assert_eq!(
                authorize(op.name, "require-ui-approval", CATEGORIES).is_ok(),
                op.read_only && matches!(op.capability, Capability::Runtime),
                "{}",
                op.name
            );
        }
        let op = resource_operation("scp://servers/list").unwrap();
        assert_eq!(op.name, "servers_list");
        assert!(authorize(op.name, "allow-with-token", &["file-transfer"]).is_err());
        assert_eq!(OPERATIONS.len(), 45);
        assert_eq!(
            OPERATIONS
                .iter()
                .map(|op| op.name)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            45
        );
    }

    #[test]
    fn protected_or_invalid_mutations_never_reach_host() {
        let cases = [
            (
                "sync_settings_save_partial",
                json!({"patch":{"mcpEnabled":true,"repoRoot":"ok"}}),
            ),
            (
                "sync_settings_save_partial",
                json!({"patch":{"ccAgentToken":"secret"}}),
            ),
            (
                "sync_settings_save_partial",
                json!({"patch":{"firstRunCompleted":true}}),
            ),
            (
                "sync_settings_save_partial",
                json!({"patch":{"boundaryCliPath":"evil"}}),
            ),
            ("sync_settings_save_partial", json!({"patch":{"typo":true}})),
            (
                "config_save_partial",
                json!({"patch":{"boundary":{"cliPath":"evil"}}}),
            ),
            (
                "config_save_partial",
                json!({"patch":{"openvpn":{"extraArgs":"--script evil"}}}),
            ),
            (
                "config_save_partial",
                json!({"patch":{"cc":{"enabled":true}}}),
            ),
            ("config_save_partial", json!({"patch":null})),
            ("server_tunnel_start", json!({"serverId":"x","cols":65536})),
            ("server_tunnel_start", json!({"serverId":"x","rows":"bad"})),
            (
                "remote_browser_delete_path",
                json!({"browserId":"x","path":"/","recursive":"yes"}),
            ),
            ("config_git_pull", json!({"repoRoot":"repo"})),
            ("config_git_push", json!({"repoRoot":"repo"})),
            ("config_sync_pull", json!({"repoRoot":"repo"})),
        ];
        for (name, args) in cases {
            let mut called = false;
            assert!(
                execute(name, "allow-with-token", CATEGORIES, &args, |_| {
                    called = true;
                    Ok(())
                })
                .is_err(),
                "{name}"
            );
            assert!(!called, "denied call reached host: {name}");
        }
        execute(
            "config_save_partial",
            "allow-with-token",
            CATEGORIES,
            &json!({"patch":{"app":{"defaultTerminalCols":100}}}),
            |_| Ok(()),
        )
        .unwrap();
        execute(
            "sync_settings_save_partial",
            "allow-with-token",
            CATEGORIES,
            &json!({"patch":{"repoRoot":"repo"}}),
            |_| Ok(()),
        )
        .unwrap();
    }

    #[test]
    fn no_untrusted_text_in_errors_or_audit() {
        let secret = "unlabelled-secret-123";
        assert!(!audit_summary("tool.call", secret, &json!({"innocent":secret})).contains(secret));
        let err = execute::<()>(
            "function_status_list",
            "allow-with-token",
            CATEGORIES,
            &json!({}),
            |_| Err(secret.into()),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::OperationFailed);
        assert!(!serde_json::to_string(&err).unwrap().contains(secret));
        let mut value = json!({"items":[{"Authorization":secret,"cc_agent_token":secret,"password":secret}],"safe":1});
        redact_fields(&mut value);
        assert!(!value.to_string().contains(secret));
        assert_eq!(value["safe"], 1);
    }
}
