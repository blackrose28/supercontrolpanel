use scp_core::error::CoreError;
use std::path::PathBuf;

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
    json: bool,
    interactive: bool,
    non_interactive: bool,
    no_profile: bool,
    stdio: bool,
    config_dir: Option<PathBuf>,
    profile: Option<String>,
    command: Option<String>,
    function_id: Option<String>,
    limit: Option<usize>,
    after: Option<String>,
    server_id: Option<String>,
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
    let is_mcp = intent.positionals.first().map(String::as_str) == Some("mcp");
    let fail = |reason: &'static str, msg: &str| ParseError {
        wants_json: intent.json,
        is_mcp,
        error: CoreError::invalid_arguments(reason, msg),
    };
    if intent.interactive {
        return Err(fail(
            "interactive_not_supported",
            "--interactive is not supported for Slice 1 read-only commands",
        ));
    }
    let pos: Vec<&str> = intent.positionals.iter().map(String::as_str).collect();
    let subcommand = match pos.as_slice() {
        ["schema"] => parse_schema(&intent, &fail)?,
        ["capabilities"] => parse_capabilities(&intent, &fail)?,
        ["status"] => parse_status(&intent, &fail)?,
        ["server", "list"] => parse_server_list(&intent, &fail)?,
        ["server", "get"] => parse_server_get(&intent, &fail)?,
        ["config", "paths"] => parse_no_extra(&intent, Subcommand::ConfigPaths, &fail)?,
        ["settings", "show"] => parse_no_extra(&intent, Subcommand::SettingsShow, &fail)?,
        ["mcp", "serve"] => parse_mcp_serve(&intent, &fail)?,
        _ => {
            return Err(fail(
                "invalid_arguments",
                "Unsupported command or arguments. Run scpctl --help.",
            ))
        }
    };
    Ok(ParsedCli::Command(CommandRequest {
        json: intent.json,
        config_dir: intent.config_dir,
        profile: intent.profile,
        subcommand,
    }))
}

fn scan_flags(args: &[String]) -> Result<RawFlags, ParseError> {
    let wants_json = args.iter().any(|a| a == "--json");
    let mut is_mcp = false;
    let mut flags = RawFlags {
        json: wants_json,
        ..Default::default()
    };
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let make_err = |msg: &str| ParseError {
            wants_json,
            is_mcp,
            error: CoreError::invalid_arguments("invalid_arguments", msg),
        };
        match arg {
            "--json" => flags.json = true,
            "--interactive" => flags.interactive = true,
            "--non-interactive" => flags.non_interactive = true,
            "--no-profile" => flags.no_profile = true,
            "--stdio" => flags.stdio = true,
            "--config-dir" => {
                flags.config_dir = Some(PathBuf::from(next_val(args, &mut i, arg, make_err)?))
            }
            "--profile" => flags.profile = Some(next_val(args, &mut i, arg, make_err)?),
            "--command" => flags.command = Some(next_val(args, &mut i, arg, make_err)?),
            "--function-id" => flags.function_id = Some(next_val(args, &mut i, arg, make_err)?),
            "--after" => flags.after = Some(next_val(args, &mut i, arg, make_err)?),
            "--server-id" => flags.server_id = Some(next_val(args, &mut i, arg, make_err)?),
            "--limit" => {
                let raw = next_val(args, &mut i, arg, make_err)?;
                let parsed = raw
                    .parse::<usize>()
                    .map_err(|_| make_err("--limit must be a positive integer"))?;
                flags.limit = Some(parsed);
            }
            _ if arg.starts_with('-') => {
                return Err(make_err(&format!("Unknown or misplaced flag: {arg}")));
            }
            _ => {
                if flags.positionals.is_empty() && arg == "mcp" {
                    is_mcp = true;
                }
                flags.positionals.push(arg.to_string());
            }
        }
        i += 1;
    }
    Ok(flags)
}

