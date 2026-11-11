use clap::{Parser, Subcommand};
use scp_core::servers::read_servers;
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

#[derive(Parser)]
#[command(
    name = "scpctl",
    version,
    about = "Standalone SuperControlPanel CLI (inventory and profile preview)"
)]
struct Cli {
    /// CLI profile directory (default: OS config directory/supercontrolpanel-cli)
    #[arg(long, global = true)]
    config_dir: Option<PathBuf>,
    /// Write JSON to stdout; errors go to stderr
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize the selected CLI profile's local identity
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Inspect the selected profile's server inventory
    Server {
        #[command(subcommand)]
        command: ServerCommand,
    },
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// Create or validate local profile metadata under an exclusive owner lock
    Init,
}

#[derive(Subcommand)]
enum ServerCommand {
    /// List configured servers without connecting to them
    List,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("scpctl: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let config_dir = cli
        .config_dir
        .or_else(|| dirs::config_dir().map(|root| root.join("supercontrolpanel-cli")))
        .ok_or("cannot locate the OS config directory; pass --config-dir")?;
    match cli.command {
        Command::Profile {
            command: ProfileCommand::Init,
        } => initialize_profile(&config_dir, cli.json),
        Command::Server {
            command: ServerCommand::List,
        } => {
            let servers = read_servers(&config_dir)?;
            let output = if cli.json {
                serde_json::to_string_pretty(&servers)
                    .map_err(|error| format!("encode servers: {error}"))?
            } else if servers.is_empty() {
                "No servers configured in this CLI profile.".to_string()
            } else {
                let mut lines = vec!["ID\tNAME\tHOST\tSSH PORT".to_string()];
                lines.extend(servers.iter().map(|server| {
                    format!(
                        "{}\t{}\t{}\t{}",
                        server.id.escape_debug(),
                        server.name.escape_debug(),
                        server.host.escape_debug(),
                        server.ssh_port,
                    )
                }));
                lines.join("\n")
            };
            writeln!(io::stdout().lock(), "{output}")
                .map_err(|error| format!("write output: {error}"))
        }
    }
}

fn initialize_profile(config_dir: &std::path::Path, json: bool) -> Result<(), String> {
    let owner = scp_core::profile::ProfileOwner::acquire(config_dir)?;
    let output = if json {
        serde_json::json!({"credentialNamespace": owner.credential_namespace()}).to_string()
    } else {
        format!("CLI profile initialized: {}", owner.credential_namespace())
    };
    writeln!(io::stdout().lock(), "{output}").map_err(|error| format!("write output: {error}"))
}
