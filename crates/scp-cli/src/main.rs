use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    let value = match args.as_slice() {
        ["--help"] => {
            println!("scpctl: standalone discovery preview\n\nUsage:\n  scpctl schema --json\n  scpctl --version\n\nRuntime commands and stdio serving are not implemented yet.\nDiscovery never opens a profile or contacts the desktop.");
            return ExitCode::SUCCESS;
        }
        ["--version"] => {
            println!("scpctl {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        ["schema", "--json"] => {
            let mut catalog = scp_mcp::standalone_catalog();
            catalog["commands"] = serde_json::json!([{
                "name":"schema","arguments":["--json"],"opensProfile":false,
                "inputSchema":{"type":"object","additionalProperties":false},
                "output":"versioned-result-envelope"
            }]);
            scp_core::success(catalog)
        }
        _ => {
            // Even failed stdio startup must reserve stdout for protocol messages.
            if args.first() == Some(&"mcp") || !args.contains(&"--json") {
                eprintln!("Unsupported command or arguments. Run scpctl --help.");
                return ExitCode::from(2);
            }
            scp_core::invalid_arguments()
        }
    };
    let exit = if value["ok"] == true { 0 } else { 2 };
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if serde_json::to_writer(&mut out, &value).is_err() || out.write_all(b"\n").is_err() {
        eprintln!("Unable to write command result.");
        return ExitCode::FAILURE;
    }
    ExitCode::from(exit)
}