fn next_val(
    args: &[String],
    idx: &mut usize,
    flag: &str,
    make_err: impl Fn(&str) -> ParseError,
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

fn ensure_absent(
    present: bool,
    flag: &str,
    fail: &dyn Fn(&'static str, &str) -> ParseError,
) -> Result<(), ParseError> {
    if present {
        return Err(fail(
            "invalid_arguments",
            &format!("Unexpected flag {flag}"),
        ));
    }
    Ok(())
}

fn check_extras(
    f: &RawFlags,
    allow_no_profile: bool,
    allow_cmd: bool,
    allow_fn: bool,
    allow_list: bool,
    allow_srv: bool,
    fail: &dyn Fn(&'static str, &str) -> ParseError,
) -> Result<(), ParseError> {
    ensure_absent(!allow_no_profile && f.no_profile, "--no-profile", fail)?;
    ensure_absent(!allow_cmd && f.command.is_some(), "--command", fail)?;
    ensure_absent(!allow_fn && f.function_id.is_some(), "--function-id", fail)?;
    ensure_absent(!allow_list && f.limit.is_some(), "--limit", fail)?;
    ensure_absent(!allow_list && f.after.is_some(), "--after", fail)?;
    ensure_absent(!allow_srv && f.server_id.is_some(), "--server-id", fail)?;
    ensure_absent(f.stdio, "--stdio", fail)?;
    Ok(())
}

fn parse_schema(
    f: &RawFlags,
    fail: &dyn Fn(&'static str, &str) -> ParseError,
) -> Result<Subcommand, ParseError> {
    check_extras(f, false, true, false, false, false, fail)?;
    Ok(Subcommand::Schema {
        command: f.command.clone(),
    })
}

fn parse_capabilities(
    f: &RawFlags,
    fail: &dyn Fn(&'static str, &str) -> ParseError,
) -> Result<Subcommand, ParseError> {
    check_extras(f, true, false, false, false, false, fail)?;
    Ok(Subcommand::Capabilities {
        no_profile: f.no_profile,
    })
}

fn parse_status(
    f: &RawFlags,
    fail: &dyn Fn(&'static str, &str) -> ParseError,
) -> Result<Subcommand, ParseError> {
    check_extras(f, false, false, true, false, false, fail)?;
    Ok(Subcommand::Status {
        function_id: f.function_id.clone(),
    })
}

fn parse_server_list(
    f: &RawFlags,
    fail: &dyn Fn(&'static str, &str) -> ParseError,
) -> Result<Subcommand, ParseError> {
    check_extras(f, false, false, false, true, false, fail)?;
    Ok(Subcommand::ServerList {
        limit: f.limit,
        after: f.after.clone(),
    })
}

fn parse_server_get(
    f: &RawFlags,
    fail: &dyn Fn(&'static str, &str) -> ParseError,
) -> Result<Subcommand, ParseError> {
    check_extras(f, false, false, false, false, true, fail)?;
    let server_id = f
        .server_id
        .clone()
        .ok_or_else(|| fail("server_id_required", "--server-id is required"))?;
    Ok(Subcommand::ServerGet { server_id })
}

fn parse_no_extra(
    f: &RawFlags,
    cmd: Subcommand,
    fail: &dyn Fn(&'static str, &str) -> ParseError,
) -> Result<Subcommand, ParseError> {
    check_extras(f, false, false, false, false, false, fail)?;
    Ok(cmd)
}

fn parse_mcp_serve(
    f: &RawFlags,
    fail: &dyn Fn(&'static str, &str) -> ParseError,
) -> Result<Subcommand, ParseError> {
    ensure_absent(f.json, "--json", fail)?;
    ensure_absent(f.non_interactive, "--non-interactive", fail)?;
    ensure_absent(f.no_profile, "--no-profile", fail)?;
    ensure_absent(f.command.is_some(), "--command", fail)?;
    ensure_absent(f.function_id.is_some(), "--function-id", fail)?;
    ensure_absent(f.limit.is_some(), "--limit", fail)?;
    ensure_absent(f.after.is_some(), "--after", fail)?;
    ensure_absent(f.server_id.is_some(), "--server-id", fail)?;
    if !f.stdio {
        return Err(fail(
            "stdio_required",
            "mcp serve requires --stdio in Slice 1",
        ));
    }
    Ok(Subcommand::McpServeStdio)
}
