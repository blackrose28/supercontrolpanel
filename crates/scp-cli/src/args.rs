use scp_core::error::CoreError;
use std::path::PathBuf;

type ParseFail<'a> = dyn Fn(&'static str, &str) -> ParseError + 'a;

const F_JSON: u32 = 1 << 0;
const F_INTERACTIVE: u32 = 1 << 1;
const F_NON_INTERACTIVE: u32 = 1 << 2;
const F_NO_PROFILE: u32 = 1 << 3;
const F_STDIO: u32 = 1 << 4;
const F_FOREGROUND: u32 = 1 << 5;
const F_RECURSIVE: u32 = 1 << 6;
const F_OVERWRITE: u32 = 1 << 7;
const F_COMMAND: u32 = 1 << 8;
const F_FUNCTION_ID: u32 = 1 << 9;
const F_LIMIT: u32 = 1 << 10;
const F_AFTER: u32 = 1 << 11;
const F_SERVER_ID: u32 = 1 << 12;
const F_FILTER: u32 = 1 << 13;
const F_FOLDER: u32 = 1 << 14;
const F_LOCAL: u32 = 1 << 15;
const F_REMOTE: u32 = 1 << 16;
const F_STD: u32 = F_JSON | F_NON_INTERACTIVE;

const FLAG_BITS: &[(u32, &str)] = &[
    (F_JSON, "--json"),
    (F_NON_INTERACTIVE, "--non-interactive"),
    (F_NO_PROFILE, "--no-profile"),
    (F_STDIO, "--stdio"),
    (F_FOREGROUND, "--foreground"),
    (F_RECURSIVE, "--recursive"),
    (F_OVERWRITE, "--overwrite"),
    (F_COMMAND, "--command"),
    (F_FUNCTION_ID, "--function-id"),
    (F_LIMIT, "--limit"),
    (F_AFTER, "--after"),
    (F_SERVER_ID, "--server-id"),
    (F_FILTER, "--filter"),
    (F_FOLDER, "--folder"),
    (F_LOCAL, "--local"),
    (F_REMOTE, "--remote"),
];

