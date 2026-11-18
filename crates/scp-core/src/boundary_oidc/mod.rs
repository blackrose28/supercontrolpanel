//! Headless Boundary OIDC login (Phase 2): Boundary HTTP auth API + IdP form driver over `curl`.
//!
//! The flow is transport-agnostic ([`HttpTransport`]) so the IdP/Boundary logic is unit-tested
//! against fakes; [`curl::CurlTransport`] is the production transport.
//!
//! Security rules enforced here (see `docs/mcp/STANDALONE_CLI_BOUNDARY_OIDC_AUTOMATION.md`):
//! credentials and one-time codes are only ever POSTed to the authority of the `auth_url` the
//! controller returned; redirects are followed manually; the single allowed hop to the
//! controller is the OIDC callback GET (no body, never followed further); `http` is allowed only
//! for loopback hosts; nothing but fixed stage messages is logged.
pub mod curl;
pub mod html;
pub mod url;

use std::time::Duration;

/// Maximum number of IdP pages/redirects before giving up.
pub const MAX_IDP_STEPS: u32 = 20;

/// Failure with a stable machine-readable `reason` (also the prefix of the exit reason string).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcError {
    pub reason: &'static str,
    pub message: String,
}

impl OidcError {
    pub fn new(reason: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }

    /// `"reason: message"`, the format stored as the function status message on failure.
    pub fn exit_reason(&self) -> String {
        format!("{}: {}", self.reason, self.message)
    }
}

pub enum HttpBody {
    /// Plain `GET`.
    None,
    /// `POST application/x-www-form-urlencoded`.
    Form(Vec<(String, String)>),
    /// `POST application/json`.
    Json(String),
}

pub struct HttpRequest {
    pub url: String,
    pub body: HttpBody,
}

pub struct HttpResponse {
    pub status: u16,
    /// Absolute redirect target for 3xx responses.
    pub location: Option<String>,
    pub body: String,
}

/// Sends one request without following redirects. Cookie handling is the transport's job.
pub trait HttpTransport {
    fn send(
        &mut self,
        req: &HttpRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<HttpResponse, OidcError>;
}

pub struct OidcCredentials {
    pub username: String,
    pub password: String,
    pub otp_secret: Option<String>,
}

pub struct OidcLogin<'a> {
    /// Boundary controller address, e.g. `https://boundary.example.com`.
    pub addr: &'a str,
    pub auth_method_id: &'a str,
    pub credentials: &'a OidcCredentials,
}

pub struct OidcEnv<'a> {
    pub cancelled: &'a dyn Fn() -> bool,
    pub log: &'a dyn Fn(&str),
    /// Produces the current 6-digit code for a secret; called only when the OTP page is reached.
    pub totp: &'a dyn Fn(&str) -> Result<String, String>,
    pub sleep: &'a dyn Fn(Duration),
    pub poll_attempts: u32,
    pub poll_interval: Duration,
}

pub struct OidcToken {
    pub token: String,
    pub expiration_time: Option<String>,
}

/// Runs the whole login: `start`, IdP form steps, callback, token poll.
pub fn run_oidc_login(
    transport: &mut dyn HttpTransport,
    login: &OidcLogin<'_>,
    env: &OidcEnv<'_>,
) -> Result<OidcToken, OidcError> {
    let addr = login.addr.trim().trim_end_matches('/');
    let controller = url::parse(addr)
        .filter(url::Url::scheme_allowed)
        .ok_or_else(|| {
            OidcError::new(
                "oidc_insecure_url",
                "Boundary address must be an https URL (plain http is only allowed for loopback)",
            )
        })?;
    let method_id = login.auth_method_id.trim();
    if method_id.is_empty()
        || !method_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(OidcError::new(
            "oidc_start_invalid",
            "Boundary authMethodId is missing or contains unexpected characters",
        ));
    }
    let endpoint = format!("{addr}/v1/auth-methods/{method_id}:authenticate");
    check_cancel(env)?;
    (env.log)("Requesting OIDC login from Boundary");
    let (auth_url, token_id) = start_login(transport, env, &endpoint)?;
    let auth = url::parse(&auth_url)
        .filter(url::Url::scheme_allowed)
        .ok_or_else(|| {
            OidcError::new(
                "oidc_insecure_url",
                "Boundary returned an IdP login URL that is not https",
            )
        })?;
    let mut driver = IdpDriver {
        idp_authority: auth.authority.clone(),
        controller_authority: controller.authority,
        creds: login.credentials,
        sent_password: false,
        sent_username_only: false,
        sent_otp: false,
    };
    driver.walk(transport, env, auth)?;
    poll_token(transport, env, &endpoint, &token_id)
}

