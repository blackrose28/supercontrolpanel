# Standalone CLI: Boundary OIDC login automation (design proposal)

Status: **Phase 1 and Phase 2 implemented and tested against local fakes; Phase 2 has not yet been run against the real IdP** (see [Still open](#still-open)). Maintainer decisions are recorded in [Decisions](#decisions).

## Problem

The desktop app logs in to Boundary OIDC automatically (`boundary_service.rs::run_boundary_oidc_automation`): it calls the Boundary HTTP auth API, drives the IdP login form in a hidden Tauri webview using the saved OIDC username, password and TOTP secret, and stores the resulting token as the app token (`KEYRING_BOUNDARY_APP_TOKEN`).

`scpctl` cannot do this. `validate_boundary_auth_mode` (`crates/scp-core/src/boundary_ops.rs`) rejects `oidc` with `webview_oidc_desktop_only` (`automated`) and `browser_oidc_unverified` (`browser`). The only working route is a manual login with the stock Boundary CLI under the token name `scpctl-<credentialNamespace>` (see the [Agent guide](STANDALONE_CLI_AGENT_GUIDE.md#boundary-oidc-login-known-limitation)). Tokens last about a day, so this is a daily manual step, and it is invisible to unattended/MCP use.

## Verified facts (Linux x86_64, Boundary CLI 2025-02 build, one controller)

- The stock `boundary authenticate oidc` opens the login URL through the first `xdg-open` on `PATH` and then **keeps polling for the token itself**, storing it under `BOUNDARY_TOKEN_NAME` (spike: a shim `xdg-open` received the URL; the CLI kept polling until killed).
- The IdP is Keycloak. Its first page is a plain server-rendered form (`#kc-form-login`, fields `username`, `password`, `credentialId`) with no captcha, reachable with a cookie-aware HTTP client.
- `scp-core` already has TOTP generation (`openvpn_ops.rs::generate_totp_code_*`).
- The desktop flow's API shapes are small: `POST {addr}/v1/auth-methods/{id}:authenticate` with `{"command":"start"}` returns `attributes.{auth_url,token_id}`; the same endpoint with `{"command":"token","token_id":…}` is polled (HTTP 400/404 = not ready) until `attributes.{token,expiration_time}` appear.

## Not yet verified

- The Keycloak **OTP step** page shape (second form after the password). Confirming it requires sending the real password from a script; do that only with the owner's explicit go-ahead.
- Whether the CLI profile keyring actually holds `boundary_oidc_password` and `boundary_oidc_otp_secret` after a backup import. `settings show` does not report it. A presence-only probe is needed (do not print values).
- Behaviour on macOS/Windows, other IdPs, and IdPs with captcha, WebAuthn or consent screens.

## Options

| Option | How | Pros | Cons |
| --- | --- | --- | --- |
| **A. scpctl runs the stock browser login** (interactive) | `scpctl boundary auth --interactive` spawns `boundary authenticate oidc -auth-method-id <id>` with `build_boundary_env` (already sets `BOUNDARY_ADDR` and `BOUNDARY_TOKEN_NAME`) | Small. Removes the token-name pitfall. No new dependencies. Works for any IdP. Same code path the user ran by hand. | Still needs a human and a browser each time. |
| **B. Headless HTTP flow** (recommended end state) | Port the desktop flow: auth `start`, HTTP-drive the IdP form with a cookie jar, TOTP from keyring, poll `token`, store as `KEYRING_BOUNDARY_APP_TOKEN` (search/connect already inject it via `env://BOUNDARY_TOKEN`) | Fully unattended; works for MCP `boundary_auth_start`; cross-platform; token name stops mattering. | Needs an HTTP client (see [Decisions](#decisions)). Form scraping is IdP-specific and brittle. Stores/uses password + OTP secret (same exposure as the desktop's automated mode). |
| C. B with a shim `xdg-open` | Run stock `boundary authenticate oidc` with a `PATH` shim that performs the IdP form fill | Boundary CLI keeps the token handling. | Linux-only trick (`xdg-open`); macOS uses `open`, Windows differs. Still needs the HTTP/form code from B, so it adds nothing over B. **Not recommended.** |
| D. Headless browser (Playwright/Chromium) | Drive a real browser | Handles any IdP page. | Very large dependency; violates the GUI-free, small-binary goal. **Not recommended.** |

## Recommendation

1. **Phase 1, option A (implemented).** Smallest change that makes the CLI usable and kills the known mistake:
   - In interactive mode, for `authKind: oidc` in **both** `automated` and `browser` modes (`oidcLoginMode` is shared config synced from the desktop, so `automated` is common on the CLI), spawn the stock login with `build_boundary_env` instead of returning `CAPABILITY_UNAVAILABLE`.
   - Keep the existing `reason` codes for `--non-interactive` and MCP.
   - Make the remaining error messages name the real fix (the exact command and token name) instead of `use_password_auth_or_desktop`.
   - Before editing, run `ripwire --impact`/`--uses` on `validate_boundary_auth_mode`; existing tests likely assert `browser_oidc_unverified`.
2. **Phase 2, option B (implemented, see [Phase 2 as implemented](#phase-2-as-implemented))**, originally planned behind a spike on the OTP-page and keyring unknowns:
   - Move the HTTP flow's types and loop out of `src-tauri` into `scp-core::boundary_oidc` behind a small `IdpDriver` trait; keep the Keycloak form driver separate so other IdPs can be added.
   - Reuse `generate_totp_code_*`. Read credentials only from the profile keyring.
   - Fail closed with a clear `reason` (for example `oidc_idp_unsupported`, `oidc_otp_required`) rather than guessing at unknown pages; fall back to Phase 1 when interactive.
   - Optional refresh: before a Boundary operation, if the stored token's `expiration_time` is near, re-run the login (only under `runtime-control` + `allow-local-policy` over MCP).

## Security constraints

- Send credentials only over `https` to the host of the `auth_url` returned by the controller; reject redirects to other hosts or to `http`, except the single body-less OIDC callback `GET` to the controller (see [Decisions](#decisions)).
- Never log request/response bodies, cookies, the password, the OTP secret or the token; add them to the existing secret redaction set.
- Over MCP the automated login is reached only through `boundary_auth_start`, which is a `runtime-control`, non-read-only operation: `authorize_standalone` (`crates/scp-mcp/src/contracts.rs`) already denies it unless the profile's `stdio-policy.json` has `sensitiveActionPolicy: allow-local-policy` **and** lists `runtime-control` in `allowedToolCategories` (the default read-only policy never exposes it). No separate gate was added; the HTTP desktop transport's `require-ui-approval` policy is not involved because the desktop keeps its own webview login.
- Keep CLI and desktop token stores separate (different credential namespaces). Do not copy the desktop app token.

## Test plan (TDD, no real IdP)

- Fake Boundary controller + fake Keycloak (local HTTP server in tests) returning: start response, login form, OTP form, redirect to callback, token poll sequence (400 → 200).
- Cases: wrong password (form re-rendered with error), wrong/expired OTP, unsupported page, redirect to a foreign host, `http` auth URL, cancellation, token stored under the app-token key, no secret in logs/`--json` output.
- Contract tests: `schema` command/operation counts and the operation ledger change deliberately; update `package-scpctl` isolation expectations if a new crate is added.

## Phase 1 as implemented

- `ProfileRuntime::start_boundary_auth_with_options(cancel, BoundaryAuthOptions { allow_browser_oidc })`; `start_boundary_auth` (used by MCP) keeps `allow_browser_oidc: false`, so headless callers still get `webview_oidc_desktop_only` / `browser_oidc_unverified` (message now names `scpctl boundary auth --interactive`; reason codes and `nextAction` unchanged).
- `scpctl boundary auth --interactive` passes the flag. It runs `boundary authenticate oidc -auth-method-id <id> -format json` through the same supervised launch as password login (`BOUNDARY_ADDR`, `BOUNDARY_TOKEN_NAME=scpctl-<ns>`, no password injection); the existing exit hook captures the token into the profile app token store. Timeout 5 minutes interactive, 30 seconds otherwise. Interactive OIDC prints "Complete the Boundary login in your browser (waiting up to 5 min)..." to stderr (stdout stays pure JSON). The supervisor inherits the parent environment, so `DISPLAY`/D-Bus reach the browser launcher (checked with a fake Boundary).
- Tests: `slice2_boundary.rs` (`boundary_oidc_login_runs_stock_cli_when_interactive_opted_in`, `boundary_oidc_interactive_requires_complete_config`) and `cli_slice2.rs::cli_boundary_auth_modes` use a fake Boundary executable. **Not yet exercised against the real controller with this code path** (an earlier manual run of the stock command proved the stock CLI side).

## Phase 2 as implemented

Modules (`crates/scp-core/src/`):

- `boundary_oidc/mod.rs`: transport-agnostic flow `run_oidc_login(&mut dyn HttpTransport, &OidcLogin, &OidcEnv)`: `start` → IdP pages → callback → bounded token poll (20 × 500 ms). The built-in Keycloak-style driver fills (a) a form with a password input (plus the username input if present; a username-only first step is also accepted) and (b) an OTP form (one text input named `*otp*`/`*totp*`/`code` or `autocomplete=one-time-code`, no password). It builds the POST like a browser (hidden inputs, checked boxes, one radio per group, the first non-`cancel` submit). Anything else (unknown visible inputs such as a captcha, `method=get`, no form, consent pages) fails closed with `oidc_idp_unsupported` naming only the page title. A form with more than one password input (Keycloak's update-password / registration pages) is also `oidc_idp_unsupported`, so it is never mistaken for a wrong password. The TOTP code is generated when the OTP page is reached (30 s window). Because the driver is selected by page shape, other IdPs can be supported by extending `classify_form`; there is no separate `IdpDriver` trait yet.
- `boundary_oidc/html.rs` and `url.rs`: dependency-free form/URL parsing (entity decoding, `&amp;` in actions, userinfo rejected, `http` only for loopback hosts).
- `boundary_oidc/curl.rs`: `CurlTransport`. Each request runs `curl -q -K -` with the config (URL, `data-urlencode`, JSON body, cookie jar, timeouts, `proto = "=https"` or `"=http"` for loopback) on **stdin**, so passwords, OTP codes and `auth_url` query strings never reach argv. No `-L`: redirects come back as `status + redirect_url`. The cookie jar and body file live in a `0700` temp dir deleted on drop. A cancel flag is polled every 20 ms and kills the curl process. Exit codes map to `oidc_network_error` / `oidc_network_timeout` / `oidc_tls_error` / `oidc_curl_failed` without echoing curl's stderr.
- `supervisor.rs`: **task sessions** (`spawn_task_with_gate`, `TaskContext`, `SessionControl::Task`). The flow runs on a thread but is a normal supervised session: it has a session id, `wait_for_session_exit`, `stop_session`/`stop_function` (sets the cancel flag and waits), the function log ring, `shutdown_all` ordering and the usual exit callback. `write` on a task fails with `task_not_writable`; `resize` is a no-op.
- `boundary_ops.rs`: `BoundaryAuthPlan::HeadlessOidc`. Routing for `authKind: oidc`: `automated` mode + complete credentials → headless (also when `allow_browser_oidc` is set, i.e. interactive); `automated` without credentials → `credentials_missing` (lists what is missing, never values) unless interactive, which uses the stock browser login; `browser` mode → stock browser login only when interactive, else `browser_oidc_unverified`. `BoundaryAuthOptions.skip_headless_oidc` forces the browser route; `ProfileRuntime::boundary_auth_route` reports the route without starting anything. Credentials: `boundary.oidcUsername` from config, `boundary_oidc_password` / `boundary_oidc_otp_secret` from the profile keyring (legacy plaintext values are migrated into the keyring when the profile loads; there is no plaintext fallback). A missing OTP secret only matters if the IdP asks for a code (`oidc_otp_required`).
- Success stores the token under `KEYRING_BOUNDARY_APP_TOKEN` and marks `boundary-auth` `authenticated` with token source `app`. Failure ends the session with exit code 1 and the status message `"<reason>: <message>"`.

CLI (`scpctl boundary auth`): the route decides the timeout (headless 90 s, browser 5 min, password 30 s) and the stderr hint. With `--interactive`, `oidc_idp_unsupported`, `oidc_otp_required` and `curl_not_found` fall back to the browser login once (`skip_headless_oidc`); wrong password/OTP, network and TLS errors do not. Failures now include the status message in the `boundary_auth_failed` error. MCP `boundary_auth_start` is unchanged (same `runtime-control` gate described under Security constraints; no new tool, so schema/ledger counts are unchanged) and now starts the headless login for automated mode with credentials. The task is host-owned: the per-request cancel token only guards acceptance (`TaskContext::is_cancelled` reflects `stop_session` only), so a client cancelling or disconnecting after the start call returns does not abort the login; `boundary_auth_stop`, the CLI signal handler and runtime shutdown do.

Reason codes: `credentials_missing`, `curl_not_found` (start-time `CAPABILITY_UNAVAILABLE`); `oidc_start_failed`, `oidc_start_invalid`, `oidc_insecure_url`, `oidc_redirect_blocked`, `oidc_idp_http_error`, `oidc_idp_unsupported`, `oidc_credentials_rejected`, `oidc_otp_required`, `oidc_otp_rejected`, `oidc_otp_invalid_secret`, `oidc_callback_failed`, `oidc_too_many_redirects`, `oidc_token_poll_failed`, `oidc_token_timeout`, `oidc_token_store_failed`, `oidc_cancelled`, `oidc_network_error`, `oidc_network_timeout`, `oidc_tls_error`, `oidc_curl_failed`, `oidc_curl_not_found`, `oidc_request_invalid`, `oidc_tempdir_failed` (task status message prefix).

Contract changes: `webview_oidc_desktop_only` is gone (automated mode without credentials is `credentials_missing`); `BoundaryAuthOptions` gained `skip_headless_oidc`; `browser_oidc_unverified` is unchanged.

Tests (no real IdP): unit tests with a scripted fake transport (full flow with OTP, password-only, wrong password/OTP, unknown pages, foreign-host redirects and form actions, `http` downgrade, callback rules, bounded poll, cancel, redirect loops, two-step login); `CurlTransport` against std `TcpListener` fakes (hostile password characters, cookies, no-redirect, stdin-only secrets via a fake `curl`, missing curl, cancel); `slice2_task_sessions.rs`; `slice2_boundary_oidc_headless.rs` (real `curl` against loopback controller + Keycloak-like IdP through `ProfileRuntime`: success, errors, stop, no secret in logs, interactive preference); `cli_boundary_oidc.rs` (headless, fail-closed, fallback, no fallback on wrong password). Tests need `curl` on `PATH`.

## Decisions

Decided by the maintainer:

1. **Use `curl`** for Phase 2's HTTP, as an external prerequisite (like `ssh`, `boundary`, `openvpn`), instead of a Rust HTTP crate. The dependency-isolation checks (`FORBIDDEN_CRATES`) therefore stay unchanged.
2. **Storing/using the OIDC password and TOTP secret in the CLI profile: accepted** (same exposure as the desktop's automated mode).
3. **Scope: both phases.** Phase 1 is implemented; Phase 2 follows.

Implementation rules for decision 1 (implemented as proposed, with one refinement; still worth a maintainer review):

- Credentials and request bodies go to curl on stdin (`-K -`), never argv.
- `--proto =https`; plain `http` only for loopback hosts, so tests can use local fakes.
- Follow redirects manually (no `-L`). Credentials and OTP codes are POSTed only to the authority (host + port) of the `auth_url` the controller returned. **Refinement:** the original wording "reject any hop to a different host" would block the flow itself, because after the last IdP step Keycloak redirects to the Boundary callback on the controller host and the token is only issued once that request lands. The flow therefore allows exactly one body-less `GET` to the configured controller authority whose path is `/v1/auth-methods/…:callback…`, and never follows that response; every other foreign host, `http` downgrade or non-callback controller URL is `oidc_redirect_blocked`.
- Private cookie jar in a `0700` temp dir, deleted afterwards; never log bodies or cookies. Task log lines are fixed stage messages only (no `auth_url`, callback URL, cookies, username, code or token), because logs are exposed through `get_function_logs` and MCP.

## Still open

4. **Real-IdP verification.** Nothing in this repository has contacted the real Keycloak. The OTP page shape and the exact form fields are still assumed from the desktop automation's selectors and Keycloak's standard theme. Needs the owner's explicit go-ahead to run one real login with the saved credentials (`scpctl boundary auth --json`); a failure names its reason code, and `oidc_idp_unsupported` reports the page title so the driver can be extended.
5. **Keyring presence after import.** Answered by the tool itself: `scpctl boundary auth --json` returns `credentials_missing` naming the missing item (username / password), and `oidc_otp_required` if the IdP asks for a code but no OTP secret is stored; no values are ever printed.
6. macOS/Windows behaviour (`curl.exe` ships with Windows 10+ and macOS), IdPs with captcha/WebAuthn/consent screens (fail closed by design), and the optional pre-operation token refresh are not covered.
