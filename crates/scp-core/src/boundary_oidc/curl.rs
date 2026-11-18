//! `curl`-backed [`HttpTransport`]: credentials and bodies go to curl on stdin (`-K -`), never
//! argv; redirects are never followed (the flow follows them itself); cookies live in a private
//! jar inside a `0700` temp directory that is deleted on drop.
use super::{HttpBody, HttpRequest, HttpResponse, HttpTransport, OidcError};
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

pub struct CurlTransport {
    program: String,
    dir: TempDir,
    jar: PathBuf,
    body_file: PathBuf,
}

impl CurlTransport {
    /// Uses `curl` from `PATH`.
    pub fn new() -> Result<Self, OidcError> {
        Self::with_program("curl")
    }

    pub fn with_program(program: &str) -> Result<Self, OidcError> {
        let dir = TempDir::new().map_err(|e| {
            OidcError::new(
                "oidc_tempdir_failed",
                format!("Cannot create private temp directory: {e}"),
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).map_err(
                |e| {
                    OidcError::new(
                        "oidc_tempdir_failed",
                        format!("Cannot restrict temp directory permissions: {e}"),
                    )
                },
            )?;
        }
        Ok(Self {
            program: program.to_string(),
            jar: dir.path().join("cookies.txt"),
            body_file: dir.path().join("body.out"),
            dir,
        })
    }

    /// Directory holding the cookie jar and response body (exposed for isolation tests).
    pub fn private_dir(&self) -> &std::path::Path {
        self.dir.path()
    }
}

/// Checks that `program` can be started (`--version`), so a missing curl fails at start time.
pub fn ensure_available(program: &str) -> Result<(), OidcError> {
    match Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Err(OidcError::new(
            "oidc_curl_not_found",
            "curl is required for automated Boundary OIDC login but was not found on PATH",
        )),
        Err(e) => Err(OidcError::new(
            "oidc_curl_failed",
            format!("Cannot start curl: {e}"),
        )),
    }
}

const CONNECT_TIMEOUT_SECS: u32 = 10;
const MAX_TIME_SECS: u32 = 30;
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(20);

impl HttpTransport for CurlTransport {
    fn send(
        &mut self,
        req: &HttpRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<HttpResponse, OidcError> {
        let config = self.build_config(req)?;
        let _ = std::fs::remove_file(&self.body_file);
        let mut child = Command::new(&self.program)
            .args(["-q", "-K", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == ErrorKind::NotFound {
                    OidcError::new(
                        "oidc_curl_not_found",
                        "curl is required for automated Boundary OIDC login but was not found on PATH",
                    )
                } else {
                    OidcError::new("oidc_curl_failed", format!("Cannot start curl: {e}"))
                }
            })?;
        if let Some(mut stdin) = child.stdin.take() {
            // A write error means curl already exited; its exit status is reported below.
            let _ = stdin.write_all(config.as_bytes());
        }
        let status = loop {
            if cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(OidcError::new("oidc_cancelled", "OIDC login cancelled"));
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(POLL_INTERVAL),
                Err(e) => {
                    return Err(OidcError::new(
                        "oidc_curl_failed",
                        format!("Cannot wait for curl: {e}"),
                    ))
                }
            }
        };
        let mut stdout = String::new();
        if let Some(mut out) = child.stdout.take() {
            let _ = out.read_to_string(&mut stdout);
        }
        if !status.success() {
            return Err(map_exit_code(status.code()));
        }
        self.parse_response(&stdout)
    }
}

