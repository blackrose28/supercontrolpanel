mod caddy_host;

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
    about = "Standalone SuperControlPanel CLI preview"
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
    /// Inspect shared configuration in the selected CLI profile
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Run Caddy in the foreground (Linux preview; direct-child cleanup only)
    Caddy {
        #[command(subcommand)]
        command: CaddyCommand,
    },
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
enum CaddyCommand {
    /// Run with raw PTY output in a new file; stdin is unused unless EOF monitoring is requested
    Run {
        /// Explicitly retain ownership until completion or signal cancellation
        #[arg(long, required = true)]
        foreground: bool,
        /// Caddy working directory containing Caddyfile
        #[arg(long)]
        folder: String,
        /// New raw output file; existing files are never overwritten (may grow without limit)
        #[arg(long)]
        output: PathBuf,
        /// Cancel on stdin closure (pipe, socket, or /dev/null); cleanup remains cooperative
        #[arg(long)]
        shutdown_on_stdin_eof: bool,
        /// Refuse launch unless bounded process-tree cleanup is qualified (currently unavailable)
        #[arg(long)]
        require_bounded_shutdown: bool,
    },
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// Create or validate local profile metadata under an exclusive owner lock
    Init,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Show config.json's disk projection; free-text values may be sensitive
    #[command(
        long_about = "Show only config.json's shared disk projection. Known credential/device-path fields are excluded; free-text values may still be sensitive. Missing files produce JSON null or a human absence message. Does not initialize a profile, read local settings/keyring, migrate, write, or start services. This is not a settings backup or import validator."
    )]
    Show,
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
        Command::Config {
            command: ConfigCommand::Show,
        } => show_config(&config_dir, cli.json),
        Command::Caddy {
            command:
                CaddyCommand::Run {
                    folder,
                    output,
                    shutdown_on_stdin_eof,
                    require_bounded_shutdown,
                    ..
                },
        } => caddy_host::run(
            &config_dir,
            &folder,
            &output,
            cli.json,
            caddy_host::ShutdownPolicy {
                stdin_eof: shutdown_on_stdin_eof,
                require_bounded: require_bounded_shutdown,
            },
        ),
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

fn show_config(config_dir: &std::path::Path, json: bool) -> Result<(), String> {
    let config = scp_core::config::read_shared_config(config_dir)?;
    let encoded =
        serde_json::to_string_pretty(&config).map_err(|error| format!("encode config: {error}"))?;
    let output = if json {
        encoded
    } else if config.is_none() {
        "No shared configuration (config.json) in this CLI profile.".to_string()
    } else {
        format!("Shared configuration (config.json; free-text values may be sensitive):\n{encoded}")
    };
    writeln!(io::stdout().lock(), "{output}").map_err(|error| format!("write output: {error}"))
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
