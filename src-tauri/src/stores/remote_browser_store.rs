use std::{
    collections::HashMap,
    io::BufReader,
    process::{Child, ChildStderr, ChildStdin, ChildStdout},
    sync::{Arc, Mutex},
};

use ssh2::Session as SshSession;

pub enum RemoteBrowserTransport {
    Sftp(SshSession),
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    Cli(CliRemoteBrowserSession),
}

pub struct CliRemoteBrowserSession {
    pub io: Mutex<CliRemoteBrowserIo>,
}

pub struct CliRemoteBrowserIo {
    pub child: Child,
    pub stdin: ChildStdin,
    pub stdout: BufReader<ChildStdout>,
    pub stderr: ChildStderr,
    pub next_marker_id: u64,
}

pub struct RemoteBrowserSessionState {
    pub server_id: String,
    pub root_path: String,
    pub transport: RemoteBrowserTransport,
}

pub struct RemoteBrowserStore {
    pub sessions: Mutex<HashMap<String, Arc<RemoteBrowserSessionState>>>,
    pub next_id: Mutex<u64>,
}

impl Default for RemoteBrowserStore {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
        }
    }
}