const COMMAND_ALLOWED: &[(&[&str], u32)] = &[
    (&["schema"], F_STD | F_COMMAND),
    (&["capabilities"], F_STD | F_NO_PROFILE),
    (&["status"], F_STD | F_FUNCTION_ID),
    (&["server", "list"], F_STD | F_LIMIT | F_AFTER),
    (&["server", "get"], F_STD | F_SERVER_ID),
    (&["config", "paths"], F_STD),
    (&["settings", "show"], F_STD),
    (&["mcp", "serve"], F_STDIO),
    (&["openvpn", "detect"], F_STD),
    (&["openvpn", "start"], F_STD | F_FOREGROUND),
    (
        &["boundary", "targets", "search"],
        F_STD | F_FILTER | F_LIMIT,
    ),
    (&["boundary", "auth"], F_STD | F_INTERACTIVE),
    (&["caddy", "start"], F_STD | F_FOREGROUND | F_FOLDER),
    (
        &["server", "tunnel", "start"],
        F_STD | F_SERVER_ID | F_FOREGROUND,
    ),
    (&["server", "ssh"], F_SERVER_ID | F_INTERACTIVE),
    (
        &["scp", "upload"],
        F_STD | F_SERVER_ID | F_LOCAL | F_REMOTE | F_RECURSIVE,
    ),
    (
        &["scp", "download"],
        F_STD | F_SERVER_ID | F_REMOTE | F_LOCAL | F_RECURSIVE | F_OVERWRITE,
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subcommand {
    Schema {
        command: Option<String>,
    },
    Capabilities {
        no_profile: bool,
    },
    Status {
        function_id: Option<String>,
    },
    ServerList {
        limit: Option<usize>,
        after: Option<String>,
    },
    ServerGet {
        server_id: String,
    },
    ConfigPaths,
    SettingsShow,
    McpServeStdio,
    OpenvpnDetect,
    OpenvpnStart,
    BoundaryTargetsSearch {
        filter: Option<String>,
        limit: Option<usize>,
    },
    BoundaryAuth {
        interactive: bool,
    },
    CaddyStart {
        folder: Option<String>,
    },
    ServerTunnelStart {
        server_id: String,
    },
    ServerSsh {
        server_id: String,
    },
    ScpUpload {
        server_id: String,
        local: String,
        remote: String,
        recursive: bool,
    },
    ScpDownload {
        server_id: String,
        remote: String,
        local: String,
        recursive: bool,
        overwrite: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRequest {
    pub json: bool,
    pub config_dir: Option<PathBuf>,
    pub profile: Option<String>,
    pub subcommand: Subcommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCli {
    Help,
    Version,
    Command(CommandRequest),
}

#[derive(Debug)]
pub struct ParseError {
    pub wants_json: bool,
    pub is_mcp: bool,
    pub error: CoreError,
}

#[derive(Default)]
struct RawFlags {
    present: u32,
    config_dir: Option<PathBuf>,
    profile: Option<String>,
    command: Option<String>,
    function_id: Option<String>,
    limit: Option<usize>,
    after: Option<String>,
    server_id: Option<String>,
    filter: Option<String>,
    folder: Option<String>,
    local: Option<String>,
    remote: Option<String>,
    positionals: Vec<String>,
}

pub fn parse_args(args: &[String]) -> Result<ParsedCli, ParseError> {
    if args.len() == 1 {
        match args[0].as_str() {
            "--help" | "-h" => return Ok(ParsedCli::Help),
            "--version" | "-V" => return Ok(ParsedCli::Version),
            _ => {}
        }
    }
    let intent = scan_flags(args)?;
    let wants_json = intent.present & F_JSON != 0;
    let is_mcp = intent.positionals.first().map(String::as_str) == Some("mcp");
    let fail = |reason: &'static str, msg: &str| ParseError {
        wants_json,
        is_mcp,
        error: CoreError::invalid_arguments(reason, msg),
    };
    if (intent.present & F_INTERACTIVE != 0) && (intent.present & F_NON_INTERACTIVE != 0) {
        return Err(fail(
            "conflicting_interaction_flags",
            "Cannot combine --interactive and --non-interactive",
        ));
    }
    let pos: Vec<&str> = intent.positionals.iter().map(String::as_str).collect();
    let subcommand = dispatch_parse(&pos, &intent, &fail)?;
    Ok(ParsedCli::Command(CommandRequest {
        json: wants_json,
        config_dir: intent.config_dir,
        profile: intent.profile,
        subcommand,
    }))
}

fn dispatch_parse(
    pos: &[&str],
    f: &RawFlags,
    fail: &ParseFail<'_>,
) -> Result<Subcommand, ParseError> {
    let allowed = COMMAND_ALLOWED
        .iter()
        .find(|&&(p, _)| p == pos)
        .map(|&(_, mask)| mask)
        .ok_or_else(|| {
            fail(
                "invalid_arguments",
                "Unsupported command or arguments. Run scpctl --help.",
            )
        })?;
    check_extras(f.present, allowed, fail)?;
    if let Some(cmd) = build_slice1_cmd(pos, f, fail)? {
        return Ok(cmd);
    }
    build_slice2_cmd(pos, f, fail)
}

fn scan_flags(args: &[String]) -> Result<RawFlags, ParseError> {
    let wants_json = args.iter().any(|a| a == "--json");
    let mut flags = RawFlags::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let is_mcp = flags.positionals.first().map(String::as_str) == Some("mcp");
        let make_err = |msg: &str| ParseError {
            wants_json,
            is_mcp,
            error: CoreError::invalid_arguments("invalid_arguments", msg),
        };
        if apply_bool_flag(arg, &mut flags)
            || apply_value_flag(arg, args, &mut i, &mut flags, &make_err)?
        {
            i += 1;
            continue;
        }
        if arg.starts_with('-') {
            return Err(make_err(&format!("Unknown or misplaced flag: {arg}")));
        }
        flags.positionals.push(arg.to_string());
        i += 1;
    }
    Ok(flags)
}

fn apply_bool_flag(arg: &str, flags: &mut RawFlags) -> bool {
    let bit = match arg {
        "--json" => F_JSON,
        "--interactive" => F_INTERACTIVE,
        "--non-interactive" => F_NON_INTERACTIVE,
        "--no-profile" => F_NO_PROFILE,
        "--stdio" => F_STDIO,
        "--foreground" => F_FOREGROUND,
        "--recursive" => F_RECURSIVE,
        "--overwrite" => F_OVERWRITE,
        _ => return false,
    };
    flags.present |= bit;
    true
}

fn apply_value_flag(
    arg: &str,
    args: &[String],
    idx: &mut usize,
    flags: &mut RawFlags,
    make_err: &dyn Fn(&str) -> ParseError,
) -> Result<bool, ParseError> {
    let bit = match arg {
        "--config-dir" => {
            flags.config_dir = Some(PathBuf::from(next_val(args, idx, arg, make_err)?));
            return Ok(true);
        }
        "--profile" => {
            flags.profile = Some(next_val(args, idx, arg, make_err)?);
            return Ok(true);
        }
        "--command" => {
            flags.command = Some(next_val(args, idx, arg, make_err)?);
            F_COMMAND
        }
        "--function-id" => {
            flags.function_id = Some(next_val(args, idx, arg, make_err)?);
            F_FUNCTION_ID
        }
        "--after" => {
            flags.after = Some(next_val(args, idx, arg, make_err)?);
            F_AFTER
        }
        "--server-id" => {
            flags.server_id = Some(next_val(args, idx, arg, make_err)?);
            F_SERVER_ID
        }
        "--filter" => {
            flags.filter = Some(next_val(args, idx, arg, make_err)?);
            F_FILTER
        }
        "--folder" => {
            flags.folder = Some(next_val(args, idx, arg, make_err)?);
            F_FOLDER
        }
        "--local" => {
            flags.local = Some(next_val(args, idx, arg, make_err)?);
            F_LOCAL
        }
        "--remote" => {
            flags.remote = Some(next_val(args, idx, arg, make_err)?);
            F_REMOTE
        }
        "--limit" => {
            let raw = next_val(args, idx, arg, make_err)?;
            let parsed = raw
                .parse::<usize>()
                .map_err(|_| make_err("--limit must be a positive integer"))?;
            flags.limit = Some(parsed);
            F_LIMIT
        }
        _ => return Ok(false),
    };
    flags.present |= bit;
    Ok(true)
}

fn next_val(
    args: &[String],
    idx: &mut usize,
    flag: &str,
    make_err: &dyn Fn(&str) -> ParseError,
) -> Result<String, ParseError> {
    *idx += 1;
    let val = args
        .get(*idx)
        .ok_or_else(|| make_err(&format!("Missing value for {flag}")))?;
    if flag != "--config-dir" && val.starts_with("--") {
        return Err(make_err(&format!("Missing value for {flag}")));
    }
    if flag == "--config-dir" && val.starts_with("--") {
        return Err(make_err(&format!("Missing path value for {flag}")));
    }
    Ok(val.clone())
}

fn require_str(
    val: &Option<String>,
    reason: &'static str,
    flag: &str,
    fail: &ParseFail<'_>,
) -> Result<String, ParseError> {
    val.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| fail(reason, &format!("{flag} is required")))
}

fn require_flag(
    present: u32,
    bit: u32,
    reason: &'static str,
    msg: &str,
    fail: &ParseFail<'_>,
) -> Result<(), ParseError> {
    if present & bit == 0 {
        return Err(fail(reason, msg));
    }
    Ok(())
}

fn check_extras(present: u32, allowed: u32, fail: &ParseFail<'_>) -> Result<(), ParseError> {
    let unexpected = present & !allowed;
    for &(bit, flag) in FLAG_BITS {
        if unexpected & bit != 0 {
            return Err(fail(
                "invalid_arguments",
                &format!("Unexpected flag {flag}"),
            ));
        }
    }
    if unexpected & F_INTERACTIVE != 0 {
        return Err(fail(
            "interactive_not_supported",
            "--interactive is not supported for this command",
        ));
    }
    Ok(())
}

fn build_slice1_cmd(
    pos: &[&str],
    f: &RawFlags,
    fail: &ParseFail<'_>,
) -> Result<Option<Subcommand>, ParseError> {
    let cmd = match pos {
        ["schema"] => Subcommand::Schema {
            command: f.command.clone(),
        },
        ["capabilities"] => Subcommand::Capabilities {
            no_profile: f.present & F_NO_PROFILE != 0,
        },
        ["status"] => Subcommand::Status {
            function_id: f.function_id.clone(),
        },
        ["server", "list"] => Subcommand::ServerList {
            limit: f.limit,
            after: f.after.clone(),
        },
        ["server", "get"] => Subcommand::ServerGet {
            server_id: require_str(&f.server_id, "server_id_required", "--server-id", fail)?,
        },
        ["config", "paths"] => Subcommand::ConfigPaths,
        ["settings", "show"] => Subcommand::SettingsShow,
        ["mcp", "serve"] => {
            require_flag(
                f.present,
                F_STDIO,
                "stdio_required",
                "mcp serve requires --stdio in Slice 1",
                fail,
            )?;
            Subcommand::McpServeStdio
        }
        _ => return Ok(None),
    };
    Ok(Some(cmd))
}

fn build_slice2_cmd(
    pos: &[&str],
    f: &RawFlags,
    fail: &ParseFail<'_>,
) -> Result<Subcommand, ParseError> {
    match pos {
        ["openvpn", "detect"] => Ok(Subcommand::OpenvpnDetect),
        ["openvpn", "start"] => {
            require_flag(
                f.present,
                F_FOREGROUND,
                "foreground_required",
                "openvpn start requires --foreground",
                fail,
            )?;
            Ok(Subcommand::OpenvpnStart)
        }
        ["boundary", "targets", "search"] => Ok(Subcommand::BoundaryTargetsSearch {
            filter: f.filter.clone(),
            limit: f.limit,
        }),
        ["boundary", "auth"] => Ok(Subcommand::BoundaryAuth {
            interactive: f.present & F_INTERACTIVE != 0,
        }),
        ["caddy", "start"] => {
            require_flag(
                f.present,
                F_FOREGROUND,
                "foreground_required",
                "caddy start requires --foreground",
                fail,
            )?;
            Ok(Subcommand::CaddyStart {
                folder: f.folder.clone(),
            })
        }
        ["server", "tunnel", "start"] => {
            require_flag(
                f.present,
                F_FOREGROUND,
                "foreground_required",
                "server tunnel start requires --foreground",
                fail,
            )?;
            let server_id = require_str(&f.server_id, "server_id_required", "--server-id", fail)?;
            Ok(Subcommand::ServerTunnelStart { server_id })
        }
        ["server", "ssh"] => {
            require_flag(
                f.present,
                F_INTERACTIVE,
                "interactive_required",
                "server ssh requires --interactive",
                fail,
            )?;
            let server_id = require_str(&f.server_id, "server_id_required", "--server-id", fail)?;
            Ok(Subcommand::ServerSsh { server_id })
        }
        ["scp", "upload"] => Ok(Subcommand::ScpUpload {
            server_id: require_str(&f.server_id, "server_id_required", "--server-id", fail)?,
            local: require_str(&f.local, "local_path_required", "--local", fail)?,
            remote: require_str(&f.remote, "remote_path_required", "--remote", fail)?,
            recursive: f.present & F_RECURSIVE != 0,
        }),
        ["scp", "download"] => Ok(Subcommand::ScpDownload {
            server_id: require_str(&f.server_id, "server_id_required", "--server-id", fail)?,
            remote: require_str(&f.remote, "remote_path_required", "--remote", fail)?,
            local: require_str(&f.local, "local_path_required", "--local", fail)?,
            recursive: f.present & F_RECURSIVE != 0,
            overwrite: f.present & F_OVERWRITE != 0,
        }),
        _ => unreachable!(),
    }
}
