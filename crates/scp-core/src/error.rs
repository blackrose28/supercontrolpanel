use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub reason: &'static str,
    pub next_action: &'static str,
    pub outcome: &'static str,
}

impl CoreError {
    pub fn new(
        code: &'static str,
        reason: &'static str,
        message: impl Into<String>,
        retryable: bool,
        next_action: &'static str,
        outcome: &'static str,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            reason,
            next_action,
            outcome,
        }
    }

    pub fn invalid_arguments(reason: &'static str, message: impl Into<String>) -> Self {
        Self::new(
            "INVALID_ARGUMENTS",
            reason,
            message,
            false,
            "inspect_schema",
            "not_started",
        )
    }

    pub fn policy_denied(
        reason: &'static str,
        message: impl Into<String>,
        next_action: &'static str,
    ) -> Self {
        Self::new(
            "POLICY_DENIED",
            reason,
            message,
            false,
            next_action,
            "not_started",
        )
    }

    pub fn capability_unavailable(
        reason: &'static str,
        message: impl Into<String>,
        next_action: &'static str,
    ) -> Self {
        Self::new(
            "CAPABILITY_UNAVAILABLE",
            reason,
            message,
            false,
            next_action,
            "not_started",
        )
    }

    pub fn conflict(
        reason: &'static str,
        message: impl Into<String>,
        retryable: bool,
        next_action: &'static str,
    ) -> Self {
        Self::new(
            "CONFLICT",
            reason,
            message,
            retryable,
            next_action,
            "not_started",
        )
    }

    pub fn not_found(
        reason: &'static str,
        message: impl Into<String>,
        next_action: &'static str,
    ) -> Self {
        Self::new(
            "NOT_FOUND",
            reason,
            message,
            false,
            next_action,
            "not_started",
        )
    }

    pub fn timeout(reason: &'static str, message: impl Into<String>) -> Self {
        Self::new(
            "TIMEOUT",
            reason,
            message,
            true,
            "inspect_status",
            "unknown",
        )
    }

    pub fn operation_failed(
        reason: &'static str,
        message: impl Into<String>,
        retryable: bool,
        next_action: &'static str,
    ) -> Self {
        Self::new(
            "OPERATION_FAILED",
            reason,
            message,
            retryable,
            next_action,
            "not_started",
        )
    }

    pub fn interrupted(message: impl Into<String>) -> Self {
        Self::new(
            "INTERRUPTED",
            "interrupted",
            message,
            false,
            "retry_later",
            "partial",
        )
    }

    pub fn exit_code(&self) -> i32 {
        match self.code {
            "INVALID_ARGUMENTS" => 2,
            "POLICY_DENIED" => 3,
            "CAPABILITY_UNAVAILABLE" => 4,
            "CONFLICT" => 5,
            "NOT_FOUND" => 6,
            "TIMEOUT" => 7,
            "INTERRUPTED" => 130,
            _ => 1,
        }
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}): {}", self.code, self.reason, self.message)
    }
}

impl std::error::Error for CoreError {}
