mod args;
mod backup_cmds;
mod commands;
mod completion;
mod mutation_cmds;
mod runtime_cmds;
mod secret_input;

use args::{parse_args, ParsedCli};
use commands::{execute_command, write_json_line};
use scp_core::error_envelope;
use std::process::ExitCode;

fn print_help() {
    println!(
        "scpctl {}\n\nUsage:\n  scpctl [--config-dir <DIR>] [--profile <NAME>] <command> [flags]\n\nCommands:\n  schema [--command <CMD>] --json\n  capabilities [--no-profile] [--json]\n  completion <bash|zsh|fish|powershell>\n  status [--function-id <ID>] [--json]\n  server list [--limit <N>] [--after <ID>] [--json]\n  server get --server-id <ID> [--json]\n  server upsert --file <PATH|-> | --stdin [--preview] [--expected-revision <REV>] [--json]\n  server delete --server-id <ID> [--preview] [--expected-revision <REV>] [--json]\n  server credentials set --server-id <ID> (--secret-file <PATH> | --secret-stdin | --interactive) [--json]\n  server credentials clear --server-id <ID> [--json]\n  server tunnel start --server-id <ID> --foreground [--json]\n  server ssh --server-id <ID> --interactive\n  server rdp --server-id <ID> --interactive\n  openvpn detect [--json]\n  openvpn start --foreground [--json]\n  boundary targets search [--filter <TEXT>] [--limit <N>] [--json]\n  boundary auth [--interactive] [--json]\n  caddy start --foreground [--folder <DIR>] [--json]\n  scp upload --server-id <ID> --local <PATH> --remote <PATH> [--recursive] [--json]\n  scp download --server-id <ID> --remote <PATH> --local <PATH> [--recursive] [--overwrite] [--json]\n  config paths [--json]\n  config save --file <PATH|-> | --stdin [--preview] [--expected-revision <REV>] [--json]\n  config patch --file <PATH|-> | --stdin [--preview] [--expected-revision <REV>] [--json]\n  config secret set --key <KEY> (--secret-file <PATH> | --secret-stdin | --interactive) [--json]\n  config secret clear --key <KEY> [--json]\n  config sync push [--repo-root <DIR>] [--preview] [--expected-revision <REV>] [--json]\n  settings show [--json]\n  settings patch --file <PATH|-> | --stdin [--preview] [--expected-revision <REV>] [--json]\n  settings export --output <PATH> [--overwrite] (--passphrase-file <PATH> | --passphrase-stdin | --interactive) [--json]\n  settings import --input <PATH> (--preview | --apply --interactive) [--passphrase-file <PATH> | --passphrase-stdin] [--json]\n  mcp serve --stdio\n\nDiscovery (`--help`, `--version`, `schema`, `capabilities --no-profile`, `completion`, `openvpn detect`) never opens a profile.",
        env!("CARGO_PKG_VERSION")
    );
}

fn main() -> ExitCode {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let code = match parse_args(&raw_args) {
        Ok(ParsedCli::Help) => {
            print_help();
            0
        }
        Ok(ParsedCli::Version) => {
            println!("scpctl {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Ok(ParsedCli::Command(req)) => execute_command(req),
        Err(parse_err) => {
            if parse_err.is_mcp || !parse_err.wants_json {
                eprintln!("{}", parse_err.error.message);
                parse_err.error.exit_code()
            } else {
                write_json_line(
                    &error_envelope(None, &parse_err.error),
                    parse_err.error.exit_code(),
                )
            }
        }
    };
    ExitCode::from(code.clamp(0, 255) as u8)
}
