//! Headless Boundary OIDC login end to end: real `curl` against loopback fakes of the Boundary
//! controller and a Keycloak-like IdP (no real IdP is ever contacted).
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

use scp_core::boundary_ops::BoundaryAuthOptions;
use scp_core::models::{
    BoundaryAuthKind, BoundaryOidcLoginMode, KEYRING_BOUNDARY_APP_TOKEN,
    KEYRING_BOUNDARY_OIDC_OTP_SECRET, KEYRING_BOUNDARY_OIDC_PASSWORD,
};
use scp_core::openvpn_ops::generate_totp_code_at_epoch;
use scp_core::profile::ProfileLocationInput;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::MemoryKeyringBackend;
use scp_core::supervisor::CancelToken;

const USER: &str = "alice";
const PASSWORD: &str = "pw\"s\\ecret&=%+1";
const OTP_SECRET: &str = "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";
const TOKEN: &str = "tbat_headless_token";

#[derive(Default)]
struct FakeState {
    otp_required: bool,
    hang_idp: bool,
    callback_done: bool,
    logins: Vec<(String, String)>,
    otps: Vec<String>,
    idp_hits: usize,
    cookie_seen: bool,
}

struct Req {
    head: String,
    body: String,
}

impl Req {
    fn line(&self) -> &str {
        self.head.lines().next().unwrap_or("")
    }
    fn is(&self, method: &str, path_prefix: &str) -> bool {
        let mut p = self.line().split(' ');
        p.next() == Some(method) && p.next().is_some_and(|x| x.starts_with(path_prefix))
    }
    fn form(&self) -> Vec<(String, String)> {
        self.body
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (pct_decode(k), pct_decode(v)))
            .collect()
    }
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        match b[i] {
            b'%' => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap()
}

fn read_req(stream: &mut TcpStream) -> Option<Req> {
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
                return Some(Req { head, body });
            }
        }
        if n == 0 {
            return None;
        }
    }
}

fn resp(status: &str, headers: &[String], body: &str) -> String {
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

type Handler = Arc<dyn Fn(&Req) -> String + Send + Sync>;

fn spawn_server(listener: TcpListener, handler: Handler, stop: Arc<AtomicBool>) {
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    if let Some(req) = read_req(&mut stream) {
                        let reply = handler(&req);
                        let _ = stream.write_all(reply.as_bytes());
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    });
}