fn check_cancel(env: &OidcEnv<'_>) -> Result<(), OidcError> {
    if (env.cancelled)() {
        Err(OidcError::new("oidc_cancelled", "OIDC login cancelled"))
    } else {
        Ok(())
    }
}

fn start_login(
    transport: &mut dyn HttpTransport,
    env: &OidcEnv<'_>,
    endpoint: &str,
) -> Result<(String, String), OidcError> {
    let resp = transport.send(
        &HttpRequest {
            url: endpoint.to_string(),
            body: HttpBody::Json(serde_json::json!({ "command": "start" }).to_string()),
        },
        env.cancelled,
    )?;
    if !(200..300).contains(&resp.status) {
        return Err(OidcError::new(
            "oidc_start_failed",
            format!("Boundary start request failed: HTTP {}", resp.status),
        ));
    }
    let invalid = || {
        OidcError::new(
            "oidc_start_invalid",
            "Boundary start response did not include auth_url and token_id",
        )
    };
    let doc: serde_json::Value = serde_json::from_str(&resp.body).map_err(|_| invalid())?;
    let field = |name: &str| {
        doc.pointer(&format!("/attributes/{name}"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
    };
    Ok((
        field("auth_url").ok_or_else(invalid)?,
        field("token_id").ok_or_else(invalid)?,
    ))
}

fn poll_token(
    transport: &mut dyn HttpTransport,
    env: &OidcEnv<'_>,
    endpoint: &str,
    token_id: &str,
) -> Result<OidcToken, OidcError> {
    (env.log)("Waiting for Boundary to issue the token");
    let body = serde_json::json!({
        "command": "token",
        "token_id": token_id,
        "attributes": { "token_id": token_id },
    })
    .to_string();
    for attempt in 0..env.poll_attempts {
        if attempt > 0 {
            (env.sleep)(env.poll_interval);
        }
        check_cancel(env)?;
        let resp = transport.send(
            &HttpRequest {
                url: endpoint.to_string(),
                body: HttpBody::Json(body.clone()),
            },
            env.cancelled,
        )?;
        match resp.status {
            200..=299 => {
                if let Some(token) = parse_token(&resp.body) {
                    return Ok(token);
                }
            }
            400 | 404 => {}
            other => {
                return Err(OidcError::new(
                    "oidc_token_poll_failed",
                    format!("Boundary token request failed: HTTP {other}"),
                ))
            }
        }
    }
    Err(OidcError::new(
        "oidc_token_timeout",
        "Boundary did not issue a token after the IdP login completed",
    ))
}

fn parse_token(body: &str) -> Option<OidcToken> {
    let doc: serde_json::Value = serde_json::from_str(body).ok()?;
    let text = |ptr: &str| {
        doc.pointer(ptr)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
    };
    Some(OidcToken {
        token: text("/attributes/token")?,
        expiration_time: text("/attributes/expiration_time"),
    })
}

/// Walks the IdP pages, filling the login/OTP forms it recognises and failing closed otherwise.
struct IdpDriver<'a> {
    idp_authority: String,
    controller_authority: String,
    creds: &'a OidcCredentials,
    sent_password: bool,
    sent_username_only: bool,
    sent_otp: bool,
}

enum PageKind {
    Login { has_password: bool },
    Otp,
}

fn blocked(authority: &str) -> OidcError {
    OidcError::new(
        "oidc_redirect_blocked",
        format!("Refusing to contact {authority}: not the IdP host (or not https)"),
    )
}

impl IdpDriver<'_> {
    fn walk(
        &mut self,
        transport: &mut dyn HttpTransport,
        env: &OidcEnv<'_>,
        first: url::Url,
    ) -> Result<(), OidcError> {
        (env.log)("Following IdP login pages");
        let mut current = first;
        let mut body = HttpBody::None;
        for _ in 0..MAX_IDP_STEPS {
            check_cancel(env)?;
            let resp = transport.send(
                &HttpRequest {
                    url: current.as_string(),
                    body,
                },
                env.cancelled,
            )?;
            match resp.status {
                300..=399 => {
                    let target = self.resolve_redirect(&current, resp.location.as_deref())?;
                    if self.is_callback(&target) {
                        return self.complete_callback(transport, env, &target);
                    }
                    current = target;
                    body = HttpBody::None;
                }
                200..=299 => {
                    let (target, form_body) = self.fill_page(&current, &resp.body, env)?;
                    current = target;
                    body = HttpBody::Form(form_body);
                }
                other => {
                    return Err(OidcError::new(
                        "oidc_idp_http_error",
                        format!("IdP returned HTTP {other}"),
                    ))
                }
            }
        }
        Err(OidcError::new(
            "oidc_too_many_redirects",
            "IdP login did not reach the Boundary callback within the step limit",
        ))
    }

    fn resolve_redirect(
        &self,
        from: &url::Url,
        location: Option<&str>,
    ) -> Result<url::Url, OidcError> {
        let target = location
            .and_then(|l| url::resolve(from, l))
            .ok_or_else(|| {
                OidcError::new(
                    "oidc_redirect_blocked",
                    "IdP redirected to a missing or invalid location",
                )
            })?;
        if !target.scheme_allowed() {
            return Err(blocked(&target.authority));
        }
        if self.is_callback(&target) || target.authority == self.idp_authority {
            Ok(target)
        } else {
            Err(blocked(&target.authority))
        }
    }

    fn is_callback(&self, target: &url::Url) -> bool {
        target.authority == self.controller_authority
            && target.path().starts_with("/v1/auth-methods/")
            && target.path().contains(":callback")
    }

    fn complete_callback(
        &self,
        transport: &mut dyn HttpTransport,
        env: &OidcEnv<'_>,
        callback: &url::Url,
    ) -> Result<(), OidcError> {
        check_cancel(env)?;
        (env.log)("Completing Boundary callback");
        let resp = transport.send(
            &HttpRequest {
                url: callback.as_string(),
                body: HttpBody::None,
            },
            env.cancelled,
        )?;
        if resp.status >= 400 {
            return Err(OidcError::new(
                "oidc_callback_failed",
                format!("Boundary callback failed: HTTP {}", resp.status),
            ));
        }
        Ok(())
    }

    /// Picks the form to submit on `page`, returning the POST target and its fields.
    fn fill_page(
        &mut self,
        page_url: &url::Url,
        html_body: &str,
        env: &OidcEnv<'_>,
    ) -> Result<(url::Url, Vec<(String, String)>), OidcError> {
        let forms = html::parse_forms(html_body);
        let Some((form, kind)) = forms.iter().find_map(|f| classify_form(f).map(|k| (f, k))) else {
            return Err(unsupported(html_body));
        };
        let target = self.form_target(page_url, form)?;
        let fields = match kind {
            PageKind::Login { has_password } => {
                let already = if has_password {
                    std::mem::replace(&mut self.sent_password, true)
                } else {
                    std::mem::replace(&mut self.sent_username_only, true)
                };
                if already {
                    return Err(OidcError::new(
                        "oidc_credentials_rejected",
                        "The IdP rejected the OIDC username or password",
                    ));
                }
                (env.log)("Submitting IdP credentials");
                build_fields(form, Some(self.creds), None)
            }
            PageKind::Otp => {
                let secret = self
                    .creds
                    .otp_secret
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        OidcError::new(
                            "oidc_otp_required",
                            "The IdP asks for a one-time code but no OIDC OTP secret is configured",
                        )
                    })?;
                if std::mem::replace(&mut self.sent_otp, true) {
                    return Err(OidcError::new(
                        "oidc_otp_rejected",
                        "The IdP rejected the one-time code",
                    ));
                }
                let code = (env.totp)(secret).map_err(|e| {
                    OidcError::new(
                        "oidc_otp_invalid_secret",
                        format!("Cannot generate a one-time code: {e}"),
                    )
                })?;
                (env.log)("Submitting one-time code");
                build_fields(form, None, Some(&code))
            }
        };
        Ok((target, fields))
    }

    fn form_target(&self, page_url: &url::Url, form: &html::Form) -> Result<url::Url, OidcError> {
        let target = if form.action.trim().is_empty() {
            Some(page_url.clone())
        } else {
            url::resolve(page_url, &form.action)
        }
        .ok_or_else(|| {
            OidcError::new(
                "oidc_redirect_blocked",
                "IdP form action is not a valid http(s) URL",
            )
        })?;
        if target.scheme_allowed() && target.authority == self.idp_authority {
            Ok(target)
        } else {
            Err(blocked(&target.authority))
        }
    }
}

