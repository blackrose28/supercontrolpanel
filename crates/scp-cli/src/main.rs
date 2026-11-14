mod args;
mod commands;

use args::{parse_args, ParsedCli};
use commands::{execute_command, write_json_line};
use scp_core::error_envelope;
use std::process::ExitCode;

fn print_help() {
    println!(
        "scpctl {}\n\nUsage:\n  scpctl [--config-dir <DIR>] [--profile <NAME>] <command> [flags]\n\nCommands:\n  schema [--command <CMD>] --json\n  capabilities [--no-profile] [--json]\n  status [--function-id <ID>] [--json]\n  server list [--limit <N>] [--after <ID>] [--json]\n  server get --server-id <ID> [--json]\n  config paths [--json]\n  settings show [--json]\n  mcp serve --stdio\n\nDiscovery (`--help`, `--version`, `schema`, `capabilities --no-profile`) never opens a profile.",
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
