use crate::secret_input::{JsonInputSource, SecretSource};
use scp_core::backup::RestoreOptions;
use scp_core::error::CoreError;
use std::collections::BTreeMap;
use std::path::PathBuf;

type ParseFail<'a> = dyn Fn(&'static str, &str) -> ParseError + 'a;

const F_JSON: u64 = 1 << 0;
const F_INTERACTIVE: u64 = 1 << 1;
const F_NON_INTERACTIVE: u64 = 1 << 2;
const F_NO_PROFILE: u64 = 1 << 3;
const F_STDIO: u64 = 1 << 4;
const F_FOREGROUND: u64 = 1 << 5;
const F_RECURSIVE: u64 = 1 << 6;
const F_OVERWRITE: u64 = 1 << 7;
const F_COMMAND: u64 = 1 << 8;
const F_FUNCTION_ID: u64 = 1 << 9;
const F_LIMIT: u64 = 1 << 10;
const F_AFTER: u64 = 1 << 11;
const F_SERVER_ID: u64 = 1 << 12;
const F_FILTER: u64 = 1 << 13;
const F_FOLDER: u64 = 1 << 14;
const F_LOCAL: u64 = 1 << 15;
const F_REMOTE: u64 = 1 << 16;
const F_SHELL: u64 = 1 << 17;
const F_FILE: u64 = 1 << 18;
const F_STDIN: u64 = 1 << 19;
const F_PREVIEW: u64 = 1 << 20;
const F_EXPECTED_REVISION: u64 = 1 << 21;
const F_SECRET_FILE: u64 = 1 << 22;
const F_SECRET_STDIN: u64 = 1 << 23;
const F_KEY: u64 = 1 << 24;
const F_REPO_ROOT: u64 = 1 << 25;
const F_OUTPUT: u64 = 1 << 26;
const F_INPUT: u64 = 1 << 27;
const F_PASSPHRASE_FILE: u64 = 1 << 28;
const F_PASSPHRASE_STDIN: u64 = 1 << 29;
const F_APPLY: u64 = 1 << 30;
const F_NO_SHARED: u64 = 1 << 31;
const F_NO_SERVERS: u64 = 1 << 32;
const F_NO_LOCAL: u64 = 1 << 33;
const F_NO_SECRETS: u64 = 1 << 34;
const F_USE_SOURCE_PATHS: u64 = 1 << 35;
const F_REMAP: u64 = 1 << 36;
const F_STD: u64 = F_JSON | F_NON_INTERACTIVE;

const FLAG_BITS: &[(u64, &str)] = &[
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
    (F_SHELL, "--shell"),
    (F_FILE, "--file"),
    (F_STDIN, "--stdin"),
    (F_PREVIEW, "--preview"),
    (F_EXPECTED_REVISION, "--expected-revision"),
    (F_SECRET_FILE, "--secret-file"),
    (F_SECRET_STDIN, "--secret-stdin"),
    (F_KEY, "--key"),
    (F_REPO_ROOT, "--repo-root"),
    (F_OUTPUT, "--output"),
    (F_INPUT, "--input"),
    (F_PASSPHRASE_FILE, "--passphrase-file"),
    (F_PASSPHRASE_STDIN, "--passphrase-stdin"),
    (F_APPLY, "--apply"),
    (F_NO_SHARED, "--no-shared"),
    (F_NO_SERVERS, "--no-servers"),
    (F_NO_LOCAL, "--no-local"),
    (F_NO_SECRETS, "--no-secrets"),
    (F_USE_SOURCE_PATHS, "--use-source-paths"),
    (F_REMAP, "--remap"),
];

