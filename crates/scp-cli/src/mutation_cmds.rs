use crate::args::MutationCommand;
use crate::secret_input::{
    ensure_interactive_tty, read_json_payload, read_secret_value, JsonInputSource, SecretSource,
};
use scp_core::error::CoreError;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::OsKeyringBackend;
use serde_json::Value;
use zeroize::Zeroizing;

pub enum PreparedMutation {
    JsonMutation {
        cmd: MutationOp,
        payload: Option<Value>,
        secret: Option<Zeroizing<String>>,
    },
}

pub enum MutationOp {
    ServerUpsert {
        preview: bool,
        expected_revision: Option<String>,
    },
    ServerDelete {
        server_id: String,
        preview: bool,
        expected_revision: Option<String>,
    },
    ServerCredentialsSet {
        server_id: String,
    },
    ServerCredentialsClear {
        server_id: String,
    },
    ServerRdp {
        server_id: String,
    },
    ConfigSave {
        preview: bool,
        expected_revision: Option<String>,
    },
    ConfigPatch {
        preview: bool,
        expected_revision: Option<String>,
    },
    ConfigSecretSet {
        key: String,
    },
    ConfigSecretClear {
        key: String,
    },
    ConfigSyncPush {
        repo_root: Option<std::path::PathBuf>,
        preview: bool,
        expected_revision: Option<String>,
    },
    SettingsPatch {
        preview: bool,
        expected_revision: Option<String>,
    },
}

pub fn prepare_mutation(cmd: MutationCommand) -> Result<PreparedMutation, CoreError> {
    match cmd {
        MutationCommand::ServerUpsert {
            input,
            preview,
            expected_revision,
        } => {
            let payload = read_json_payload(&input)?;
            Ok(PreparedMutation::JsonMutation {
                cmd: MutationOp::ServerUpsert {
                    preview,
                    expected_revision,
                },
                payload: Some(payload),
                secret: None,
            })
        }
        MutationCommand::ServerDelete {
            server_id,
            preview,
            expected_revision,
        } => Ok(PreparedMutation::JsonMutation {
            cmd: MutationOp::ServerDelete {
                server_id,
                preview,
                expected_revision,
            },
            payload: None,
            secret: None,
        }),
        MutationCommand::ServerCredentialsSet {
            server_id,
            source,
            non_interactive,
        } => {
            let secret = read_secret_if_tty_ready(&source, non_interactive)?;
            Ok(PreparedMutation::JsonMutation {
                cmd: MutationOp::ServerCredentialsSet { server_id },
                payload: None,
                secret: Some(secret),
            })
        }
        MutationCommand::ServerCredentialsClear { server_id } => {
            Ok(PreparedMutation::JsonMutation {
                cmd: MutationOp::ServerCredentialsClear { server_id },
                payload: None,
                secret: None,
            })
        }
        MutationCommand::ServerRdp {
            server_id,
            interactive,
            non_interactive,
        } => {
            ensure_interactive_tty(
                interactive,
                non_interactive,
                "server rdp requires --interactive in an active terminal",
            )?;
            Ok(PreparedMutation::JsonMutation {
                cmd: MutationOp::ServerRdp { server_id },
                payload: None,
                secret: None,
            })
        }
        MutationCommand::ConfigSave {
            input,
            preview,
            expected_revision,
        } => Ok(PreparedMutation::JsonMutation {
            cmd: MutationOp::ConfigSave {
                preview,
                expected_revision,
            },
            payload: Some(read_json_or_stdin(&input)?),
            secret: None,
        }),
        MutationCommand::ConfigPatch {
            input,
            preview,
            expected_revision,
        } => Ok(PreparedMutation::JsonMutation {
            cmd: MutationOp::ConfigPatch {
                preview,
                expected_revision,
            },
            payload: Some(read_json_or_stdin(&input)?),
            secret: None,
        }),
        MutationCommand::ConfigSecretSet {
            key,
            source,
            non_interactive,
        } => {
            let secret = read_secret_if_tty_ready(&source, non_interactive)?;
            Ok(PreparedMutation::JsonMutation {
                cmd: MutationOp::ConfigSecretSet { key },
                payload: None,
                secret: Some(secret),
            })
        }
        MutationCommand::ConfigSecretClear { key } => Ok(PreparedMutation::JsonMutation {
            cmd: MutationOp::ConfigSecretClear { key },
            payload: None,
            secret: None,
        }),
        MutationCommand::ConfigSyncPush {
            repo_root,
            preview,
            expected_revision,
        } => Ok(PreparedMutation::JsonMutation {
            cmd: MutationOp::ConfigSyncPush {
                repo_root,
                preview,
                expected_revision,
            },
            payload: None,
            secret: None,
        }),
        MutationCommand::SettingsPatch {
            input,
            preview,
            expected_revision,
        } => Ok(PreparedMutation::JsonMutation {
            cmd: MutationOp::SettingsPatch {
                preview,
                expected_revision,
            },
            payload: Some(read_json_payload(&input)?),
            secret: None,
        }),
    }
}