struct Fake {
    ctrl_port: u16,
    state: Arc<Mutex<FakeState>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

const LOGIN_PAGE: &str = r#"<html><head><title>Sign in</title></head><body>
<form id="kc-form-login" action="/realms/r/login-actions/authenticate?a=1&amp;b=2" method="post">
<input name="username" type="text"/><input name="password" type="password"/>
<input type="hidden" name="credentialId" value=""/>
<input name="login" type="submit" value="Sign In"/></form></body></html>"#;

const OTP_PAGE: &str = r#"<html><head><title>OTP</title></head><body>
<form action="/realms/r/login-actions/otp" method="post">
<input name="otp" type="text"/><input name="login" type="submit" value="Sign In"/>
</form></body></html>"#;

fn start_fake(otp_required: bool) -> Fake {
    let ctrl = TcpListener::bind("127.0.0.1:0").unwrap();
    let idp = TcpListener::bind("127.0.0.1:0").unwrap();
    let (ctrl_port, idp_port) = (
        ctrl.local_addr().unwrap().port(),
        idp.local_addr().unwrap().port(),
    );
    let state = Arc::new(Mutex::new(FakeState {
        otp_required,
        ..Default::default()
    }));
    let stop = Arc::new(AtomicBool::new(false));

    let st = Arc::clone(&state);
    let ctrl_handler: Handler = Arc::new(move |req| {
        let mut s = st.lock().unwrap();
        if req.is("POST", "/v1/auth-methods/amoidc_1:authenticate") {
            if req.body.contains("\"start\"") {
                let auth = format!("http://127.0.0.1:{idp_port}/realms/r/auth?state=S");
                return resp(
                    "200 OK",
                    &[],
                    &json!({"attributes": {"auth_url": auth, "token_id": "tokid"}}).to_string(),
                );
            }
            return if s.callback_done {
                resp(
                    "200 OK",
                    &[],
                    &json!({"attributes": {"token": TOKEN, "expiration_time": "2026-12-01T00:00:00Z"}})
                        .to_string(),
                )
            } else {
                resp("400 Bad Request", &[], "{}")
            };
        }
        if req.is("GET", "/v1/auth-methods/amoidc_1:authenticate:callback") {
            s.callback_done = true;
            return resp("200 OK", &[], "ok");
        }
        resp("404 Not Found", &[], "")
    });

    let st = Arc::clone(&state);
    let idp_handler: Handler = Arc::new(move |req| {
        let hang = {
            let mut s = st.lock().unwrap();
            s.idp_hits += 1;
            s.hang_idp
        };
        if hang {
            std::thread::sleep(Duration::from_millis(1500));
            return resp("200 OK", &[], "late");
        }
        let mut s = st.lock().unwrap();
        let callback = format!(
            "Location: http://127.0.0.1:{ctrl_port}/v1/auth-methods/amoidc_1:authenticate:callback?code=c&state=S"
        );
        if req.is("GET", "/realms/r/auth") {
            return resp(
                "200 OK",
                &["Set-Cookie: KC_RESTART=1; Path=/".to_string()],
                LOGIN_PAGE,
            );
        }
        if req.is("POST", "/realms/r/login-actions/authenticate") {
            s.cookie_seen = req.head.contains("KC_RESTART=1");
            let f = req.form();
            let get = |k: &str| f.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
            let (u, p) = (
                get("username").unwrap_or_default(),
                get("password").unwrap_or_default(),
            );
            s.logins.push((u.clone(), p.clone()));
            if u == USER && p == PASSWORD {
                return if s.otp_required {
                    resp("200 OK", &[], OTP_PAGE)
                } else {
                    resp("302 Found", &[callback], "")
                };
            }
            return resp("200 OK", &[], LOGIN_PAGE);
        }
        if req.is("POST", "/realms/r/login-actions/otp") {
            let code = req
                .form()
                .into_iter()
                .find(|(k, _)| k == "otp")
                .map(|(_, v)| v)
                .unwrap_or_default();
            s.otps.push(code.clone());
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let valid = [now - 30, now, now + 30]
                .iter()
                .any(|t| generate_totp_code_at_epoch(OTP_SECRET, *t).unwrap() == code);
            return if valid {
                resp("302 Found", &[callback], "")
            } else {
                resp("200 OK", &[], OTP_PAGE)
            };
        }
        resp("404 Not Found", &[], "")
    });

    spawn_server(ctrl, ctrl_handler, Arc::clone(&stop));
    spawn_server(idp, idp_handler, Arc::clone(&stop));
    Fake {
        ctrl_port,
        state,
        stop,
    }
}

fn open_runtime(
    tmp: &TempDir,
    fake: &Fake,
    mode: BoundaryOidcLoginMode,
    boundary_script: Option<&str>,
) -> ProfileRuntime<MemoryKeyringBackend> {
    let cfg_dir = tmp.path().join("cfg");
    let pdir = cfg_dir.join("profiles").join("default");
    fs::create_dir_all(&pdir).unwrap();
    fs::write(
        pdir.join("config.json"),
        json!({
            "app": {"defaultTerminalCols": 120, "defaultTerminalRows": 36},
            "boundary": {
                "addr": format!("http://127.0.0.1:{}", fake.ctrl_port),
                "authMethodId": "amoidc_1",
                "authKind": BoundaryAuthKind::Oidc,
                "oidcLoginMode": mode,
                "oidcUsername": USER
            }
        })
        .to_string(),
    )
    .unwrap();
    let script = tmp.path().join("fake-boundary");
    fs::write(
        &script,
        boundary_script.unwrap_or("#!/bin/sh\necho unexpected boundary call >&2\nexit 9\n"),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        pdir.join("sync-local.json"),
        json!({ "boundaryCliPath": script.to_string_lossy() }).to_string(),
    )
    .unwrap();
    ProfileRuntime::open_with_backend(
        &ProfileLocationInput {
            config_dir: Some(&cfg_dir),
            desktop_config_dir: Some(&tmp.path().join("desktop")),
            profile: Some("default"),
            ..Default::default()
        },
        "test",
        MemoryKeyringBackend::default(),
    )
    .unwrap()
}

fn set_creds(rt: &ProfileRuntime<MemoryKeyringBackend>, otp: bool) {
    rt.write_secret(KEYRING_BOUNDARY_OIDC_PASSWORD, Some(PASSWORD))
        .unwrap();
    if otp {
        rt.write_secret(KEYRING_BOUNDARY_OIDC_OTP_SECRET, Some(OTP_SECRET))
            .unwrap();
    }
}

fn wait_done(
    rt: &ProfileRuntime<MemoryKeyringBackend>,
    session_id: &str,
) -> scp_core::models::FunctionStatus {
    assert!(rt
        .supervisor()
        .wait_for_session_exit(session_id, Duration::from_secs(20)));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let st = rt.get_function_status("boundary-auth").unwrap();
        if st.session_id.is_none() {
            return st;
        }
        assert!(Instant::now() < deadline, "status never settled");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assert_no_secret_in_logs(rt: &ProfileRuntime<MemoryKeyringBackend>) {
    let logs = rt.get_function_logs("boundary-auth").unwrap().join("\n");
    for secret in [
        PASSWORD,
        OTP_SECRET,
        TOKEN,
        "state=S",
        "code=c",
        "KC_RESTART",
        USER,
    ] {
        assert!(!logs.contains(secret), "logs leaked {secret}: {logs}");
    }
}

#[test]
fn automated_oidc_logs_in_headlessly_with_otp_and_stores_the_app_token() {
    let tmp = TempDir::new().unwrap();
    let fake = start_fake(true);
    let rt = open_runtime(&tmp, &fake, BoundaryOidcLoginMode::Automated, None);
    set_creds(&rt, true);
    assert!(rt
        .read_optional_secret(KEYRING_BOUNDARY_APP_TOKEN)
        .is_none());

    // Default entry point (what MCP boundary_auth_start uses): no interactive opt-in needed.
    let started = rt.start_boundary_auth(&CancelToken::new()).unwrap();
    let st = wait_done(&rt, &started.session_id);

    assert_eq!(st.state, "authenticated", "status: {st:?}");
    assert_eq!(st.boundary_token_source.as_deref(), Some("app"));
    assert_eq!(
        rt.read_optional_secret(KEYRING_BOUNDARY_APP_TOKEN)
            .as_deref(),
        Some(TOKEN)
    );
    let s = fake.state.lock().unwrap();
    assert_eq!(s.logins, [(USER.to_string(), PASSWORD.to_string())]);
    assert_eq!(s.otps.len(), 1);
    assert!(s.cookie_seen, "IdP session cookie must be replayed");
    drop(s);
    assert_no_secret_in_logs(&rt);
}

#[test]
fn password_only_idp_works_without_an_otp_secret() {
    let tmp = TempDir::new().unwrap();
    let fake = start_fake(false);
    let rt = open_runtime(&tmp, &fake, BoundaryOidcLoginMode::Automated, None);
    set_creds(&rt, false);
    let started = rt.start_boundary_auth(&CancelToken::new()).unwrap();
    assert_eq!(wait_done(&rt, &started.session_id).state, "authenticated");
}

#[test]
fn wrong_password_ends_in_error_state_without_storing_a_token() {
    let tmp = TempDir::new().unwrap();
    let fake = start_fake(false);
    let rt = open_runtime(&tmp, &fake, BoundaryOidcLoginMode::Automated, None);
    rt.write_secret(KEYRING_BOUNDARY_OIDC_PASSWORD, Some("wrong-password"))
        .unwrap();
    let started = rt.start_boundary_auth(&CancelToken::new()).unwrap();
    let st = wait_done(&rt, &started.session_id);
    assert_eq!(st.state, "error");
    assert!(
        st.message.starts_with("oidc_credentials_rejected"),
        "message: {}",
        st.message
    );
    assert!(!st.message.contains("wrong-password"));
    assert!(rt
        .read_optional_secret(KEYRING_BOUNDARY_APP_TOKEN)
        .is_none());
}

#[test]
fn otp_page_without_a_secret_fails_closed_with_a_machine_readable_reason() {
    let tmp = TempDir::new().unwrap();
    let fake = start_fake(true);
    let rt = open_runtime(&tmp, &fake, BoundaryOidcLoginMode::Automated, None);
    set_creds(&rt, false);
    let started = rt.start_boundary_auth(&CancelToken::new()).unwrap();
    let st = wait_done(&rt, &started.session_id);
    assert_eq!(st.state, "error");
    assert!(
        st.message.starts_with("oidc_otp_required"),
        "{}",
        st.message
    );
    assert!(fake.state.lock().unwrap().otps.is_empty());
}

#[test]
fn missing_credentials_are_named_and_nothing_is_contacted() {
    let tmp = TempDir::new().unwrap();
    let fake = start_fake(false);
    let rt = open_runtime(&tmp, &fake, BoundaryOidcLoginMode::Automated, None);
    let err = rt.start_boundary_auth(&CancelToken::new()).unwrap_err();
    assert_eq!(err.code, "CAPABILITY_UNAVAILABLE");
    assert_eq!(err.reason, "credentials_missing");
    assert!(
        err.message.contains("boundary_oidc_password"),
        "{}",
        err.message
    );
    assert!(
        err.message.contains("scpctl boundary auth --interactive"),
        "{}",
        err.message
    );
    assert_eq!(fake.state.lock().unwrap().idp_hits, 0);
}

#[test]
fn browser_mode_stays_rejected_without_the_interactive_opt_in() {
    let tmp = TempDir::new().unwrap();
    let fake = start_fake(false);
    let rt = open_runtime(&tmp, &fake, BoundaryOidcLoginMode::Browser, None);
    set_creds(&rt, true);
    let err = rt.start_boundary_auth(&CancelToken::new()).unwrap_err();
    assert_eq!(err.reason, "browser_oidc_unverified");
    assert_eq!(fake.state.lock().unwrap().idp_hits, 0);
}

#[test]
fn stop_cancels_a_stuck_login_quickly_and_leaves_no_token() {
    let tmp = TempDir::new().unwrap();
    let fake = start_fake(false);
    fake.state.lock().unwrap().hang_idp = true;
    let rt = open_runtime(&tmp, &fake, BoundaryOidcLoginMode::Automated, None);
    set_creds(&rt, true);
    let started = rt.start_boundary_auth(&CancelToken::new()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.state.lock().unwrap().idp_hits == 0 {
        assert!(Instant::now() < deadline, "login never reached the IdP");
        std::thread::sleep(Duration::from_millis(10));
    }
    let begin = Instant::now();
    assert!(rt.stop_boundary_auth(Duration::from_secs(2)).unwrap());
    assert!(begin.elapsed() < Duration::from_millis(1500));
    let st = wait_done(&rt, &started.session_id);
    assert_eq!(st.state, "idle", "{st:?}");
    assert!(rt
        .read_optional_secret(KEYRING_BOUNDARY_APP_TOKEN)
        .is_none());
    // A second login can start afterwards (function slot released).
    fake.state.lock().unwrap().hang_idp = false;
    let again = rt.start_boundary_auth(&CancelToken::new()).unwrap();
    assert_eq!(wait_done(&rt, &again.session_id).state, "authenticated");
}

#[test]
fn target_search_errors_redact_the_oidc_password_and_otp_secret() {
    let tmp = TempDir::new().unwrap();
    let fake = start_fake(false);
    // Rust's `{:?}` yields a valid Python string literal for these ASCII secrets.
    let script = format!(
        "#!/usr/bin/env python3\nimport sys\nsys.stderr.write(\"auth failed for \" + {PASSWORD:?} + \" and \" + {OTP_SECRET:?})\nsys.exit(1)\n"
    );
    let rt = open_runtime(&tmp, &fake, BoundaryOidcLoginMode::Automated, Some(&script));
    set_creds(&rt, true);
    let err = rt.search_boundary_targets(None, None).unwrap_err();
    assert_eq!(err.reason, "boundary_search_failed");
    assert!(!err.message.contains(PASSWORD), "{}", err.message);
    assert!(!err.message.contains(OTP_SECRET), "{}", err.message);
    assert!(err.message.contains("[REDACTED]"), "{}", err.message);
}

fn recording_boundary_script(record: &std::path::Path) -> String {
    format!(
        "#!/bin/sh\necho called >> {}\nprintf '{{\"item\":{{\"attributes\":{{\"token\":\"tbat_from_stock\"}}}}}}'\n",
        record.display()
    )
}

#[test]
fn interactive_runs_prefer_headless_and_skip_flag_forces_the_stock_browser_login() {
    let tmp = TempDir::new().unwrap();
    let record = tmp.path().join("calls.txt");
    let fake = start_fake(false);
    let rt = open_runtime(
        &tmp,
        &fake,
        BoundaryOidcLoginMode::Automated,
        Some(&recording_boundary_script(&record)),
    );
    set_creds(&rt, false);

    let interactive = BoundaryAuthOptions {
        allow_browser_oidc: true,
        ..Default::default()
    };
    let started = rt
        .start_boundary_auth_with_options(&CancelToken::new(), interactive)
        .unwrap();
    assert_eq!(wait_done(&rt, &started.session_id).state, "authenticated");
    assert_eq!(
        rt.read_optional_secret(KEYRING_BOUNDARY_APP_TOKEN)
            .as_deref(),
        Some(TOKEN),
        "credentials are configured, so the automated route must be used"
    );
    assert!(!record.exists(), "stock boundary must not run");

    let forced = BoundaryAuthOptions {
        allow_browser_oidc: true,
        skip_headless_oidc: true,
    };
    let started = rt
        .start_boundary_auth_with_options(&CancelToken::new(), forced)
        .unwrap();
    assert_eq!(wait_done(&rt, &started.session_id).state, "authenticated");
    assert_eq!(
        rt.read_optional_secret(KEYRING_BOUNDARY_APP_TOKEN)
            .as_deref(),
        Some("tbat_from_stock")
    );
    assert!(record.exists());
}

#[test]
fn route_reports_which_login_will_run() {
    use scp_core::boundary_ops::BoundaryAuthRoute;
    let tmp = TempDir::new().unwrap();
    let fake = start_fake(false);
    let rt = open_runtime(&tmp, &fake, BoundaryOidcLoginMode::Automated, None);
    let interactive = BoundaryAuthOptions {
        allow_browser_oidc: true,
        ..Default::default()
    };
    assert_eq!(
        rt.boundary_auth_route(interactive).unwrap(),
        BoundaryAuthRoute::BrowserOidc
    );
    set_creds(&rt, false);
    assert_eq!(
        rt.boundary_auth_route(interactive).unwrap(),
        BoundaryAuthRoute::HeadlessOidc
    );
    assert_eq!(
        rt.boundary_auth_route(BoundaryAuthOptions {
            skip_headless_oidc: true,
            ..interactive
        })
        .unwrap(),
        BoundaryAuthRoute::BrowserOidc
    );
}
