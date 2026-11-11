//! Wire DTOs for the existing custom MCP adapters.
//!
//! These types carry values only: hosts retain result construction, policy,
//! clock and transport behavior. They do not define standard MCP results.

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallEnvelope {
    pub ok: bool,
    pub message: String,
    pub data: serde_json::Value,
    pub warnings: Vec<String>,
    pub timestamp: u64,
}

/// Existing `function_status_get` arguments; intentionally no trimming or ID validation.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpFunctionStatusGetArgs<'a> {
    pub function_id: &'a str,
}

impl<'a> McpFunctionStatusGetArgs<'a> {
    /// Match the custom dispatcher's permissive string lookup, including its error.
    /// Extra fields are ignored. Non-object, missing and non-string values fail alike.
    pub fn from_args(args: &'a serde_json::Value) -> Result<Self, String> {
        Ok(Self {
            function_id: required_string_arg(args, "functionId")?,
        })
    }
}

/// Bare status object, with the runtime model's exact fields and null semantics.
pub type McpFunctionStatusGetResult = crate::runtime::FunctionStatus;

/// Bare array in host-supplied iteration order; no sorting or filtering.
pub type McpFunctionStatusListResult = Vec<McpFunctionStatusGetResult>;

/// Bare `openvpn_get_status` status or null when the host's fixed-ID lookup is absent.
pub type McpOpenVpnGetStatusResult = Option<crate::runtime::FunctionStatus>;

/// Bare `boundary_get_status` status or null when the host's fixed-ID lookup is absent.
pub type McpBoundaryGetStatusResult = Option<crate::runtime::FunctionStatus>;

/// Bare `app_get_overview` result; hosts supply version and store-derived counts.
/// Active functions count statuses with any session ID, independently of state.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppOverviewResult {
    pub app_version: String,
    pub active_functions: usize,
    pub known_function_count: usize,
}

/// Existing `server_get` arguments; no trimming, normalization or ID validation.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerGetArgs<'a> {
    pub server_id: &'a str,
}

impl<'a> McpServerGetArgs<'a> {
    /// Preserve the dispatcher's string lookup and error; ignore extra fields.
    pub fn from_args(args: &'a serde_json::Value) -> Result<Self, String> {
        Ok(Self {
            server_id: required_string_arg(args, "serverId")?,
        })
    }
}

/// Bare inventory record, with the shared server model's exact fields.
pub type McpServerGetResult = crate::servers::Server;

/// Bare array in host loader order, with no filtering, sorting or deduplication.
pub type McpServersListResult = Vec<McpServerGetResult>;

/// Bare `config_get_paths` object; desktop hosts retain path resolution and errors.
pub type McpConfigGetPathsResult = crate::config::ConfigPaths;

/// Existing transfer lookup arguments; any string is accepted without normalization.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpScpTransferGetArgs<'a> {
    pub transfer_id: &'a str,
}

impl<'a> McpScpTransferGetArgs<'a> {
    /// Preserve the host's string lookup and required error; ignore extra fields.
    pub fn from_args(args: &'a serde_json::Value) -> Result<Self, String> {
        Ok(Self {
            transfer_id: required_string_arg(args, "transferId")?,
        })
    }
}

/// Bare stored transfer, with the model's exact fields and null semantics.
pub type McpScpTransferGetResult = crate::scp_transfer::ScpTransferEntry;

/// Bare transfer array in host store order; no sorting, filtering or deduplication.
pub type McpScpTransferListResult = Vec<McpScpTransferGetResult>;

/// Shared arguments for `remote_browser_list_children` and `remote_browser_stat_path`.
/// Hosts retain session lookup and path normalization after this permissive lookup.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRemoteBrowserReadArgs<'a> {
    pub browser_id: &'a str,
    pub path: &'a str,
}

impl<'a> McpRemoteBrowserReadArgs<'a> {
    /// Any browser ID string is accepted; absent/non-string paths become empty.
    pub fn from_args(args: &'a serde_json::Value) -> Result<Self, String> {
        Ok(Self {
            browser_id: required_string_arg(args, "browserId")?,
            path: args
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
        })
    }
}

/// Bare entries in host enumeration order, without sorting or filtering here.
pub type McpRemoteBrowserListChildrenResult = Vec<crate::remote_browser::RemoteBrowseEntry>;

/// Bare node information; kind and existence are host-supplied values.
pub type McpRemoteBrowserStatPathResult = crate::remote_browser::RemoteBrowseNodeInfo;

/// Existing browser-open lookup; the host retains allocation and session replacement.
/// Reuse the identical server-ID wire contract, including permissive parsing/errors.
pub type McpRemoteBrowserOpenArgs<'a> = McpServerGetArgs<'a>;

/// Bare host-allocated browser handle and resolved root path.
pub type McpRemoteBrowserOpenResult = crate::remote_browser::RemoteBrowseSession;

/// Existing close lookup; no validation or normalization of browser IDs.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRemoteBrowserCloseArgs<'a> {
    pub browser_id: &'a str,
}

impl<'a> McpRemoteBrowserCloseArgs<'a> {
    /// Preserve the required error; hosts retain lookup, cleanup and missing-ID behavior.
    pub fn from_args(args: &'a serde_json::Value) -> Result<Self, String> {
        required_string_arg(args, "browserId").map(|browser_id| Self { browser_id })
    }
}

/// Successful custom MCP close returns `{}`, not null.
#[derive(Serialize)]
pub struct McpRemoteBrowserCloseResult {}

/// Required browser and path strings for `remote_browser_create_directory`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRemoteBrowserCreateDirectoryArgs<'a> {
    pub browser_id: &'a str,
    pub path: &'a str,
}

impl<'a> McpRemoteBrowserCreateDirectoryArgs<'a> {
    /// Preserve browser-before-path errors; hosts retain path validation and SFTP.
    pub fn from_args(args: &'a serde_json::Value) -> Result<Self, String> {
        Ok(Self {
            browser_id: required_string_arg(args, "browserId")?,
            path: required_string_arg(args, "path")?,
        })
    }
}

/// Existing delete arguments; non-boolean recursion values remain absent.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRemoteBrowserDeletePathArgs<'a> {
    pub browser_id: &'a str,
    pub path: &'a str,
    pub recursive: Option<bool>,
}

impl<'a> McpRemoteBrowserDeletePathArgs<'a> {
    pub fn from_args(args: &'a serde_json::Value) -> Result<Self, String> {
        let parsed = McpRemoteBrowserCreateDirectoryArgs::from_args(args)?;
        Ok(Self {
            browser_id: parsed.browser_id,
            path: parsed.path,
            recursive: args.get("recursive").and_then(serde_json::Value::as_bool),
        })
    }
}

/// Successful create/delete use the same empty-object wire contract as close.
pub type McpRemoteBrowserCreateDirectoryResult = McpRemoteBrowserCloseResult;
pub type McpRemoteBrowserDeletePathResult = McpRemoteBrowserCloseResult;

fn required_string_arg<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("{key} is required"))
}