fn unsupported(html_body: &str) -> OidcError {
    let title = html::page_title(html_body)
        .map(|t| {
            let t: String = t.chars().take(80).collect();
            format!(" (page title: {t})")
        })
        .unwrap_or_default();
    OidcError::new(
        "oidc_idp_unsupported",
        format!("Unrecognised IdP login page{title}; automated login cannot continue"),
    )
}

fn is_text_like(input: &html::FormInput) -> bool {
    matches!(input.kind.as_str(), "text" | "email" | "tel" | "number")
}

fn is_otp_input(input: &html::FormInput) -> bool {
    let name = input.name.to_ascii_lowercase();
    is_text_like(input)
        && (name.contains("otp")
            || name.contains("totp")
            || name == "code"
            || input.autocomplete.eq_ignore_ascii_case("one-time-code"))
}

fn is_username_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("user") || n.contains("email")
}

/// Recognises the only form shapes this driver is willing to fill; everything else is `None`.
fn classify_form(form: &html::Form) -> Option<PageKind> {
    if form.method != "post" {
        return None;
    }
    let password_count = form.inputs.iter().filter(|i| i.kind == "password").count();
    if password_count > 1 {
        // "Update password" / registration pages (password-new + password-confirm).
        return None;
    }
    let has_password = password_count == 1;
    let texts: Vec<&html::FormInput> = form.inputs.iter().filter(|i| is_text_like(i)).collect();
    let otp_count = texts.iter().filter(|i| is_otp_input(i)).count();
    if !has_password && otp_count == 1 && texts.len() == 1 {
        return Some(PageKind::Otp);
    }
    if otp_count > 0 {
        return None;
    }
    match (has_password, texts.len()) {
        (true, 0 | 1) => Some(PageKind::Login { has_password }),
        (false, 1) if is_username_name(&texts[0].name) => Some(PageKind::Login { has_password }),
        _ => None,
    }
}

