//! `scpctl boundary auth` with automated OIDC: headless login against a loopback fake of the
//! controller + IdP, and the interactive fallback to the stock browser login.
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

const PASSWORD: &str = "cli-oidc-pw";
const LOGIN_FORM: &str = r#"<form action="/idp/login" method="post"><input name="username" type="text"><input name="password" type="password"></form>"#;

fn read_req(stream: &mut TcpStream) -> Option<(String, String)> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        let n = stream.read(&mut chunk).ok()?;
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf).to_string();
        if let Some(idx) = text.find("\r\n\r\n") {
            let head = text[..idx].to_string();
            let len = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if buf.len() >= idx + 4 + len {
                let body = String::from_utf8_lossy(&buf[idx + 4..idx + 4 + len]).to_string();
                return Some((head.lines().next().unwrap_or("").to_string(), body));
            }
        }
        if n == 0 {
            return None;
        }
    }
}

fn reply(status: &str, headers: &[String], body: &str) -> String {
    let mut out = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for h in headers {
        out.push_str(h);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    out.push_str(body);
    out
}

/// One loopback server playing both the controller and the IdP (same authority).
struct Fake {
    port: u16,
    stop: Arc<AtomicBool>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn start_fake(idp_supported: bool) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    std::thread::spawn(move || {
        let mut callback_done = false;
        while !stop2.load(Ordering::SeqCst) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            let Some((line, body)) = read_req(&mut stream) else {
                continue;
            };
            let out = if line.starts_with("POST /v1/auth-methods/amoidc_1:authenticate") {
                if body.contains("\"start\"") {
                    reply(
                        "200 OK",
                        &[],
                        &json!({"attributes": {
                            "auth_url": format!("http://127.0.0.1:{port}/idp/auth?state=S"),
                            "token_id": "tokid"}})
                        .to_string(),
                    )
                } else if callback_done {
                    reply(
                        "200 OK",
                        &[],
                        &json!({"attributes": {"token": "tbat_cli_headless"}}).to_string(),
                    )
                } else {
                    reply("400 Bad Request", &[], "{}")
                }
            } else if line.starts_with("GET /idp/auth") {
                if idp_supported {
                    reply("200 OK", &[], LOGIN_FORM)
                } else {
                    reply("200 OK", &[], "<title>Grant access?</title><p>consent</p>")
                }
            } else if line.starts_with("POST /idp/login") {
                if body.contains(&format!("password={PASSWORD}")) {
                    reply(
                        "302 Found",
                        &[format!(
                            "Location: http://127.0.0.1:{port}/v1/auth-methods/amoidc_1:authenticate:callback?code=c"
                        )],
                        "",
                    )
                } else {
                    reply("200 OK", &[], LOGIN_FORM)
                }
            } else if line.starts_with("GET /v1/auth-methods/amoidc_1:authenticate:callback") {
                callback_done = true;
                reply("200 OK", &[], "ok")
            } else {
                reply("404 Not Found", &[], "")
            };
            let _ = stream.write_all(out.as_bytes());
        }
    });
    Fake { port, stop }
}

fn seed_profile(root: &std::path::Path, cfg: Value, sync: Value) {
    let dir = root.join("profiles").join("oidc");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("config.json"), cfg.to_string()).unwrap();
    fs::write(dir.join("sync-local.json"), sync.to_string()).unwrap();
    fs::write(
        dir.join("servers.json"),
        json!({ "servers": [] }).to_string(),
    )
    .unwrap();
}

struct Env {
    tmp: TempDir,
}

impl Env {
    fn new(fake: &Fake) -> Self {
        let tmp = TempDir::new().unwrap();
        let script = tmp.path().join("fake-boundary");
        fs::write(
            &script,
            "#!/bin/sh\nprintf '{\"item\":{\"attributes\":{\"token\":\"tbat_from_stock_browser\"}}}'\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        seed_profile(
            &tmp.path().join("root"),
            json!({
                "app": {"defaultTerminalCols": 120, "defaultTerminalRows": 36},
                "boundary": {
                    "addr": format!("http://127.0.0.1:{}", fake.port),
                    "authMethodId": "amoidc_1",
                    "authKind": "oidc",
                    "oidcLoginMode": "automated",
                    "oidcUsername": "alice",
                    // Legacy plaintext is migrated into the (test) keyring on load.
                    "oidcPassword": PASSWORD
                }
            }),
            json!({ "boundaryCliPath": script }),
        );
        Self { tmp }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_scpctl"))
            .args([
                "--config-dir",
                self.tmp.path().join("root").to_str().unwrap(),
                "--profile",
                "oidc",
            ])
            .args(args)
            .env("SCPCTL_DESKTOP_CONFIG_DIR", self.tmp.path().join("desktop"))
            .env(
                "SCPCTL_TEST_KEYRING_FILE",
                self.tmp.path().join("keyring.json"),
            )
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn keyring_text(&self) -> String {
        fs::read_to_string(self.tmp.path().join("keyring.json")).unwrap_or_default()
    }
}

fn body(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout must be one JSON document: {e}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

#[test]
fn automated_oidc_logs_in_headlessly_without_interactive() {
    let fake = start_fake(true);
    let env = Env::new(&fake);
    let out = env.run(&["boundary", "auth", "--json"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr={stderr}");
    let b = body(&out);
    assert_eq!(b["data"]["state"], "authenticated");
    assert_eq!(b["data"]["boundaryTokenSource"], "app");
    assert!(env.keyring_text().contains("tbat_cli_headless"));
    let all = format!("{}{stderr}", String::from_utf8_lossy(&out.stdout));
    for secret in ["tbat_cli_headless", PASSWORD] {
        assert!(!all.contains(secret), "output leaked {secret}: {all}");
    }
}

#[test]
fn unrecognised_idp_fails_closed_non_interactive_and_falls_back_when_interactive() {
    let fake = start_fake(false);
    let env = Env::new(&fake);

    let out = env.run(&["boundary", "auth", "--json"]);
    assert!(!out.status.success());
    let b = body(&out);
    assert_eq!(b["error"]["reason"], "boundary_auth_failed");
    let msg = b["error"]["message"].as_str().unwrap();
    assert!(msg.contains("oidc_idp_unsupported"), "{msg}");
    assert!(msg.contains("Grant access?"), "{msg}");
    assert!(
        !env.keyring_text().contains("tbat_"),
        "no token may be stored"
    );

    let out = env.run(&["boundary", "auth", "--interactive", "--json"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr={stderr}");
    assert!(
        stderr.contains("falling back to the browser login"),
        "{stderr}"
    );
    assert!(
        stderr.contains("Complete the Boundary login in your browser"),
        "{stderr}"
    );
    assert_eq!(body(&out)["data"]["state"], "authenticated");
    assert!(env.keyring_text().contains("tbat_from_stock_browser"));
}

#[test]
fn wrong_password_is_not_retried_in_the_browser() {
    let fake = start_fake(true);
    let env = Env::new(&fake);
    // Break the stored password through the config path used by the fake's check.
    let cfg_path = env.tmp.path().join("root/profiles/oidc/config.json");
    let cfg = fs::read_to_string(&cfg_path)
        .unwrap()
        .replace(PASSWORD, "not-the-password");
    fs::write(&cfg_path, cfg).unwrap();
    let out = env.run(&["boundary", "auth", "--interactive", "--json"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("falling back"), "{stderr}");
    assert!(body(&out)["error"]["message"]
        .as_str()
        .unwrap()
        .contains("oidc_credentials_rejected"));
}