fn read_json_or_stdin(src: &JsonInputSource) -> Result<Value, CoreError> {
    read_json_payload(src)
}

fn read_secret_if_tty_ready(
    source: &SecretSource,
    non_interactive: bool,
) -> Result<Zeroizing<String>, CoreError> {
    if matches!(source, SecretSource::InteractiveTty) {
        ensure_interactive_tty(
            true,
            non_interactive,
            "Interactive secret entry requires --interactive and a controlling terminal",
        )?;
    }
    read_secret_value(source, "Enter secret: ", non_interactive)
}

pub fn execute_mutation(
    rt: &ProfileRuntime<OsKeyringBackend>,
    prepared: PreparedMutation,
) -> Result<Value, CoreError> {
    let PreparedMutation::JsonMutation {
        cmd,
        payload,
        secret,
    } = prepared;
    match cmd {
        MutationOp::ServerUpsert {
            preview,
            expected_revision,
        } => rt.upsert_server(
            &payload.expect("payload validated"),
            preview,
            expected_revision.as_deref(),
        ),
        MutationOp::ServerDelete {
            server_id,
            preview,
            expected_revision,
        } => rt.delete_server(&server_id, preview, expected_revision.as_deref()),
        MutationOp::ServerCredentialsSet { server_id } => {
            rt.set_server_ssh_password(&server_id, secret.as_deref().expect("secret validated"))
        }
        MutationOp::ServerCredentialsClear { server_id } => {
            rt.clear_server_ssh_password(&server_id)
        }
        MutationOp::ServerRdp { server_id } => rt.launch_server_rdp(&server_id, true),
        MutationOp::ConfigSave {
            preview,
            expected_revision,
        } => rt.save_app_config(
            &payload.expect("payload validated"),
            preview,
            expected_revision.as_deref(),
        ),
        MutationOp::ConfigPatch {
            preview,
            expected_revision,
        } => rt.patch_app_config(
            &payload.expect("payload validated"),
            preview,
            expected_revision.as_deref(),
        ),
        MutationOp::ConfigSecretSet { key } => {
            rt.set_config_secret(&key, secret.as_deref().expect("secret validated"))
        }
        MutationOp::ConfigSecretClear { key } => rt.clear_config_secret(&key),
        MutationOp::ConfigSyncPush {
            repo_root,
            preview,
            expected_revision,
        } => rt.push_config_sync(
            repo_root.as_deref().map(|p| p.to_string_lossy()).as_deref(),
            preview,
            expected_revision.as_deref(),
        ),
        MutationOp::SettingsPatch {
            preview,
            expected_revision,
        } => rt.patch_sync_settings(
            &payload.expect("payload validated"),
            preview,
            expected_revision.as_deref(),
        ),
    }
}
