# SuperControlPanel MCP PR1 Rust Scaffolding

## Purpose

This document defines the first concrete refactor PR for preparing the Rust backend for MCP work.

PR1 should be intentionally small:

- extract shared model types from `src-tauri/src/main.rs`
- add module wiring
- avoid behavior changes

## PR1 goal

Create a `models/` module tree and move type definitions there with minimal or no logic changes.

## PR1 status

Completed.

- `src-tauri/src/models/` was created
- model types were moved out of `src-tauri/src/main.rs`
- `main.rs` now imports the extracted model modules
- `cargo check` passed after the extraction

## PR1 non-goals

- no service extraction yet
- no MCP implementation yet
- no Tauri command renaming
- no event contract changes
- no store behavior changes

## Files to create in PR1

```text
src-tauri/src/models/
├─ mod.rs
├─ function_status.rs
├─ config.rs
├─ boundary.rs
└─ remote_browser.rs
```

## Suggested `src-tauri/src/models/mod.rs`

```rust
pub mod boundary;
pub mod config;
pub mod function_status;
pub mod remote_browser;

pub use boundary::*;
pub use config::*;
pub use function_status::*;
pub use remote_browser::*;
```

## Suggested `src-tauri/src/models/function_status.rs`

Move these types here first:

- `PtyOutputPayload`
- `PtyExitPayload`
- `FunctionStatus`
- `StartFunctionResult`
- `OpenVpnLogPayload`

Suggested initial shape:

```rust
use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PtyOutputPayload {
    pub id: String,
    pub data: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PtyExitPayload {
    pub id: String,
    pub exit_code: Option<i32>,
    pub reason: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FunctionStatus {
    pub function_id: String,
    pub state: String,
    pub message: String,
    pub boundary_token_source: Option<String>,
    pub session_id: Option<String>,
    pub updated_at_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StartFunctionResult {
    pub function_id: String,
    pub session_id: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnLogPayload {
    pub function_id: String,
    pub session_id: String,
    pub data: String,
}
```

## Suggested `src-tauri/src/models/boundary.rs`

Move these types here:

- `BoundaryTargetSummary`

Suggested initial shape:

```rust
use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryTargetSummary {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target_type: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub address: String,
}
```

## Suggested `src-tauri/src/models/remote_browser.rs`

Move these types here:

- `RemotePathKind`
- `RemoteBrowseSession`
- `RemoteBrowseEntry`
- `RemoteBrowseNodeInfo`
- `SshEndpoint`

Suggested initial shape:

```rust
use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum RemotePathKind {
    File,
    Directory,
    Symlink,
    Other,
    Missing,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBrowseSession {
    pub browser_id: String,
    pub root_path: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBrowseEntry {
    pub name: String,
    pub path: String,
    pub kind: RemotePathKind,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBrowseNodeInfo {
    pub path: String,
    pub kind: RemotePathKind,
    pub exists: bool,
}

#[derive(Clone)]
pub struct SshEndpoint {
    pub user: String,
    pub host: String,
    pub port: u16,
}
```

## Suggested `src-tauri/src/models/config.rs`

Move these types here:

- `AppConfig`
- `BoundaryAuthKind`
- `BoundaryOidcLoginMode`
- `BoundaryConfig`
- `BoundaryConfigForDisk`
- `AppRuntimeConfig`
- `OpenVpnConfig`
- `OpenVpnLaunchConfig`
- `HelperStatusFile`
- `OpenVpnConfigForDisk`
- `AppConfigForDisk`

