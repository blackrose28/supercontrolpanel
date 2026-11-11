//! Single-file reads for the shared configuration and inventory repositories.

use serde::de::DeserializeOwned;
use std::{fs, io::ErrorKind, path::Path};

pub(crate) fn read_json<T: DeserializeOwned>(
    path: &Path,
    context: &str,
    description: &str,
) -> Result<Option<T>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read {context}: {error}")),
    };
    serde_json::from_str(&raw).map(Some).map_err(|error| {
        // Serde's details can echo file values, including legacy credentials.
        format!(
            "parse {context}: invalid {description} at line {}, column {}",
            error.line(),
            error.column()
        )
    })
}
