use std::collections::HashMap;
use std::fs;
use std::net::TcpListener as StdTcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use russh::keys::{load_secret_key, PublicKey};
use russh::server::{
    Auth, ChannelOpenHandle, Msg, RunningServerHandle, Server, Session, VerifiedPublicKey,
};
use russh::{Channel, ChannelId, ChannelMsg};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Command as TokioCommand;
use tokio::sync::Mutex;

fn claimed_ports() -> &'static std::sync::Mutex<std::collections::HashSet<u16>> {
    static CLAIMED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<u16>>> =
        std::sync::OnceLock::new();
    CLAIMED.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

fn next_odd_port_offset() -> usize {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let base = (std::process::id() as usize).wrapping_mul(131) % 15_000;
    base.wrapping_add(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

fn bind_unique_odd_listener() -> StdTcpListener {
    for _ in 0..20_000 {
        let candidate = 15_001u16 + ((next_odd_port_offset() % 20_000) * 2) as u16;
        if claimed_ports().lock().unwrap().contains(&candidate) {
            continue;
        }
        if let Ok(listener) = StdTcpListener::bind(("127.0.0.1", candidate)) {
            if claimed_ports().lock().unwrap().insert(candidate) {
                return listener;
            }
        }
    }
    panic!("unable to allocate unique odd loopback port");
}

#[allow(dead_code)]
pub fn free_port() -> u16 {
    let listener = bind_unique_odd_listener();
    listener.local_addr().unwrap().port()
}

pub struct SshdFixture {
    _dir: TempDir,
    shutdown: Option<RunningServerHandle>,
    server_thread: Option<std::thread::JoinHandle<()>>,
    pub port: u16,
    pub user: String,
    pub host_pub_algo_and_key: String,
    pub user_key: PathBuf,
    #[allow(dead_code)]
    pub known_hosts: PathBuf,
    pub boundary_script: PathBuf,
    pub remote_sandbox: PathBuf,
}

impl SshdFixture {
    pub fn start() -> Self {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (host_key_path, user_key) = generate_ed25519_keypairs(root);
        let host_pub_algo_and_key = read_host_pub_entry(&root.join("host_ed25519.pub"));
        let host_key = load_secret_key(&host_key_path, None).expect("load host_ed25519");
        let allowed_pubkey = load_secret_key(&user_key, None)
            .expect("load user_ed25519")
            .public_key()
            .clone();

        let remote_sandbox = root.join("remote_sandbox");
        fs::create_dir_all(&remote_sandbox).unwrap();

        let std_listener = bind_unique_odd_listener();
        std_listener.set_nonblocking(true).unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let user = current_username();
        let known_hosts = root.join("known_hosts");
        let fixture_kh = format!("[127.0.0.1]:{port} {host_pub_algo_and_key}\n");
        fs::write(&known_hosts, fixture_kh).unwrap();

        let (shutdown, server_thread) =
            spawn_ssh_server_thread(std_listener, host_key, user.clone(), allowed_pubkey);
        let boundary_script = root.join("fake-boundary");
        let script = FAKE_BOUNDARY_TEMPLATE.replace("__SSHD_PORT__", &port.to_string());
        fs::write(&boundary_script, script).unwrap();
        fs::set_permissions(&boundary_script, fs::Permissions::from_mode(0o755)).unwrap();

        Self {
            _dir: dir,
            shutdown: Some(shutdown),
            server_thread: Some(server_thread),
            port,
            user,
            host_pub_algo_and_key,
            user_key,
            known_hosts,
            boundary_script,
            remote_sandbox,
        }
    }

    pub fn write_known_hosts(&self, path: &Path, ports: &[u16]) {
        let mut content = String::new();
        for p in ports {
            content.push_str(&format!("[127.0.0.1]:{p} {}\n", self.host_pub_algo_and_key));
        }
        fs::write(path, content).unwrap();
    }
}

impl Drop for SshdFixture {
    fn drop(&mut self) {
        if let Some(handle) = self.shutdown.take() {
            handle.shutdown("fixture shutdown".to_string());
        }
        if let Some(thread) = self.server_thread.take() {
            let _ = thread.join();
        }
    }
}

fn generate_ed25519_keypairs(root: &Path) -> (PathBuf, PathBuf) {
    let host_key = root.join("host_ed25519");
    let user_key = root.join("user_ed25519");
    for path in [&host_key, &user_key] {
        let status = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success(), "ssh-keygen failed");
    }
    fs::set_permissions(&user_key, fs::Permissions::from_mode(0o600)).unwrap();
    (host_key, user_key)
}

fn read_host_pub_entry(path: &Path) -> String {
    let raw = fs::read_to_string(path).unwrap();
    let parts: Vec<&str> = raw.split_whitespace().collect();
    format!("{} {}", parts[0], parts[1])
}

fn current_username() -> String {
    if let Ok(user) = std::env::var("USER") {
        if !user.trim().is_empty() {
            return user;
        }
    }
    let out = Command::new("id").arg("-un").output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn spawn_ssh_server_thread(
    std_listener: StdTcpListener,
    host_key: russh::keys::PrivateKey,
    expected_user: String,
    allowed_pubkey: PublicKey,
) -> (RunningServerHandle, std::thread::JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = TcpListener::from_std(std_listener).unwrap();
            let config = Arc::new(russh::server::Config {
                auth_rejection_time: Duration::from_millis(10),
                auth_rejection_time_initial: Some(Duration::from_millis(0)),
                inactivity_timeout: Some(Duration::from_secs(120)),
                keys: vec![host_key],
                ..Default::default()
            });
            let mut srv = TestSshServer {
                expected_user,
                allowed_pubkey,
            };
            let running = srv.run_on_socket(config, &listener);
            let _ = tx.send(running.handle());
            let _ = running.await;
        });
    });
    let handle = rx.recv().expect("fixture server handle");
    (handle, thread)
}