/// Builds the POST fields the way a browser would: hidden values, the filled inputs, checked
/// boxes, one radio per group and the primary (non-cancel) submit button.
fn build_fields(
    form: &html::Form,
    creds: Option<&OidcCredentials>,
    otp: Option<&str>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut radios_done: Vec<&str> = Vec::new();
    let mut submit_done = false;
    for input in form.inputs.iter().filter(|i| !i.name.is_empty()) {
        let name = input.name.clone();
        match input.kind.as_str() {
            "password" => out.push((name, creds.map(|c| c.password.clone()).unwrap_or_default())),
            "hidden" => out.push((name, input.value.clone())),
            "checkbox" if input.checked => out.push((name, or_on(&input.value))),
            "radio" if !radios_done.contains(&input.name.as_str()) => {
                let group_checked = form
                    .inputs
                    .iter()
                    .any(|r| r.kind == "radio" && r.name == input.name && r.checked);
                if input.checked || !group_checked {
                    radios_done.push(&input.name);
                    out.push((name, input.value.clone()));
                }
            }
            "submit" if !submit_done && !input.name.to_ascii_lowercase().starts_with("cancel") => {
                submit_done = true;
                out.push((name, input.value.clone()));
            }
            _ if is_text_like(input) => {
                if let Some(code) = otp.filter(|_| is_otp_input(input)) {
                    out.push((name, code.to_string()));
                } else if let Some(c) = creds {
                    out.push((name, c.username.clone()));
                }
            }
            _ => {}
        }
    }
    out
}

