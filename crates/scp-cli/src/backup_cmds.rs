use crate::args::{BackupCommand, CommandRequest, Subcommand};
use crate::secret_input::{ensure_interactive_tty, read_passphrase_value, SecretSource};
use scp_core::backup::{Preview, RestoreOptions};
use scp_core::error::CoreError;
use scp_core::profile::OwnedProfileLocation;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::OsKeyringBackend;
use scp_core::tty::{read_tty_line, SignalWatcher};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use zeroize::Zeroizing;

const DEFAULT_PREVIEW_TTL_MS: u64 = 300_000;

pub fn preview_ttl() -> Duration {
    let ms = std::env::var("SCPCTL_TEST_PREVIEW_TTL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_PREVIEW_TTL_MS);
    Duration::from_millis(ms)
}

pub enum PreparedBackup {
    Export {
        output: PathBuf,
        overwrite: bool,
        passphrase: Zeroizing<String>,
    },
    ImportPreview {
        input: PathBuf,
        passphrase: Zeroizing<String>,
        options: RestoreOptions,
    },
}

pub fn prepare_backup(cmd: BackupCommand) -> Result<PreparedBackup, CoreError> {
    match cmd {
        BackupCommand::SettingsExport {
            output,
            overwrite,
            source,
            non_interactive,
        } => {
            if matches!(source, SecretSource::InteractiveTty) {
                ensure_interactive_tty(
                    true,
                    non_interactive,
                    "Interactive export requires --interactive and a controlling terminal",
                )?;
            }
            let passphrase = read_passphrase_value(&source, true, non_interactive)?;
            Ok(PreparedBackup::Export {
                output,
                overwrite,
                passphrase,
            })
        }
        BackupCommand::SettingsImport {
            input,
            preview: true,
            apply: false,
            interactive: _,
            non_interactive,
            source,
            options,
        } => {
            let source = source.ok_or_else(|| {
                CoreError::invalid_arguments(
                    "passphrase_source_required",
                    "Use --passphrase-file, --passphrase-stdin, or --interactive",
                )
            })?;
            if matches!(source, SecretSource::InteractiveTty) {
                ensure_interactive_tty(
                    true,
                    non_interactive,
                    "Interactive preview requires --interactive and a controlling terminal",
                )?;
            }
            let passphrase = read_passphrase_value(&source, false, non_interactive)?;
            Ok(PreparedBackup::ImportPreview {
                input,
                passphrase,
                options,
            })
        }
        _ => unreachable!(),
    }
}

pub fn execute_backup(
    rt: &ProfileRuntime<OsKeyringBackend>,
    prepared: PreparedBackup,
) -> Result<Value, CoreError> {
    match prepared {
        PreparedBackup::Export {
            output,
            overwrite,
            passphrase,
        } => rt.export_settings_backup(&output, &passphrase, overwrite),
        PreparedBackup::ImportPreview {
            input,
            passphrase,
            options,
        } => {
            let (preview, pending) =
                rt.prepare_settings_import(&input, &passphrase, options, preview_ttl())?;
            drop(pending);
            serde_json::to_value(preview).map_err(|e| {
                CoreError::operation_failed(
                    "preview_encode_failed",
                    e.to_string(),
                    false,
                    "retry_later",
                )
            })
        }
    }
}

pub fn execute_interactive_import(req: CommandRequest) -> i32 {
    let Subcommand::Backup(BackupCommand::SettingsImport {
        input,
        interactive,
        non_interactive,
        source,
        options,
        ..
    }) = req.subcommand
    else {
        unreachable!();
    };
    if let Err(err) = ensure_interactive_tty(
        interactive,
        non_interactive,
        "settings import --apply requires --interactive and an active controlling terminal",
    ) {
        eprintln!("{err}");
        return err.exit_code();
    }
    let source = source.unwrap_or(SecretSource::InteractiveTty);
    if matches!(source, SecretSource::InteractiveTty) {
        if let Err(err) = ensure_interactive_tty(
            true,
            non_interactive,
            "Interactive import requires a controlling terminal",
        ) {
            eprintln!("{err}");
            return err.exit_code();
        }
    }
    let passphrase = match read_passphrase_value(&source, false, non_interactive) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("{err}");
            return err.exit_code();
        }
    };

    let loc = OwnedProfileLocation::from_env(req.config_dir, req.profile);
    let rt = match ProfileRuntime::open(&loc.as_input(), "cli") {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("{err}");
            return err.exit_code();
        }
    };
    let exit_code = run_interactive_import_session(&rt, &input, &passphrase, options);
    rt.shutdown(Duration::from_secs(2));
    exit_code
}

