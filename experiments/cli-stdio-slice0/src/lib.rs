pub mod boundary_spike;
pub mod keyring_spike;
pub mod profile_lock;
pub mod stdio_host;
pub mod supervisor_spike;
pub mod vpn_spike;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpikeError {
    pub code: &'static str,
    pub reason: &'static str,
    pub message: String,
    pub exit_code: u8,
}

impl SpikeError {
    pub fn new(
        code: &'static str,
        reason: &'static str,
        message: impl Into<String>,
        exit_code: u8,
    ) -> Self {
        Self {
            code,
            reason,
            message: message.into(),
            exit_code,
        }
    }
}