fn or_on(value: &str) -> String {
    if value.is_empty() {
        "on".to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    const CTRL: &str = "https://boundary.example.test";
    const AUTH_URL: &str =
        "https://idp.example.test/realms/r/auth?client_id=boundary&state=STATE123&nonce=NONCE456";
    const CALLBACK: &str =
        "https://boundary.example.test/v1/auth-methods/amoidc_1:authenticate:callback?code=CODE789&state=STATE123";
    const PASSWORD: &str = "p@ss\"w\\ord&=%+";
    const OTP_SECRET: &str = "JBSWY3DPEHPK3PXP";

    fn login_page() -> String {
        r#"<html><head><title>Sign in</title></head><body>
<form id="kc-form-login" action="https://idp.example.test/realms/r/login-actions/authenticate?session_code=SC&amp;execution=E1&amp;tab_id=T" method="post">
<input id="username" name="username" type="text"/>
<input id="password" name="password" type="password"/>
<input type="checkbox" id="rememberMe" name="rememberMe"/>
<input type="hidden" name="credentialId" value=""/>
<input name="login" id="kc-login" type="submit" value="Sign In"/>
<input name="cancel" type="submit" value="Cancel"/>
</form></body></html>"#
            .to_string()
    }

    fn otp_page() -> String {
        r#"<html><head><title>Authenticate</title></head><body>
<form id="kc-otp-login-form" action="/realms/r/login-actions/authenticate?session_code=SC2&amp;execution=E2" method="post">
<input id="otp" name="otp" autocomplete="off" type="text" autofocus/>
<input type="hidden" name="credentialId" value="cred-1"/>
<input name="login" id="kc-login" type="submit" value="Sign In"/>
</form></body></html>"#
            .to_string()
    }

    fn json_resp(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            location: None,
            body: body.to_string(),
        }
    }

    fn page(body: String) -> HttpResponse {
        HttpResponse {
            status: 200,
            location: None,
            body,
        }
    }

    fn redirect(to: &str) -> HttpResponse {
        HttpResponse {
            status: 302,
            location: Some(to.to_string()),
            body: String::new(),
        }
    }

    fn start_ok() -> HttpResponse {
        json_resp(
            200,
            &format!(r#"{{"attributes":{{"auth_url":"{AUTH_URL}","token_id":"tokid_1"}}}}"#),
        )
    }

    fn token_ok() -> HttpResponse {
        json_resp(
            200,
            r#"{"attributes":{"token":"at_secret_token","expiration_time":"2026-11-19T00:00:00Z"}}"#,
        )
    }

    /// Replays scripted responses in order and records every request.
    struct Script {
        responses: VecDeque<Result<HttpResponse, OidcError>>,
        seen: RefCell<Vec<(String, String)>>,
    }

    impl Script {
        fn new(responses: Vec<HttpResponse>) -> Self {
            Self {
                responses: responses.into_iter().map(Ok).collect(),
                seen: RefCell::default(),
            }
        }
    }

    impl HttpTransport for Script {
        fn send(
            &mut self,
            req: &HttpRequest,
            _cancelled: &dyn Fn() -> bool,
        ) -> Result<HttpResponse, OidcError> {
            let desc = match &req.body {
                HttpBody::None => "GET".to_string(),
                HttpBody::Form(f) => format!(
                    "POST-FORM {}",
                    f.iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join("&")
                ),
                HttpBody::Json(j) => format!("POST-JSON {j}"),
            };
            self.seen.borrow_mut().push((req.url.clone(), desc));
            self.responses
                .pop_front()
                .unwrap_or_else(|| panic!("unexpected extra request to {}", req.url))
        }
    }

    fn creds(otp: Option<&str>) -> OidcCredentials {
        OidcCredentials {
            username: "alice".to_string(),
            password: PASSWORD.to_string(),
            otp_secret: otp.map(str::to_string),
        }
    }

    struct Run {
        result: Result<OidcToken, OidcError>,
        seen: Vec<(String, String)>,
        logs: Vec<String>,
    }

    fn run_with(
        responses: Vec<HttpResponse>,
        otp: Option<&str>,
        cancelled: &dyn Fn() -> bool,
    ) -> Run {
        run_addr(CTRL, responses, otp, cancelled)
    }

    fn run_addr(
        addr: &str,
        responses: Vec<HttpResponse>,
        otp: Option<&str>,
        cancelled: &dyn Fn() -> bool,
    ) -> Run {
        let mut net = Script::new(responses);
        let logs = RefCell::new(Vec::new());
        let c = creds(otp);
        let login = OidcLogin {
            addr,
            auth_method_id: "amoidc_1",
            credentials: &c,
        };
        let log = |s: &str| logs.borrow_mut().push(s.to_string());
        let totp = |secret: &str| {
            assert_eq!(secret, OTP_SECRET);
            Ok("654321".to_string())
        };
        let sleep = |_: Duration| {};
        let env = OidcEnv {
            cancelled,
            log: &log,
            totp: &totp,
            sleep: &sleep,
            poll_attempts: 3,
            poll_interval: Duration::from_millis(1),
        };
        let result = run_oidc_login(&mut net, &login, &env);
        Run {
            result,
            seen: net.seen.into_inner(),
            logs: logs.into_inner(),
        }
    }

    fn run(responses: Vec<HttpResponse>, otp: Option<&str>) -> Run {
        run_with(responses, otp, &|| false)
    }

    fn err(run: &Run) -> &OidcError {
        run.result.as_ref().err().expect("expected failure")
    }

    #[test]
    fn full_flow_with_password_and_otp_stores_token() {
        let r = run(
            vec![
                start_ok(),
                page(login_page()),
                page(otp_page()),
                redirect(CALLBACK),
                page("<html>Login successful</html>".into()),
                json_resp(400, "{}"),
                token_ok(),
            ],
            Some(OTP_SECRET),
        );
        let tok = r.result.as_ref().unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(tok.token, "at_secret_token");
        assert_eq!(tok.expiration_time.as_deref(), Some("2026-11-19T00:00:00Z"));

        let urls: Vec<&str> = r.seen.iter().map(|(u, _)| u.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://boundary.example.test/v1/auth-methods/amoidc_1:authenticate",
                AUTH_URL,
                "https://idp.example.test/realms/r/login-actions/authenticate?session_code=SC&execution=E1&tab_id=T",
                "https://idp.example.test/realms/r/login-actions/authenticate?session_code=SC2&execution=E2",
                CALLBACK,
                "https://boundary.example.test/v1/auth-methods/amoidc_1:authenticate",
                "https://boundary.example.test/v1/auth-methods/amoidc_1:authenticate",
            ]
        );
        assert_eq!(r.seen[0].1, r#"POST-JSON {"command":"start"}"#);
        assert_eq!(
            r.seen[2].1,
            format!("POST-FORM username=alice&password={PASSWORD}&credentialId=&login=Sign In"),
            "unchecked checkbox and cancel button must be omitted"
        );
        assert_eq!(
            r.seen[3].1,
            "POST-FORM otp=654321&credentialId=cred-1&login=Sign In"
        );
        assert_eq!(r.seen[4].1, "GET", "callback is a body-less GET");
        assert!(r.seen[5].1.contains(r#""command":"token""#));
        assert!(r.seen[5].1.contains("tokid_1"));

        let all_logs = r.logs.join("\n");
        for secret in [
            PASSWORD,
            OTP_SECRET,
            "654321",
            "at_secret_token",
            "STATE123",
            "NONCE456",
            "CODE789",
            "session_code",
            "alice",
        ] {
            assert!(
                !all_logs.contains(secret),
                "log leaked {secret}: {all_logs}"
            );
        }
        assert!(!r.logs.is_empty());
    }

    #[test]
    fn password_only_idp_skips_otp_and_does_not_need_a_secret() {
        let r = run(
            vec![
                start_ok(),
                page(login_page()),
                redirect(CALLBACK),
                page("ok".into()),
                token_ok(),
            ],
            None,
        );
        assert_eq!(r.result.unwrap().token, "at_secret_token");
    }

    #[test]
    fn relative_redirects_on_the_idp_are_followed() {
        let r = run(
            vec![
                start_ok(),
                redirect("/realms/r/login-actions/next?x=1"),
                page(login_page()),
                redirect(CALLBACK),
                page("ok".into()),
                token_ok(),
            ],
            None,
        );
        assert!(r.result.is_ok(), "{:?}", r.result.err());
        assert_eq!(
            r.seen[2].0,
            "https://idp.example.test/realms/r/login-actions/next?x=1"
        );
    }

    #[test]
    fn wrong_password_is_reported_when_login_form_returns() {
        let r = run(
            vec![start_ok(), page(login_page()), page(login_page())],
            None,
        );
        assert_eq!(err(&r).reason, "oidc_credentials_rejected");
        assert!(!err(&r).message.contains(PASSWORD));
    }

    #[test]
    fn wrong_otp_is_reported_when_otp_form_returns() {
        let r = run(
            vec![
                start_ok(),
                page(login_page()),
                page(otp_page()),
                page(otp_page()),
            ],
            Some(OTP_SECRET),
        );
        assert_eq!(err(&r).reason, "oidc_otp_rejected");
    }

    #[test]
    fn otp_page_without_configured_secret_fails_closed_without_posting() {
        let r = run(vec![start_ok(), page(login_page()), page(otp_page())], None);
        assert_eq!(err(&r).reason, "oidc_otp_required");
        assert_eq!(r.seen.len(), 3, "no OTP request may be sent");
    }

    #[test]
    fn unrecognised_idp_page_fails_closed_naming_only_the_title() {
        for body in [
            "<html><head><title>Grant access?</title></head><body>no form</body></html>"
                .to_string(),
            r#"<title>Captcha</title><form action="/x" method="post"><input name="username"><input name="captcha" type="text"></form>"#
                .to_string(),
            r#"<title>Update</title><form action="/x" method="post"><input name="newpass" type="text"></form>"#
                .to_string(),
        ] {
            let r = run(vec![start_ok(), page(body)], Some(OTP_SECRET));
            assert_eq!(err(&r).reason, "oidc_idp_unsupported");
            assert_eq!(r.seen.len(), 2, "nothing may be posted to an unknown page");
        }
        let r = run(
            vec![start_ok(), page("<title>Grant access?</title>".into())],
            None,
        );
        assert!(err(&r).message.contains("Grant access?"));
    }

    #[test]
    fn redirect_to_a_foreign_host_is_blocked_before_any_request() {
        let r = run(
            vec![start_ok(), redirect("https://evil.example.test/steal")],
            None,
        );
        assert_eq!(err(&r).reason, "oidc_redirect_blocked");
        assert_eq!(r.seen.len(), 2);
        assert!(err(&r).message.contains("evil.example.test"));
        assert!(!err(&r).message.contains("steal"));
    }

    #[test]
    fn redirect_downgrade_to_http_is_blocked() {
        let r = run(
            vec![
                start_ok(),
                redirect("http://idp.example.test/realms/r/auth"),
            ],
            None,
        );
        assert_eq!(err(&r).reason, "oidc_redirect_blocked");
    }

    #[test]
    fn form_action_pointing_at_another_host_never_receives_credentials() {
        let evil = login_page().replace(
            "https://idp.example.test/realms",
            "https://evil.test/realms",
        );
        let r = run(vec![start_ok(), page(evil)], None);
        assert_eq!(err(&r).reason, "oidc_redirect_blocked");
        assert_eq!(r.seen.len(), 2, "credentials must not be sent anywhere");
    }

    #[test]
    fn get_method_login_form_is_unsupported_so_credentials_never_go_in_a_query() {
        let get_form = login_page().replace(r#"method="post""#, r#"method="get""#);
        let r = run(vec![start_ok(), page(get_form)], None);
        assert_eq!(err(&r).reason, "oidc_idp_unsupported");
        assert_eq!(r.seen.len(), 2);
    }

    #[test]
    fn only_one_callback_hop_to_the_controller_is_allowed_and_it_is_not_followed() {
        // Controller callback answers with a redirect: it must not be followed.
        let r = run(
            vec![
                start_ok(),
                page(login_page()),
                redirect(CALLBACK),
                redirect("https://evil.example.test/after"),
                token_ok(),
            ],
            None,
        );
        assert!(r.result.is_ok(), "{:?}", r.result.err());
        assert!(!r.seen.iter().any(|(u, _)| u.contains("evil")));

        // A non-callback controller URL is not an allowed hop.
        let r = run(
            vec![
                start_ok(),
                redirect("https://boundary.example.test/v1/scopes"),
            ],
            None,
        );
        assert_eq!(err(&r).reason, "oidc_redirect_blocked");
    }

    #[test]
    fn callback_failure_is_reported() {
        let r = run(
            vec![
                start_ok(),
                page(login_page()),
                redirect(CALLBACK),
                json_resp(500, "boom"),
            ],
            None,
        );
        assert_eq!(err(&r).reason, "oidc_callback_failed");
    }

    #[test]
    fn insecure_or_bad_addresses_are_rejected_before_any_request() {
        let r = run_addr("http://boundary.example.test", vec![], None, &|| false);
        assert_eq!(err(&r).reason, "oidc_insecure_url");
        assert!(r.seen.is_empty());
        let r = run_addr("not a url", vec![], None, &|| false);
        assert_eq!(err(&r).reason, "oidc_insecure_url");

        let bad_auth = json_resp(
            200,
            r#"{"attributes":{"auth_url":"http://idp.example.test/a","token_id":"t"}}"#,
        );
        let r = run(vec![bad_auth], None);
        assert_eq!(err(&r).reason, "oidc_insecure_url");
    }

    #[test]
    fn http_is_allowed_for_loopback_fakes() {
        let start = json_resp(
            200,
            r#"{"attributes":{"auth_url":"http://127.0.0.1:7001/auth","token_id":"t"}}"#,
        );
        let cb = "http://127.0.0.1:7000/v1/auth-methods/amoidc_1:authenticate:callback?code=c";
        let r = run_addr(
            "http://127.0.0.1:7000/",
            vec![start, redirect(cb), page("ok".into()), token_ok()],
            None,
            &|| false,
        );
        assert!(r.result.is_ok(), "{:?}", r.result.err());
        assert_eq!(
            r.seen[0].0,
            "http://127.0.0.1:7000/v1/auth-methods/amoidc_1:authenticate"
        );
    }

    #[test]
    fn start_failures_are_classified() {
        let r = run(vec![json_resp(403, "{\"message\":\"nope\"}")], None);
        assert_eq!(err(&r).reason, "oidc_start_failed");
        assert!(err(&r).message.contains("403"));
        assert!(!err(&r).message.contains("nope"), "bodies are never echoed");
        let r = run(vec![json_resp(200, "{\"attributes\":{}}")], None);
        assert_eq!(err(&r).reason, "oidc_start_invalid");
        let r = run(vec![json_resp(200, "not json")], None);
        assert_eq!(err(&r).reason, "oidc_start_invalid");
    }

    #[test]
    fn cancellation_stops_between_steps() {
        let calls = std::cell::Cell::new(0u32);
        let cancelled = || {
            calls.set(calls.get() + 1);
            calls.get() > 2
        };
        let r = run_with(
            vec![start_ok(), page(login_page()), page(otp_page())],
            Some(OTP_SECRET),
            &cancelled,
        );
        assert_eq!(err(&r).reason, "oidc_cancelled");
        assert!(r.seen.len() < 3);
    }

    #[test]
    fn token_poll_is_bounded_and_classifies_failures() {
        let steps = || {
            vec![
                start_ok(),
                page(login_page()),
                redirect(CALLBACK),
                page("ok".into()),
            ]
        };
        let mut v = steps();
        v.extend([
            json_resp(400, "{}"),
            json_resp(404, "{}"),
            json_resp(400, "{}"),
        ]);
        let r = run(v, None);
        assert_eq!(err(&r).reason, "oidc_token_timeout");

        let mut v = steps();
        v.push(json_resp(500, "oops"));
        let r = run(v, None);
        assert_eq!(err(&r).reason, "oidc_token_poll_failed");
        assert!(err(&r).message.contains("500"));

        // 200 without a token yet keeps polling.
        let mut v = steps();
        v.extend([json_resp(200, "{\"attributes\":{}}"), token_ok()]);
        assert!(run(v, None).result.is_ok());
    }

    #[test]
    fn redirect_loops_hit_the_step_limit() {
        let mut v = vec![start_ok()];
        for _ in 0..MAX_IDP_STEPS + 2 {
            v.push(redirect("/loop"));
        }
        let r = run(v, None);
        assert_eq!(err(&r).reason, "oidc_too_many_redirects");
    }

    #[test]
    fn update_password_page_is_unsupported_not_reported_as_a_wrong_password() {
        let update = r#"<title>Update password</title><form action="/realms/r/login-actions/required-action" method="post">
<input type="password" id="password-new" name="password-new"/>
<input type="password" id="password-confirm" name="password-confirm"/>
<input name="login" type="submit" value="Submit"/></form>"#;
        let r = run(
            vec![start_ok(), page(login_page()), page(update.into())],
            None,
        );
        assert_eq!(err(&r).reason, "oidc_idp_unsupported");
        assert!(err(&r).message.contains("Update password"));
        assert_eq!(r.seen.len(), 3, "the stored password must not be re-sent");
    }

    #[test]
    fn two_step_username_then_password_is_supported() {
        let username_only = r#"<form action="/u" method="post"><input name="username" type="text"><input name="login" type="submit" value="Next"></form>"#;
        let password_only =
            login_page().replace(r#"<input id="username" name="username" type="text"/>"#, "");
        let r = run(
            vec![
                start_ok(),
                page(username_only.into()),
                page(password_only),
                redirect(CALLBACK),
                page("ok".into()),
                token_ok(),
            ],
            None,
        );
        assert!(r.result.is_ok(), "{:?}", r.result.err());
        assert_eq!(r.seen[2].1, "POST-FORM username=alice&login=Next");
    }
}