impl CurlTransport {
    fn build_config(&self, req: &HttpRequest) -> Result<String, OidcError> {
        let url = super::url::parse(&req.url)
            .filter(super::url::Url::scheme_allowed)
            .ok_or_else(|| {
                OidcError::new(
                    "oidc_insecure_url",
                    "Refusing a request that is not https (plain http is only allowed for loopback)",
                )
            })?;
        let invalid = |what: &str| OidcError::new("oidc_request_invalid", what.to_string());
        let mut cfg = String::from("silent\nshow-error\n");
        let mut kv = |key: &str, value: &str| {
            cfg.push_str(key);
            cfg.push_str(" = ");
            cfg.push_str(&quote(value));
            cfg.push('\n');
        };
        kv("url", &req.url);
        kv(
            "proto",
            if url.scheme == "https" {
                "=https"
            } else {
                "=http"
            },
        );
        kv("output", &path_str(&self.body_file));
        kv("write-out", "%{http_code} %{redirect_url}");
        kv("cookie", &path_str(&self.jar));
        kv("cookie-jar", &path_str(&self.jar));
        kv("connect-timeout", &CONNECT_TIMEOUT_SECS.to_string());
        kv("max-time", &MAX_TIME_SECS.to_string());
        kv("max-filesize", &MAX_BODY_BYTES.to_string());
        kv("user-agent", "scpctl");
        match &req.body {
            HttpBody::None => {}
            HttpBody::Form(fields) => {
                for (name, value) in fields {
                    if name.contains('=') || name.chars().any(char::is_control) {
                        return Err(invalid("Form field name is not safe to send"));
                    }
                    kv("data-urlencode", &format!("{name}={value}"));
                }
            }
            HttpBody::Json(body) => {
                if body.starts_with('@') {
                    return Err(invalid("JSON body must not start with '@'"));
                }
                kv("header", "Content-Type: application/json");
                kv("header", "Accept: application/json");
                kv("data-binary", body);
            }
        }
        Ok(cfg)
    }

    fn parse_response(&self, stdout: &str) -> Result<HttpResponse, OidcError> {
        let (code, redirect) = stdout.trim().split_once(' ').unwrap_or((stdout.trim(), ""));
        let status: u16 = code.parse().map_err(|_| {
            OidcError::new(
                "oidc_curl_failed",
                "curl returned an unreadable status line",
            )
        })?;
        let bytes = std::fs::read(&self.body_file).unwrap_or_default();
        Ok(HttpResponse {
            status,
            location: (!redirect.is_empty()).then(|| redirect.to_string()),
            body: String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_BODY_BYTES)]).into_owned(),
        })
    }
}

