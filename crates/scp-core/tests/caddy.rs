use scp_core::caddy::build_caddy_command;
use std::{ffi::OsString, fs};

#[test]
fn rejects_empty_and_missing_folders_without_creating_them() {
    assert_eq!(
        build_caddy_command(" \t\n").unwrap_err(),
        "folder cannot be empty"
    );
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    assert_eq!(
        build_caddy_command(missing.to_str().unwrap()).unwrap_err(),
        format!("folder does not exist: {}", missing.display())
    );
    assert!(!missing.exists());
}

#[test]
fn requires_caddyfile_without_writing_configuration() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        build_caddy_command(root.path().to_str().unwrap()).unwrap_err(),
        format!("Caddyfile not found in folder: {}", root.path().display())
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    let file = root.path().join("ordinary-file");
    fs::write(&file, "untouched").unwrap();
    assert_eq!(
        build_caddy_command(file.to_str().unwrap()).unwrap_err(),
        format!("Caddyfile not found in folder: {}", file.display())
    );
    assert_eq!(fs::read_to_string(file).unwrap(), "untouched");
}

#[test]
fn builds_foreground_command_with_literal_working_directory_and_no_shell() {
    let root = tempfile::tempdir().unwrap();
    // Valid on Windows too; shell metacharacters must remain path characters.
    let folder = root.path().join("Caddy space & (literal) café");
    fs::create_dir(&folder).unwrap();
    let contents = b":8080 { respond test }\n";
    fs::write(folder.join("Caddyfile"), contents).unwrap();
    let command = build_caddy_command(&format!(" \t{}\n", folder.display())).unwrap();
    assert_eq!(
        command.get_argv(),
        &vec!["caddy", "run", "--config", "Caddyfile"]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
    );
    assert_eq!(command.get_cwd(), Some(&folder.into_os_string()));
    assert_eq!(
        fs::read(root.path().join("Caddy space & (literal) café/Caddyfile")).unwrap(),
        contents
    );
}
