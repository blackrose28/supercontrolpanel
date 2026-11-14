//! Shared MCP registry, schemas, standalone dispatcher, and optional SDK/stdio transport hosts.
pub mod contracts;
#[cfg(feature = "sdk")]
pub mod lifecycle;
#[cfg(feature = "sdk")]
pub mod protocol;
pub mod public_dto;
pub mod standalone;
#[cfg(feature = "stdio")]
pub mod stdio;
pub mod validation;

/// Backwards-compatible wrapper exposing the standalone schema catalog.
pub fn standalone_catalog() -> serde_json::Value {
    standalone::standalone_schema_catalog(None).unwrap_or_default()
}