#[derive(Clone)]
struct TestSshServer {
    expected_user: String,
    allowed_pubkey: PublicKey,
}

impl russh::server::Server for TestSshServer {
    type Handler = TestSshSession;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        TestSshSession {
            expected_user: self.expected_user.clone(),
            allowed_pubkey: self.allowed_pubkey.clone(),
            channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

struct TestSshSession {
    expected_user: String,
    allowed_pubkey: PublicKey,
    channels: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
}

impl TestSshSession {
    async fn take_channel(&self, id: ChannelId) -> Option<Channel<Msg>> {
        self.channels.lock().await.remove(&id)
    }
}

impl russh::server::Handler for TestSshSession {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        if user == self.expected_user && key.public_key.key_data() == self.allowed_pubkey.key_data()
        {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.lock().await.insert(channel.id(), channel);
        let _ = reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        _variable_name: &str,
        _variable_value: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp" {
            if let Some(channel) = self.take_channel(channel_id).await {
                session.channel_success(channel_id)?;
                spawn_child_bridge(
                    channel,
                    session.handle(),
                    channel_id,
                    "/usr/lib/openssh/sftp-server",
                    Vec::new(),
                );
                return Ok(());
            }
        }
        session.channel_failure(channel_id)?;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel_id: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(channel) = self.take_channel(channel_id).await {
            session.channel_success(channel_id)?;
            let cmd = String::from_utf8_lossy(data).into_owned();
            spawn_child_bridge(
                channel,
                session.handle(),
                channel_id,
                "/bin/sh",
                vec!["-c".to_string(), cmd],
            );
        } else {
            session.channel_failure(channel_id)?;
        }
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel_id: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(channel) = self.take_channel(channel_id).await {
            session.channel_success(channel_id)?;
            spawn_child_bridge(channel, session.handle(), channel_id, "/bin/sh", Vec::new());
        } else {
            session.channel_failure(channel_id)?;
        }
        Ok(())
    }
}

fn spawn_child_bridge(
    channel: Channel<Msg>,
    handle: russh::server::Handle,
    channel_id: ChannelId,
    program: &str,
    args: Vec<String>,
) {
    let program = program.to_string();
    tokio::spawn(async move {
        let mut child = match TokioCommand::new(&program)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(_) => {
                let _ = handle.exit_status_request(channel_id, 127).await;
                let _ = handle.close(channel_id).await;
                return;
            }
        };
        let child_stdin = child.stdin.take().unwrap();
        let child_stdout = child.stdout.take().unwrap();
        let child_stderr = child.stderr.take().unwrap();
        let (read_half, _write_half) = channel.split();
        let stdin_task = tokio::spawn(pump_channel_to_stdin(read_half, child_stdin));
        let out_task = tokio::spawn(pump_pipe_to_channel(
            child_stdout,
            handle.clone(),
            channel_id,
            None,
        ));
        let err_task = tokio::spawn(pump_pipe_to_channel(
            child_stderr,
            handle.clone(),
            channel_id,
            Some(1),
        ));
        let _ = out_task.await;
        let _ = err_task.await;
        let code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(1) as u32;
        let _ = handle.exit_status_request(channel_id, code).await;
        let _ = handle.eof(channel_id).await;
        let _ = handle.close(channel_id).await;
        stdin_task.abort();
    });
}