fn run_interactive_import_session(
    rt: &ProfileRuntime<OsKeyringBackend>,
    input: &std::path::Path,
    passphrase: &str,
    mut options: RestoreOptions,
) -> i32 {
    let ttl = preview_ttl();
    let (preview, mut pending) =
        match rt.prepare_settings_import(input, passphrase, options.clone(), ttl) {
            Ok(pair) => pair,
            Err(err) => {
                eprintln!("{err}");
                return err.exit_code();
            }
        };
    let mut current_id = preview.id.clone();
    print_preview("Import preview", &preview);

    let watcher = match SignalWatcher::install() {
        Ok(w) => w,
        Err(err) => {
            eprintln!("Failed to initialize signal handler: {err}");
            return 1;
        }
    };

    loop {
        let line = match read_tty_line(
            "Apply changes? [yes/no/sections <list>/paths <preserve|source>/remap <k=v>/show]: ",
            Some(&watcher),
        ) {
            Ok(l) => l,
            Err(err) => {
                eprintln!("{err}");
                return err.exit_code();
            }
        };
        let cmd = line.trim();
        match parse_interactive_action(cmd, &options) {
            InteractiveAction::Apply => {
                return match rt.commit_pending_import(pending, &current_id) {
                    Ok(_) => {
                        println!("Settings import applied (preview ID: {current_id}).");
                        0
                    }
                    Err(err) => {
                        eprintln!("{err}");
                        err.exit_code()
                    }
                };
            }
            InteractiveAction::Cancel => {
                eprintln!("Settings import cancelled without changes.");
                return 1;
            }
            InteractiveAction::Show => {
                println!("Preview ID: {current_id}");
            }
            InteractiveAction::UpdateOptions(next_opts) => {
                match rt.refresh_pending_import_preview(&mut pending, next_opts.clone(), ttl) {
                    Ok(refreshed) => {
                        options = next_opts;
                        current_id = refreshed.id.clone();
                        print_preview("Updated preview", &refreshed);
                    }
                    Err(err) if err.code == "INVALID_ARGUMENTS" => {
                        eprintln!("{}", err.message);
                    }
                    Err(err) => {
                        eprintln!("{err}");
                        return err.exit_code();
                    }
                }
            }
            InteractiveAction::Invalid(msg) => {
                eprintln!("{msg}");
            }
        }
    }
}

enum InteractiveAction {
    Apply,
    Cancel,
    Show,
    UpdateOptions(RestoreOptions),
    Invalid(String),
}

fn parse_interactive_action(line: &str, current: &RestoreOptions) -> InteractiveAction {
    if line.eq_ignore_ascii_case("yes") || line.eq_ignore_ascii_case("apply") {
        return InteractiveAction::Apply;
    }
    if line.is_empty()
        || line.eq_ignore_ascii_case("no")
        || line.eq_ignore_ascii_case("abort")
        || line.eq_ignore_ascii_case("quit")
    {
        return InteractiveAction::Cancel;
    }
    if line.eq_ignore_ascii_case("show") {
        return InteractiveAction::Show;
    }
    if let Some(spec) = line.strip_prefix("sections ") {
        let mut next = current.clone();
        next.shared = false;
        next.servers = false;
        next.local = false;
        next.secrets = false;
        let trimmed = spec.trim();
        if !trimmed.eq_ignore_ascii_case("none") {
            for item in trimmed.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                match item {
                    "shared" => next.shared = true,
                    "servers" => next.servers = true,
                    "local" => next.local = true,
                    "secrets" => next.secrets = true,
                    other => {
                        return InteractiveAction::Invalid(format!(
                            "Unknown section '{other}'; expected comma-separated shared,servers,local,secrets or none"
                        ));
                    }
                }
            }
        }
        return InteractiveAction::UpdateOptions(next);
    }
    if line == "paths preserve" {
        let mut next = current.clone();
        next.preserve_paths = true;
        return InteractiveAction::UpdateOptions(next);
    }
    if line == "paths source" {
        let mut next = current.clone();
        next.preserve_paths = false;
        return InteractiveAction::UpdateOptions(next);
    }
    if let Some(pair) = line.strip_prefix("remap ") {
        let Some((k, v)) = pair.trim().split_once('=') else {
            return InteractiveAction::Invalid("Expected: remap <key>=<path>".into());
        };
        if k.trim().is_empty() || v.trim().is_empty() {
            return InteractiveAction::Invalid("Expected non-empty remap key and path".into());
        }
        let mut next = current.clone();
        next.path_overrides
            .insert(k.trim().to_string(), v.trim().to_string());
        return InteractiveAction::UpdateOptions(next);
    }
    InteractiveAction::Invalid(
        "Enter 'yes', 'no', 'show', 'sections <shared,servers,local,secrets>', 'paths <preserve|source>', or 'remap <key>=<path>'".into(),
    )
}

fn print_preview(heading: &str, preview: &Preview) {
    println!("{heading}");
    println!("Preview ID: {}", preview.id);
    println!(
        "Source: {} v{} | Servers: {} | Credentials: {} set, {} deleted, {} kept",
        preview.source_platform,
        preview.source_version,
        preview.servers,
        preview.credentials_set,
        preview.credentials_deleted,
        preview.credentials_kept
    );
    for section in &preview.sections {
        println!(
            "  - {}: {} (changed={})",
            section.name, section.state, section.changed
        );
    }
    if !preview.warnings.is_empty() {
        for warn in &preview.warnings {
            println!("  warning: {warn}");
        }
    }
}
