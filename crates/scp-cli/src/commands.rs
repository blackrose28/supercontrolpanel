use crate::args::{BackupCommand, CommandRequest, Subcommand};
use crate::backup_cmds::{execute_backup, execute_interactive_import, prepare_backup};
use crate::completion::generate_completion;
use crate::mutation_cmds::{execute_mutation, prepare_mutation};
use scp_core::error::CoreError;
use scp_core::profile::OwnedProfileLocation;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::OsKeyringBackend;
use scp_core::{error_envelope, success, success_with_runtime};
use scp_mcp::contracts::redact_fields;
use scp_mcp::public_dto::{SafeAppConfig, SafeFunctionStatus, SafeSyncSettings};
use scp_mcp::standalone::{capabilities_report, standalone_schema_catalog};
use scp_mcp::stdio::serve_stdio_runtime;
use serde_json::{json, Value};
use std::io::{self, Write};
use std::sync::Arc;

pub fn execute_command(req: CommandRequest) -> i32 {
    if matches!(req.subcommand, Subcommand::McpServeStdio) {
        return run_mcp_stdio(req.config_dir, req.profile);
    }
    if let Subcommand::Completion { shell } = &req.subcommand {
        return match generate_completion(shell) {
            Ok(script) => {
                print!("{script}");
                0
            }
            Err(err) => {
                eprintln!("{err}");
                err.exit_code()
            }
        };
    }
    if matches!(req.subcommand, Subcommand::ServerSsh { .. }) {
        return crate::runtime_cmds::execute_interactive_ssh(req);
    }
    if matches!(
        req.subcommand,
        Subcommand::Backup(BackupCommand::SettingsImport { apply: true, .. })
    ) {
        return execute_interactive_import(req);
    }

    let json_mode = req.json;
    let outcome = run_shell_command(req);
    emit_shell_outcome(outcome, json_mode)
}

fn run_mcp_stdio(config_dir: Option<std::path::PathBuf>, profile: Option<String>) -> i32 {
    let loc = OwnedProfileLocation::from_env(config_dir, profile);
    let runtime = match ProfileRuntime::open(&loc.as_input(), "stdio") {
        Ok(rt) => Arc::new(rt),
        Err(err) => {
            // Never emit anything to stdout on stdio startup failures.
            eprintln!("{err}");
            return err.exit_code();
        }
    };
    serve_stdio_runtime(runtime, env!("CARGO_PKG_VERSION"))
}

struct ShellResult {
    runtime_id: Option<String>,
    result: Result<Value, CoreError>,
}

fn run_shell_command(req: CommandRequest) -> ShellResult {
    match req.subcommand {
        Subcommand::Schema { command } => ShellResult {
            runtime_id: None,
            result: standalone_schema_catalog(command.as_deref()),
        },
        Subcommand::Capabilities { no_profile: true } => ShellResult {
            runtime_id: None,
            result: capabilities_report(None::<&ProfileRuntime<OsKeyringBackend>>),
        },
        Subcommand::OpenvpnDetect => ShellResult {
            runtime_id: None,
            result: crate::runtime_cmds::openvpn_detect(),
        },
        Subcommand::Mutation(cmd) => match prepare_mutation(cmd) {
            Ok(prep) => {
                with_profile_runtime(req.config_dir, req.profile, |rt| execute_mutation(rt, prep))
            }
            Err(err) => ShellResult {
                runtime_id: None,
                result: Err(err),
            },
        },
        Subcommand::Backup(cmd) => match prepare_backup(cmd) {
            Ok(prep) => {
                with_profile_runtime(req.config_dir, req.profile, |rt| execute_backup(rt, prep))
            }
            Err(err) => ShellResult {
                runtime_id: None,
                result: Err(err),
            },
        },
        other => with_profile_runtime(req.config_dir, req.profile, |rt| {
            dispatch_with_runtime(rt, other)
        }),
    }
}

fn with_profile_runtime(
    config_dir: Option<std::path::PathBuf>,
    profile: Option<String>,
    op: impl FnOnce(&ProfileRuntime<OsKeyringBackend>) -> Result<Value, CoreError>,
) -> ShellResult {
    let loc = OwnedProfileLocation::from_env(config_dir, profile);
    let rt = match ProfileRuntime::open(&loc.as_input(), "cli") {
        Ok(rt) => rt,
        Err(err) => {
            return ShellResult {
                runtime_id: None,
                result: Err(err),
            }
        }
    };
    let runtime_id = Some(rt.runtime_id().to_string());
    let result = op(&rt);
    rt.shutdown(std::time::Duration::from_secs(2));
    ShellResult { runtime_id, result }
}