const COMMAND_ALLOWED: &[(&[&str], u64)] = &[
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

const SLICE3_COMMAND_ALLOWED: &[(&[&str], u64)] = &[
    (
        &["server", "upsert"],
        F_STD | F_FILE | F_STDIN | F_PREVIEW | F_EXPECTED_REVISION,
    ),
    (
        &["server", "delete"],
        F_STD | F_SERVER_ID | F_PREVIEW | F_EXPECTED_REVISION,
    ),
    (
        &["server", "credentials", "set"],
        F_STD | F_SERVER_ID | F_SECRET_FILE | F_SECRET_STDIN | F_INTERACTIVE,
    ),
    (&["server", "credentials", "clear"], F_STD | F_SERVER_ID),
    (&["server", "rdp"], F_SERVER_ID | F_INTERACTIVE | F_STD),
    (
        &["config", "save"],
        F_STD | F_FILE | F_STDIN | F_PREVIEW | F_EXPECTED_REVISION,
    ),
    (
        &["config", "patch"],
        F_STD | F_FILE | F_STDIN | F_PREVIEW | F_EXPECTED_REVISION,
    ),
    (
        &["config", "secret", "set"],
        F_STD | F_KEY | F_SECRET_FILE | F_SECRET_STDIN | F_INTERACTIVE,
    ),
    (&["config", "secret", "clear"], F_STD | F_KEY),
    (
        &["config", "sync", "push"],
        F_STD | F_REPO_ROOT | F_PREVIEW | F_EXPECTED_REVISION,
    ),
    (
        &["settings", "patch"],
        F_STD | F_FILE | F_STDIN | F_PREVIEW | F_EXPECTED_REVISION,
    ),
    (
        &["settings", "export"],
        F_STD | F_OUTPUT | F_OVERWRITE | F_PASSPHRASE_FILE | F_PASSPHRASE_STDIN | F_INTERACTIVE,
    ),
    (
        &["settings", "import"],
        F_STD
            | F_INPUT
            | F_PREVIEW
            | F_APPLY
            | F_INTERACTIVE
            | F_PASSPHRASE_FILE
            | F_PASSPHRASE_STDIN
            | F_NO_SHARED
            | F_NO_SERVERS
            | F_NO_LOCAL
            | F_NO_SECRETS
            | F_USE_SOURCE_PATHS
            | F_REMAP,
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationCommand {
    ServerUpsert {
        input: JsonInputSource,
        preview: bool,
        expected_revision: Option<String>,
    },
    ServerDelete {
        server_id: String,
        preview: bool,
        expected_revision: Option<String>,
    },
    ServerCredentialsSet {
        server_id: String,
        source: SecretSource,
        non_interactive: bool,
    },
    ServerCredentialsClear {
        server_id: String,
    },
    ServerRdp {
        server_id: String,
        interactive: bool,
        non_interactive: bool,
    },
    ConfigSave {
        input: JsonInputSource,
        preview: bool,
        expected_revision: Option<String>,
    },
    ConfigPatch {
        input: JsonInputSource,
        preview: bool,
        expected_revision: Option<String>,
    },
    ConfigSecretSet {
        key: String,
        source: SecretSource,
        non_interactive: bool,
    },
    ConfigSecretClear {
        key: String,
    },
    ConfigSyncPush {
        repo_root: Option<PathBuf>,
        preview: bool,
        expected_revision: Option<String>,
    },
    SettingsPatch {
        input: JsonInputSource,
        preview: bool,
        expected_revision: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupCommand {
    SettingsExport {
        output: PathBuf,
        overwrite: bool,
        source: SecretSource,
        non_interactive: bool,
    },
    SettingsImport {
        input: PathBuf,
        preview: bool,
        apply: bool,
        interactive: bool,
        non_interactive: bool,
        source: Option<SecretSource>,
        options: RestoreOptions,
    },
}

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
    Completion {
        shell: String,
    },
    Mutation(MutationCommand),
    Backup(BackupCommand),
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
    present: u64,
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
    shell: Option<String>,
    file: Option<String>,
    expected_revision: Option<String>,
    secret_file: Option<PathBuf>,
    key: Option<String>,
    repo_root: Option<PathBuf>,
    output: Option<PathBuf>,
    input: Option<PathBuf>,
    passphrase_file: Option<PathBuf>,
    remaps: BTreeMap<String, String>,
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
    if pos.first() == Some(&"completion") {
        check_extras(f.present, F_SHELL, fail)?;
        let shell = match pos {
            ["completion"] => require_str(&f.shell, "shell_required", "--shell", fail)?,
            ["completion", sh] if f.shell.is_none() => (*sh).to_string(),
            _ => {
                return Err(fail(
                    "invalid_arguments",
                    "Usage: scpctl completion <bash|zsh|fish|powershell>",
                ))
            }
        };
        return Ok(Subcommand::Completion { shell });
    }
    let allowed = COMMAND_ALLOWED
        .iter()
        .chain(SLICE3_COMMAND_ALLOWED.iter())
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
    if let Some(cmd) = build_slice3_cmd(pos, f, fail)? {
        return Ok(cmd);
    }
    build_slice2_cmd(pos, f, fail)
}

fn is_forbidden_flag(arg: &str) -> bool {
    let key = arg.split('=').next().unwrap_or(arg);
    matches!(
        key,
        "--password" | "--secret" | "--passphrase" | "--recover-identity"
    )
}

fn scan_flags(args: &[String]) -> Result<RawFlags, ParseError> {
    let wants_json = args.iter().any(|a| a == "--json");
    let mut flags = RawFlags::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let is_mcp = flags.positionals.first().map(String::as_str) == Some("mcp");
        if is_forbidden_flag(arg) {
            return Err(ParseError {
                wants_json,
                is_mcp,
                error: CoreError::invalid_arguments(
                    "unsafe_secret_flag",
                    "Passing secrets, passphrases, or recovery flags on the command line is forbidden; use --secret-file, --secret-stdin, --passphrase-file, --passphrase-stdin, or --interactive",
                ),
            });
        }
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
        "--stdin" => F_STDIN,
        "--preview" => F_PREVIEW,
        "--secret-stdin" => F_SECRET_STDIN,
        "--passphrase-stdin" => F_PASSPHRASE_STDIN,
        "--apply" => F_APPLY,
        "--no-shared" => F_NO_SHARED,
        "--no-servers" => F_NO_SERVERS,
        "--no-local" => F_NO_LOCAL,
        "--no-secrets" => F_NO_SECRETS,
        "--use-source-paths" => F_USE_SOURCE_PATHS,
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
        _ => return apply_slice3_value_flag(arg, args, idx, flags, make_err),
    };
    flags.present |= bit;
    Ok(true)
}

fn apply_slice3_value_flag(
    arg: &str,
    args: &[String],
    idx: &mut usize,
    flags: &mut RawFlags,
    make_err: &dyn Fn(&str) -> ParseError,
) -> Result<bool, ParseError> {
    let bit = match arg {
        "--shell" => {
            flags.shell = Some(next_val(args, idx, arg, make_err)?);
            F_SHELL
        }
        "--file" => {
            flags.file = Some(next_val(args, idx, arg, make_err)?);
            F_FILE
        }
        "--expected-revision" => {
            flags.expected_revision = Some(next_val(args, idx, arg, make_err)?);
            F_EXPECTED_REVISION
        }
        "--secret-file" => {
            flags.secret_file = Some(PathBuf::from(next_val(args, idx, arg, make_err)?));
            F_SECRET_FILE
        }
        "--key" => {
            flags.key = Some(next_val(args, idx, arg, make_err)?);
            F_KEY
        }
        "--repo-root" => {
            flags.repo_root = Some(PathBuf::from(next_val(args, idx, arg, make_err)?));
            F_REPO_ROOT
        }
        "--output" => {
            flags.output = Some(PathBuf::from(next_val(args, idx, arg, make_err)?));
            F_OUTPUT
        }
        "--input" => {
            flags.input = Some(PathBuf::from(next_val(args, idx, arg, make_err)?));
            F_INPUT
        }
        "--passphrase-file" => {
            flags.passphrase_file = Some(PathBuf::from(next_val(args, idx, arg, make_err)?));
            F_PASSPHRASE_FILE
        }
        "--remap" => {
            let raw = next_val(args, idx, arg, make_err)?;
            let (k, v) = raw
                .split_once('=')
                .ok_or_else(|| make_err("--remap must have form <KEY=PATH>"))?;
            if k.trim().is_empty() || v.trim().is_empty() {
                return Err(make_err("--remap key and path must not be empty"));
            }
            flags
                .remaps
                .insert(k.trim().to_string(), v.trim().to_string());
            F_REMAP
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
    present: u64,
    bit: u64,
    reason: &'static str,
    msg: &str,
    fail: &ParseFail<'_>,
) -> Result<(), ParseError> {
    if present & bit == 0 {
        return Err(fail(reason, msg));
    }
    Ok(())
}

fn check_extras(present: u64, allowed: u64, fail: &ParseFail<'_>) -> Result<(), ParseError> {
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

fn forbid_interactive_json(f: &RawFlags, fail: &ParseFail<'_>) -> Result<(), ParseError> {
    if (f.present & F_INTERACTIVE != 0) && (f.present & F_JSON != 0) {
        return Err(fail(
            "conflicting_flags",
            "Cannot combine --interactive with --json",
        ));
    }
    Ok(())
}

fn resolve_json_input(f: &RawFlags, fail: &ParseFail<'_>) -> Result<JsonInputSource, ParseError> {
    let has_file = f.present & F_FILE != 0;
    let has_stdin = f.present & F_STDIN != 0;
    match (has_file, has_stdin) {
        (true, true) => Err(fail(
            "conflicting_flags",
            "Cannot combine --file and --stdin",
        )),
        (false, false) => Err(fail("input_required", "Provide --file <PATH> or --stdin")),
        (false, true) => Ok(JsonInputSource::Stdin),
        (true, false) => {
            let raw = require_str(&f.file, "input_required", "--file", fail)?;
            if raw == "-" {
                Ok(JsonInputSource::Stdin)
            } else {
                Ok(JsonInputSource::File(PathBuf::from(raw)))
            }
        }
    }
}

fn resolve_secret_source(
    file_bit: u64,
    stdin_bit: u64,
    file_path: Option<&PathBuf>,
    file_flag: &str,
    stdin_flag: &str,
    f: &RawFlags,
    fail: &ParseFail<'_>,
) -> Result<SecretSource, ParseError> {
    forbid_interactive_json(f, fail)?;
    let has_file = f.present & file_bit != 0;
    let has_stdin = f.present & stdin_bit != 0;
    let has_tty = f.present & F_INTERACTIVE != 0;
    let count = usize::from(has_file) + usize::from(has_stdin) + usize::from(has_tty);
    if count > 1 {
        return Err(fail(
            "conflicting_flags",
            &format!("Combine at most one of {file_flag}, {stdin_flag}, or --interactive"),
        ));
    }
    if count == 0 {
        return Err(fail(
            "secret_source_required",
            &format!("Provide one of {file_flag}, {stdin_flag}, or --interactive"),
        ));
    }
    if has_file {
        Ok(SecretSource::File(file_path.expect("file checked").clone()))
    } else if has_stdin {
        Ok(SecretSource::Stdin)
    } else {
        Ok(SecretSource::InteractiveTty)
    }
}

fn build_slice3_cmd(
    pos: &[&str],
    f: &RawFlags,
    fail: &ParseFail<'_>,
) -> Result<Option<Subcommand>, ParseError> {
    let preview = f.present & F_PREVIEW != 0;
    let non_interactive = f.present & F_NON_INTERACTIVE != 0;
    let expected_revision = f.expected_revision.clone();
    let cmd = match pos {
        ["server", "upsert"] => Subcommand::Mutation(MutationCommand::ServerUpsert {
            input: resolve_json_input(f, fail)?,
            preview,
            expected_revision,
        }),
        ["server", "delete"] => Subcommand::Mutation(MutationCommand::ServerDelete {
            server_id: require_str(&f.server_id, "server_id_required", "--server-id", fail)?,
            preview,
            expected_revision,
        }),
        ["server", "credentials", "set"] => {
            Subcommand::Mutation(MutationCommand::ServerCredentialsSet {
                server_id: require_str(&f.server_id, "server_id_required", "--server-id", fail)?,
                source: resolve_secret_source(
                    F_SECRET_FILE,
                    F_SECRET_STDIN,
                    f.secret_file.as_ref(),
                    "--secret-file",
                    "--secret-stdin",
                    f,
                    fail,
                )?,
                non_interactive,
            })
        }
        ["server", "credentials", "clear"] => {
            Subcommand::Mutation(MutationCommand::ServerCredentialsClear {
                server_id: require_str(&f.server_id, "server_id_required", "--server-id", fail)?,
            })
        }
        ["server", "rdp"] => {
            forbid_interactive_json(f, fail)?;
            Subcommand::Mutation(MutationCommand::ServerRdp {
                server_id: require_str(&f.server_id, "server_id_required", "--server-id", fail)?,
                interactive: f.present & F_INTERACTIVE != 0,
                non_interactive,
            })
        }
        ["config", "save"] => Subcommand::Mutation(MutationCommand::ConfigSave {
            input: resolve_json_input(f, fail)?,
            preview,
            expected_revision,
        }),
        ["config", "patch"] => Subcommand::Mutation(MutationCommand::ConfigPatch {
            input: resolve_json_input(f, fail)?,
            preview,
            expected_revision,
        }),
        ["config", "secret", "set"] => Subcommand::Mutation(MutationCommand::ConfigSecretSet {
            key: require_str(&f.key, "secret_key_required", "--key", fail)?,
            source: resolve_secret_source(
                F_SECRET_FILE,
                F_SECRET_STDIN,
                f.secret_file.as_ref(),
                "--secret-file",
                "--secret-stdin",
                f,
                fail,
            )?,
            non_interactive,
        }),
        ["config", "secret", "clear"] => Subcommand::Mutation(MutationCommand::ConfigSecretClear {
            key: require_str(&f.key, "secret_key_required", "--key", fail)?,
        }),
        ["config", "sync", "push"] => Subcommand::Mutation(MutationCommand::ConfigSyncPush {
            repo_root: f.repo_root.clone(),
            preview,
            expected_revision,
        }),
        ["settings", "patch"] => Subcommand::Mutation(MutationCommand::SettingsPatch {
            input: resolve_json_input(f, fail)?,
            preview,
            expected_revision,
        }),
        ["settings", "export"] => {
            let output = f
                .output
                .clone()
                .filter(|p| !p.as_os_str().is_empty())
                .ok_or_else(|| fail("output_required", "--output is required"))?;
            Subcommand::Backup(BackupCommand::SettingsExport {
                output,
                overwrite: f.present & F_OVERWRITE != 0,
                source: resolve_secret_source(
                    F_PASSPHRASE_FILE,
                    F_PASSPHRASE_STDIN,
                    f.passphrase_file.as_ref(),
                    "--passphrase-file",
                    "--passphrase-stdin",
                    f,
                    fail,
                )?,
                non_interactive,
            })
        }
        ["settings", "import"] => {
            forbid_interactive_json(f, fail)?;
            let apply = f.present & F_APPLY != 0;
            if preview == apply {
                return Err(fail(
                    "invalid_restore_mode",
                    "Specify exactly one of --preview or --apply for settings import",
                ));
            }
            let input = f
                .input
                .clone()
                .filter(|p| !p.as_os_str().is_empty())
                .ok_or_else(|| fail("input_required", "--input is required"))?;
            let has_file = f.present & F_PASSPHRASE_FILE != 0;
            let has_stdin = f.present & F_PASSPHRASE_STDIN != 0;
            if has_file && has_stdin {
                return Err(fail(
                    "conflicting_flags",
                    "Cannot combine --passphrase-file and --passphrase-stdin",
                ));
            }
            let source = if apply {
                if has_file {
                    Some(SecretSource::File(
                        f.passphrase_file.clone().expect("file checked"),
                    ))
                } else if has_stdin {
                    Some(SecretSource::Stdin)
                } else {
                    Some(SecretSource::InteractiveTty)
                }
            } else {
                Some(resolve_secret_source(
                    F_PASSPHRASE_FILE,
                    F_PASSPHRASE_STDIN,
                    f.passphrase_file.as_ref(),
                    "--passphrase-file",
                    "--passphrase-stdin",
                    f,
                    fail,
                )?)
            };
            let options = RestoreOptions {
                shared: f.present & F_NO_SHARED == 0,
                servers: f.present & F_NO_SERVERS == 0,
                local: f.present & F_NO_LOCAL == 0,
                secrets: f.present & F_NO_SECRETS == 0,
                preserve_paths: f.present & F_USE_SOURCE_PATHS == 0,
                recover_identity: false,
                path_overrides: f.remaps.clone(),
            };
            Subcommand::Backup(BackupCommand::SettingsImport {
                input,
                preview,
                apply,
                interactive: f.present & F_INTERACTIVE != 0,
                non_interactive,
                source,
                options,
            })
        }
        _ => return Ok(None),
    };
    Ok(Some(cmd))
}