fn path_str(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Quotes a value for a curl config file (`\\`, `"` and control escapes).
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{0b}' => out.push_str("\\v"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Maps curl's exit status to a stable reason without echoing curl's stderr (it can contain URLs).
fn map_exit_code(code: Option<i32>) -> OidcError {
    match code {
        Some(6 | 7) => OidcError::new("oidc_network_error", "Cannot connect to the host"),
        Some(28) => OidcError::new("oidc_network_timeout", "The request timed out"),
        Some(35 | 51 | 58 | 59 | 60 | 77 | 83 | 90 | 91) => OidcError::new(
            "oidc_tls_error",
            "TLS verification or handshake failed (check the server certificate)",
        ),
        Some(n) => OidcError::new(
            "oidc_curl_failed",
            format!("curl failed with exit code {n}"),
        ),
        None => OidcError::new("oidc_curl_failed", "curl was terminated by a signal"),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[derive(Debug, Clone)]
    struct Raw {
        head: String,
        body: String,
    }

    /// Serves `replies.len()` connections sequentially on loopback; records each request.
    fn serve(replies: Vec<String>) -> (u16, Arc<Mutex<Vec<Raw>>>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = Arc::clone(&seen);
        let h = std::thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let raw = read_request(&mut stream);
                seen2.lock().unwrap().push(raw);
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });
        (port, seen, h)
    }

    fn read_request(stream: &mut TcpStream) -> Raw {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
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
                if buf.len() >= idx + 4 + len || n == 0 {
                    let body = String::from_utf8_lossy(&buf[idx + 4..]).to_string();
                    return Raw { head, body };
                }
            } else if n == 0 {
                return Raw {
                    head: text,
                    body: String::new(),
                };
            }
        }
    }

    fn http(status: &str, headers: &[&str], body: &str) -> String {
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

    fn get(url: &str) -> HttpRequest {
        HttpRequest {
            url: url.to_string(),
            body: HttpBody::None,
        }
    }

    fn no_cancel() -> bool {
        false
    }

    #[test]
    fn get_returns_status_and_body_and_does_not_follow_redirects() {
        let (port, seen, h) = serve(vec![
            http("200 OK", &[], "<html>hello</html>"),
            http("302 Found", &["Location: /next?a=1"], ""),
        ]);
        let mut t = CurlTransport::new().unwrap();
        let r = t
            .send(&get(&format!("http://127.0.0.1:{port}/p")), &no_cancel)
            .unwrap();
        assert_eq!((r.status, r.body.as_str()), (200, "<html>hello</html>"));
        assert_eq!(r.location, None);
        let r = t
            .send(&get(&format!("http://127.0.0.1:{port}/p")), &no_cancel)
            .unwrap();
        assert_eq!(r.status, 302);
        assert_eq!(
            r.location.as_deref(),
            Some(format!("http://127.0.0.1:{port}/next?a=1").as_str())
        );
        h.join().unwrap();
        assert_eq!(
            seen.lock().unwrap().len(),
            2,
            "redirect must not be followed"
        );
        assert!(seen.lock().unwrap()[0].head.starts_with("GET /p HTTP/1.1"));
    }

    #[test]
    fn form_post_urlencodes_hostile_values() {
        let (port, seen, h) = serve(vec![http("200 OK", &[], "ok")]);
        let mut t = CurlTransport::new().unwrap();
        let hostile = "p@ss\"w\\ord&=%+ \u{e9}\nline2";
        let req = HttpRequest {
            url: format!("http://127.0.0.1:{port}/login"),
            body: HttpBody::Form(vec![
                ("username".into(), "alice".into()),
                ("password".into(), hostile.into()),
                ("empty".into(), String::new()),
            ]),
        };
        assert_eq!(t.send(&req, &no_cancel).unwrap().status, 200);
        h.join().unwrap();
        let raw = seen.lock().unwrap()[0].clone();
        assert!(raw.head.starts_with("POST /login HTTP/1.1"), "{}", raw.head);
        assert!(raw
            .head
            .to_ascii_lowercase()
            .contains("content-type: application/x-www-form-urlencoded"));
        let pairs: Vec<(String, String)> = raw
            .body
            .split('&')
            .map(|kv| {
                let (k, v) = kv.split_once('=').unwrap();
                (k.to_string(), pct_decode(v))
            })
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("username".to_string(), "alice".to_string()),
                ("password".to_string(), hostile.to_string()),
                ("empty".to_string(), String::new()),
            ]
        );
    }

    fn pct_decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'%' => {
                    out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                    i += 3;
                }
                b'+' => {
                    out.push(b' ');
                    i += 1;
                }
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn json_post_sends_exact_body_and_content_type() {
        let (port, seen, h) = serve(vec![http("200 OK", &[], "{}")]);
        let mut t = CurlTransport::new().unwrap();
        let body = r#"{"command":"start","note":"a\"b @file"}"#;
        let req = HttpRequest {
            url: format!("http://127.0.0.1:{port}/v1/x:authenticate"),
            body: HttpBody::Json(body.to_string()),
        };
        t.send(&req, &no_cancel).unwrap();
        h.join().unwrap();
        let raw = seen.lock().unwrap()[0].clone();
        assert!(raw
            .head
            .to_ascii_lowercase()
            .contains("content-type: application/json"));
        assert_eq!(raw.body, body);
    }

    #[test]
    fn cookies_persist_between_requests_in_the_private_jar() {
        let (port, seen, h) = serve(vec![
            http("200 OK", &["Set-Cookie: KC_SESSION=abc123; Path=/"], "a"),
            http("200 OK", &[], "b"),
        ]);
        let mut t = CurlTransport::new().unwrap();
        let url = format!("http://127.0.0.1:{port}/");
        t.send(&get(&url), &no_cancel).unwrap();
        t.send(&get(&url), &no_cancel).unwrap();
        h.join().unwrap();
        let second = seen.lock().unwrap()[1].head.clone();
        assert!(second.contains("KC_SESSION=abc123"), "{second}");
        let mode = std::fs::metadata(t.private_dir())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "temp dir must be private, mode {mode:o}");
        let dir = t.private_dir().to_path_buf();
        drop(t);
        assert!(!dir.exists(), "temp dir must be deleted on drop");
    }

    #[test]
    fn secrets_go_through_stdin_and_never_argv() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("argv.txt");
        let stdin_log = tmp.path().join("stdin.txt");
        let script = tmp.path().join("fake-curl");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {argv}\ncat > {stdin}\nprintf '200 '\n",
                argv = log.display(),
                stdin = stdin_log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut t = CurlTransport::with_program(script.to_str().unwrap()).unwrap();
        let req = HttpRequest {
            url: "https://idp.example.test/login?session_code=SECRETQ".into(),
            body: HttpBody::Form(vec![("password".into(), "hunter2-secret".into())]),
        };
        let r = t.send(&req, &no_cancel).unwrap();
        assert_eq!(r.status, 200);
        let argv = std::fs::read_to_string(&log).unwrap();
        let stdin = std::fs::read_to_string(&stdin_log).unwrap();
        for secret in ["hunter2-secret", "SECRETQ", "idp.example.test"] {
            assert!(!argv.contains(secret), "argv leaked {secret}: {argv}");
            assert!(stdin.contains(secret), "stdin must carry {secret}: {stdin}");
        }
        assert!(
            argv.lines().any(|l| l == "-q"),
            "must ignore ~/.curlrc: {argv}"
        );
        assert!(stdin.contains("proto = \"=https\""), "{stdin}");
        assert!(
            !stdin.contains("location"),
            "must never follow redirects: {stdin}"
        );
    }

    #[test]
    fn ensure_available_detects_missing_and_present_curl() {
        assert_eq!(
            ensure_available("definitely-not-a-real-curl-binary")
                .err()
                .unwrap()
                .reason,
            "oidc_curl_not_found"
        );
        ensure_available("curl").unwrap();
    }

    #[test]
    fn missing_curl_is_a_clear_error() {
        let mut t = CurlTransport::with_program("definitely-not-a-real-curl-binary").unwrap();
        let e = t
            .send(&get("https://idp.example.test/"), &no_cancel)
            .err()
            .unwrap();
        assert_eq!(e.reason, "oidc_curl_not_found");
    }

    #[test]
    fn plain_http_to_a_non_loopback_host_is_refused_without_running_curl() {
        let mut t = CurlTransport::with_program("definitely-not-a-real-curl-binary").unwrap();
        let e = t
            .send(&get("http://idp.example.test/"), &no_cancel)
            .err()
            .unwrap();
        assert_eq!(e.reason, "oidc_insecure_url");
    }

    #[test]
    fn rejects_form_names_that_curl_would_misparse() {
        let mut t = CurlTransport::with_program("definitely-not-a-real-curl-binary").unwrap();
        let req = HttpRequest {
            url: "https://idp.example.test/".into(),
            body: HttpBody::Form(vec![("a=b".into(), "v".into())]),
        };
        assert_eq!(
            t.send(&req, &no_cancel).err().unwrap().reason,
            "oidc_request_invalid"
        );
    }

    #[test]
    fn connection_refused_is_classified() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let mut t = CurlTransport::new().unwrap();
        let e = t
            .send(&get(&format!("http://127.0.0.1:{port}/")), &no_cancel)
            .err()
            .unwrap();
        assert_eq!(e.reason, "oidc_network_error");
    }

    #[test]
    fn cancellation_kills_a_hanging_request_quickly() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(1500));
            drop(stream);
        });
        let start = Instant::now();
        let cancelled = move || start.elapsed() > Duration::from_millis(300);
        let mut t = CurlTransport::new().unwrap();
        let e = t
            .send(&get(&format!("http://127.0.0.1:{port}/")), &cancelled)
            .err()
            .unwrap();
        assert_eq!(e.reason, "oidc_cancelled");
        assert!(start.elapsed() < Duration::from_millis(1200));
        h.join().unwrap();
    }
}
