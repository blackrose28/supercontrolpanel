//! Caddy launch preparation shared by desktop and future foreground hosts.

use portable_pty::CommandBuilder;
use std::path::PathBuf;

/// Prepare the desktop's foreground Caddy command without launching or changing files.
///
/// Preserve the desktop's trim/existence checks. This does not parse the Caddyfile,
/// check executable availability, or guarantee paths still exist at launch time.
/// The host owns profile/session lifetime, output and cancellation policy.
pub fn build_caddy_command(folder: &str) -> Result<CommandBuilder, String> {
    let folder_trim = folder.trim();
    if folder_trim.is_empty() {
        return Err("folder cannot be empty".to_string());
    }
    let folder_path = PathBuf::from(folder_trim);
    if !folder_path.exists() {
        return Err(format!(
            "folder does not exist: {}",
            folder_path.to_string_lossy()
        ));
    }
    if !folder_path.join("Caddyfile").exists() {
        return Err(format!(
            "Caddyfile not found in folder: {}",
            folder_path.to_string_lossy()
        ));
    }
    let mut command = CommandBuilder::new("caddy");
    command.args(["run", "--config", "Caddyfile"]);
    command.cwd(folder_path);
    Ok(command)
}