Suggested initial shape:

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub app: AppRuntimeConfig,
    #[serde(default)]
    pub openvpn: OpenVpnConfig,
    #[serde(default)]
    pub boundary: BoundaryConfig,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoundaryAuthKind {
    Password,
    #[default]
    Oidc,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoundaryOidcLoginMode {
    #[default]
    Browser,
    Automated,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryConfig {
    #[serde(default)]
    pub cli_path: String,
    #[serde(default)]
    pub addr: String,
    #[serde(default)]
    pub auth_method_id: String,
    #[serde(default)]
    pub login_name: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub auth_kind: BoundaryAuthKind,
    #[serde(default)]
    pub oidc_login_mode: BoundaryOidcLoginMode,
    #[serde(default)]
    pub oidc_username: String,
    #[serde(default)]
    pub oidc_password: String,
    #[serde(default)]
    pub oidc_otp_secret: String,
    #[serde(default)]
    pub connect_ssh_options: String,
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
            oidc_login_mode: BoundaryOidcLoginMode::Browser,
            oidc_username: String::new(),
            oidc_password: String::new(),
            oidc_otp_secret: String::new(),
            connect_ssh_options: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryConfigForDisk {
    #[serde(default)]
    pub addr: String,
    #[serde(default)]
    pub auth_method_id: String,
    #[serde(default)]
    pub login_name: String,
    #[serde(default)]
    pub auth_kind: BoundaryAuthKind,
    #[serde(default)]
    pub oidc_login_mode: BoundaryOidcLoginMode,
    #[serde(default)]
    pub oidc_username: String,
    #[serde(default)]
    pub connect_ssh_options: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppRuntimeConfig {
    pub default_terminal_cols: u16,
    pub default_terminal_rows: u16,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub executable_path: String,
    #[serde(default)]
    pub ovpn_path: String,
    #[serde(default)]
    pub extra_args: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub otp_secret: String,
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    pub terminate_open_vpn_when_helper_exits: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnLaunchConfig {
    pub executable_path: String,
    pub args: String,
    pub username: String,
    pub password: String,
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    pub terminate_open_vpn_when_helper_exits: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperStatusFile {
    pub state: String,
    pub message: String,
    pub updated_at_ms: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnConfigForDisk {
    pub command: String,
    pub extra_args: String,
    pub username: String,
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    pub terminate_open_vpn_when_helper_exits: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfigForDisk {
    pub app: AppRuntimeConfig,
    pub openvpn: OpenVpnConfigForDisk,
    #[serde(default)]
    pub boundary: BoundaryConfigForDisk,
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
                oidc_login_mode: c.boundary.oidc_login_mode,
                oidc_username: c.boundary.oidc_username.clone(),
                connect_ssh_options: c.boundary.connect_ssh_options.clone(),
            },
        }
    }
}
```

## Suggested `main.rs` changes in PR1

Only make the minimal module wiring changes:

```rust
mod models;
```

Then replace the moved local type definitions with imports, for example:

```rust
use crate::models::{
    AppConfig, AppConfigForDisk, AppRuntimeConfig, BoundaryAuthKind, BoundaryConfig,
    BoundaryConfigForDisk, BoundaryOidcLoginMode, BoundaryTargetSummary, FunctionStatus,
    HelperStatusFile, OpenVpnConfig, OpenVpnConfigForDisk, OpenVpnLaunchConfig,
    OpenVpnLogPayload, PtyExitPayload, PtyOutputPayload, RemoteBrowseEntry,
    RemoteBrowseNodeInfo, RemoteBrowseSession, RemotePathKind, SshEndpoint,
    StartFunctionResult,
};
```

## Recommended PR1 review checklist

- [x] App still compiles
- [ ] No command names changed
- [ ] No event names changed
- [ ] Serde attributes preserved exactly
- [ ] OpenVPN config serialization still matches previous behavior
- [ ] Boundary config serialization still matches previous behavior
- [ ] Remote browser payloads still serialize the same way

## Nice-to-have but optional in PR1

- split `config.rs` later if it becomes too large
- add lightweight unit tests for `AppConfigForDisk::from`

## What PR2 should do next

After PR1 lands:

- add `stores/`
- move `SessionStore` and `FunctionStore`
- introduce `app_state.rs`

## Related docs

- `docs/mcp/MCP_FIRST_REFACTOR_SLICE.md`
- `docs/mcp/MCP_RUST_MODULE_PLAN.md`