async fn pump_channel_to_stdin(
    mut read_half: russh::ChannelReadHalf,
    mut stdin: tokio::process::ChildStdin,
) {
    while let Some(msg) = read_half.wait().await {
        match msg {
            ChannelMsg::Data { data } => {
                if stdin.write_all(&data).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }
    drop(stdin);
}

async fn pump_pipe_to_channel<R: AsyncRead + Unpin>(
    mut reader: R,
    handle: russh::server::Handle,
    channel_id: ChannelId,
    ext: Option<u32>,
) {
    let mut buf = vec![0u8; 16384];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let chunk = buf[..n].to_vec();
                let res = match ext {
                    Some(code) => handle.extended_data(channel_id, code, chunk).await,
                    None => handle.data(channel_id, chunk).await,
                };
                if res.is_err() {
                    break;
                }
            }
        }
    }
}

#[allow(dead_code)]
pub fn seed_named_profile(
    root: &Path,
    profile: &str,
    cfg: Option<serde_json::Value>,
    sync: serde_json::Value,
    servers: serde_json::Value,
) {
    let pdir = root.join("profiles").join(profile);
    fs::create_dir_all(&pdir).unwrap();
    if let Some(c) = cfg {
        fs::write(pdir.join("config.json"), c.to_string()).unwrap();
    }
    fs::write(pdir.join("sync-local.json"), sync.to_string()).unwrap();
    fs::write(pdir.join("servers.json"), servers.to_string()).unwrap();
}

#[allow(dead_code)]
pub fn seed_default_boundary_profile(
    cfg_dir: &Path,
    servers: serde_json::Value,
    boundary_cli_path: &Path,
) {
    seed_named_profile(
        cfg_dir,
        "default",
        None,
        serde_json::json!({ "boundaryCliPath": boundary_cli_path.to_string_lossy() }),
        servers,
    );
}

const FAKE_BOUNDARY_TEMPLATE: &str = r#"#!/usr/bin/env python3
import os, signal, socket, sys, threading, time

args = sys.argv[1:]
if not args or args[0] != "connect":
    sys.exit(2)

target_id = ""
listen_port = 0
i = 1
while i < len(args):
    if args[i] == "-target-id" and i + 1 < len(args):
        target_id = args[i + 1]
        i += 2
    elif args[i] == "-listen-port" and i + 1 < len(args):
        listen_port = int(args[i + 1])
        i += 2
    else:
        i += 1

if target_id == "early-exit":
    sys.stderr.write("simulated early boundary failure\n")
    sys.exit(1)

if target_id == "hang-no-listen":
    signal.signal(signal.SIGTERM, lambda *_: os._exit(0))
    while True:
        time.sleep(0.2)

def pipe(src, dst):
    try:
        while True:
            data = src.recv(16384)
            if not data:
                break
            dst.sendall(data)
    except OSError:
        pass
    finally:
        try:
            dst.shutdown(socket.SHUT_WR)
        except OSError:
            pass

def handle_conn(client):
    upstream = None
    try:
        upstream = socket.create_connection(("127.0.0.1", __SSHD_PORT__))
        t1 = threading.Thread(target=pipe, args=(client, upstream), daemon=True)
        t2 = threading.Thread(target=pipe, args=(upstream, client), daemon=True)
        t1.start()
        t2.start()
        t1.join()
        t2.join()
    except OSError:
        pass
    finally:
        for sock in (client, upstream):
            if sock is not None:
                try:
                    sock.close()
                except OSError:
                    pass

srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", listen_port))
srv.listen(32)
signal.signal(signal.SIGTERM, lambda *_: os._exit(0))

while True:
    client, _ = srv.accept()
    threading.Thread(target=handle_conn, args=(client,), daemon=True).start()
"#;
