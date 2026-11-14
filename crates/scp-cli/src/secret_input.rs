use scp_core::error::CoreError;
use scp_core::tty::{is_interactive_tty, read_tty_secret, SignalWatcher};
use serde_json::Value;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_SECRET_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    File(PathBuf),
    Stdin,
    InteractiveTty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonInputSource {
    File(PathBuf),
    Stdin,
}

pub fn ensure_interactive_tty(
    interactive: bool,
    non_interactive: bool,
    msg: &str,
) -> Result<(), CoreError> {
    if !interactive || non_interactive || !is_interactive_tty() {
        return Err(CoreError::capability_unavailable(
            "interactive_tty_required",
            msg,
            "run_interactively",
        ));
    }
    Ok(())
}

pub fn read_json_payload(src: &JsonInputSource) -> Result<Value, CoreError> {
    let raw = match src {
        JsonInputSource::File(path) => read_bounded_file(path, MAX_JSON_BYTES, "JSON payload")?,
        JsonInputSource::Stdin => {
            read_bounded_reader(&mut io::stdin().lock(), MAX_JSON_BYTES, "JSON payload")?
        }
    };
    serde_json::from_slice(&raw).map_err(|e| {
        CoreError::invalid_arguments("invalid_json", format!("Invalid JSON payload: {e}"))
    })
}

pub fn read_secret_value(
    source: &SecretSource,
    prompt: &str,
    non_interactive: bool,
) -> Result<Zeroizing<String>, CoreError> {
    match source {
        SecretSource::File(path) => read_secret_from_file(path),
        SecretSource::Stdin => read_secret_from_stdin(),
        SecretSource::InteractiveTty => {
            ensure_interactive_tty(
                true,
                non_interactive,
                "Interactive secret entry requires --interactive and a controlling terminal",
            )?;
            let watcher = SignalWatcher::install().map_err(|e| {
                CoreError::operation_failed(
                    "signal_setup_failed",
                    format!("Failed to initialize signal handler: {e}"),
                    false,
                    "retry_later",
                )
            })?;
            let secret = read_tty_secret(prompt, Some(&watcher))?;
            if secret.is_empty() {
                return Err(CoreError::invalid_arguments(
                    "empty_secret",
                    "Secret value must not be empty",
                ));
            }
            Ok(secret)
        }
    }
}

pub fn read_passphrase_value(
    source: &SecretSource,
    confirm_on_tty: bool,
    non_interactive: bool,
) -> Result<Zeroizing<String>, CoreError> {
    let prompt = "Enter backup passphrase: ";
    let pass = read_secret_value(source, prompt, non_interactive)?;
    if confirm_on_tty && pass.chars().count() < 12 {
        return Err(CoreError::invalid_arguments(
            "invalid_passphrase",
            "Backup passphrase must be at least 12 characters",
        ));
    }
    if confirm_on_tty && matches!(source, SecretSource::InteractiveTty) {
        let watcher = SignalWatcher::install().map_err(|e| {
            CoreError::operation_failed(
                "signal_setup_failed",
                format!("Failed to initialize signal handler: {e}"),
                false,
                "retry_later",
            )
        })?;
        let confirm = read_tty_secret("Confirm backup passphrase: ", Some(&watcher))?;
        if *confirm != *pass {
            return Err(CoreError::invalid_arguments(
                "passphrase_mismatch",
                "Backup passphrases do not match",
            ));
        }
    }
    Ok(pass)
}

fn read_secret_from_file(path: &Path) -> Result<Zeroizing<String>, CoreError> {
    let bytes = Zeroizing::new(read_bounded_file(path, MAX_SECRET_BYTES, "secret")?);
    decode_secret_bytes(bytes)
}

fn read_secret_from_stdin() -> Result<Zeroizing<String>, CoreError> {
    let bytes = Zeroizing::new(read_bounded_reader(
        &mut io::stdin().lock(),
        MAX_SECRET_BYTES,
        "secret",
    )?);
    decode_secret_bytes(bytes)
}

fn decode_secret_bytes(bytes: Zeroizing<Vec<u8>>) -> Result<Zeroizing<String>, CoreError> {
    let raw = std::str::from_utf8(&bytes).map_err(|_| {
        CoreError::invalid_arguments(
            "invalid_secret_encoding",
            "Secret input must be valid UTF-8",
        )
    })?;
    let trimmed = raw
        .strip_suffix("\r\n")
        .or_else(|| raw.strip_suffix('\n'))
        .unwrap_or(raw);
    let out = Zeroizing::new(trimmed.to_string());
    if out.is_empty() {
        return Err(CoreError::invalid_arguments(
            "empty_secret",
            "Secret value must not be empty",
        ));
    }
    Ok(out)
}

fn read_bounded_file(path: &Path, max_bytes: usize, label: &str) -> Result<Vec<u8>, CoreError> {
    let mut file = fs::File::open(path).map_err(|e| {
        CoreError::invalid_arguments(
            "input_file_unreadable",
            format!("Unable to open {label} file '{}': {e}", path.display()),
        )
    })?;
    read_bounded_reader(&mut file, max_bytes, label)
}

fn read_bounded_reader(
    reader: &mut dyn Read,
    max_bytes: usize,
    label: &str,
) -> Result<Vec<u8>, CoreError> {
    let mut buf = Vec::new();
    let mut limited = reader.take((max_bytes + 1) as u64);
    limited.read_to_end(&mut buf).map_err(|e| {
        CoreError::invalid_arguments("input_unreadable", format!("Failed reading {label}: {e}"))
    })?;
    if buf.len() > max_bytes {
        return Err(CoreError::invalid_arguments(
            "input_too_large",
            format!("{label} exceeds maximum size of {max_bytes} bytes"),
        ));
    }
    Ok(buf)
}