fn dispatch_with_runtime(
    rt: &ProfileRuntime<OsKeyringBackend>,
    cmd: Subcommand,
) -> Result<Value, CoreError> {
    match cmd {
        Subcommand::Capabilities { .. } => capabilities_report(Some(rt)),
        Subcommand::Status { function_id } => status_json(rt, function_id.as_deref()),
        Subcommand::ServerList { limit, after } => {
            let page = rt.list_servers(limit, after.as_deref())?;
            Ok(json!({
                "servers": page.servers,
                "truncated": page.truncated,
                "nextAfter": page.next_after,
                "total": page.total
            }))
        }
        Subcommand::ServerGet { server_id } => Ok(json!(rt.get_server(&server_id)?)),
        Subcommand::ConfigPaths => config_paths_json(rt),
        Subcommand::SettingsShow => settings_show_json(rt),
        other => dispatch_runtime_ops(rt, other),
    }
}

fn status_json(
    rt: &ProfileRuntime<OsKeyringBackend>,
    function_id: Option<&str>,
) -> Result<Value, CoreError> {
    match function_id {
        None => {
            let list = rt
                .list_function_statuses()?
                .iter()
                .map(SafeFunctionStatus::from)
                .collect::<Vec<_>>();
            Ok(json!({ "statuses": list }))
        }
        Some(id) => Ok(json!(SafeFunctionStatus::from(
            &rt.get_function_status(id)?
        ))),
    }
}

fn config_paths_json(rt: &ProfileRuntime<OsKeyringBackend>) -> Result<Value, CoreError> {
    let paths = rt.profile_paths();
    Ok(json!({
        "appConfigPath": paths.config_path,
        "profileDir": paths.profile_dir,
        "serversPath": paths.servers_path,
        "syncLocalPath": paths.sync_local_path,
        "stdioPolicyPath": paths.stdio_policy_path
    }))
}

fn settings_show_json(rt: &ProfileRuntime<OsKeyringBackend>) -> Result<Value, CoreError> {
    let app_cfg = SafeAppConfig::from(&rt.get_app_config()?);
    let sync = SafeSyncSettings::from(&rt.get_sync_settings()?);
    let policy = rt.get_stdio_policy()?;
    let mut val = json!({
        "appConfig": app_cfg,
        "syncSettings": sync,
        "stdioPolicy": policy
    });
    redact_fields(&mut val);
    Ok(val)
}

fn dispatch_runtime_ops(
    rt: &ProfileRuntime<OsKeyringBackend>,
    cmd: Subcommand,
) -> Result<Value, CoreError> {
    match cmd {
        Subcommand::OpenvpnStart
        | Subcommand::CaddyStart { .. }
        | Subcommand::ServerTunnelStart { .. } => crate::runtime_cmds::run_foreground(rt, cmd),
        Subcommand::BoundaryTargetsSearch { filter, limit } => Ok(json!({
            "targets": rt.search_boundary_targets(filter.as_deref(), limit)?
        })),
        Subcommand::BoundaryAuth { .. } => crate::runtime_cmds::run_boundary_auth(rt),
        cmd @ (Subcommand::ScpUpload { .. } | Subcommand::ScpDownload { .. }) => {
            crate::runtime_cmds::run_scp(rt, &cmd)
        }
        Subcommand::Schema { .. }
        | Subcommand::McpServeStdio
        | Subcommand::OpenvpnDetect
        | Subcommand::ServerSsh { .. }
        | Subcommand::Capabilities { .. }
        | Subcommand::Status { .. }
        | Subcommand::ServerList { .. }
        | Subcommand::ServerGet { .. }
        | Subcommand::ConfigPaths
        | Subcommand::SettingsShow
        | Subcommand::Completion { .. }
        | Subcommand::Mutation(..)
        | Subcommand::Backup(..) => unreachable!(),
    }
}

fn emit_shell_outcome(res: ShellResult, json_mode: bool) -> i32 {
    if !json_mode {
        match res.result {
            Ok(data) => {
                let pretty = serde_json::to_string_pretty(&data).unwrap_or_default();
                println!("{pretty}");
                return 0;
            }
            Err(err) => {
                eprintln!("{err}");
                return err.exit_code();
            }
        }
    }
    let (exit, envelope) = match res.result {
        Ok(data) => {
            let env = match res.runtime_id.as_deref() {
                Some(id) => success_with_runtime(id, data),
                None => success(data),
            };
            (0, env)
        }
        Err(err) => (
            err.exit_code(),
            error_envelope(res.runtime_id.as_deref(), &err),
        ),
    };
    write_json_line(&envelope, exit)
}

pub fn write_json_line(value: &Value, success_exit: i32) -> i32 {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if serde_json::to_writer(&mut out, value).is_err() || out.write_all(b"\n").is_err() {
        eprintln!("Unable to write command result.");
        return 1;
    }
    success_exit
}
