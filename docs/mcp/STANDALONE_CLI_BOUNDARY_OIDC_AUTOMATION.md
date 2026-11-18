# Standalone CLI: Boundary OIDC login automation (design proposal)

Status: **Phase 1 implemented; Phase 2 designed, not implemented.** Maintainer decisions are recorded in [Decisions](#decisions).

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
2. **Phase 2, option B (not implemented)**, behind a spike that resolves the OTP-page and keyring unknowns:
   - Move the HTTP flow's types and loop out of `src-tauri` into `scp-core::boundary_oidc` behind a small `IdpDriver` trait; keep the Keycloak form driver separate so other IdPs can be added.
   - Reuse `generate_totp_code_*`. Read credentials only from the profile keyring.
   - Fail closed with a clear `reason` (for example `oidc_idp_unsupported`, `oidc_otp_required`) rather than guessing at unknown pages; fall back to Phase 1 when interactive.
   - Optional refresh: before a Boundary operation, if the stored token's `expiration_time` is near, re-run the login (only under `runtime-control` + `allow-local-policy` over MCP).

## Security constraints

- Send credentials only over `https` to the host of the `auth_url` returned by the controller; reject redirects to other hosts or to `http`.
- Never log request/response bodies, cookies, the password, the OTP secret or the token; add them to the existing secret redaction set.
- Over MCP, expose automated login only under `runtime-control` with `allow-local-policy`; `require-ui-approval` style policies must still reject it.
- Keep CLI and desktop token stores separate (different credential namespaces). Do not copy the desktop app token.

## Test plan (TDD, no real IdP)

- Fake Boundary controller + fake Keycloak (local HTTP server in tests) returning: start response, login form, OTP form, redirect to callback, token poll sequence (400 → 200).
- Cases: wrong password (form re-rendered with error), wrong/expired OTP, unsupported page, redirect to a foreign host, `http` auth URL, cancellation, token stored under the app-token key, no secret in logs/`--json` output.
- Contract tests: `schema` command/operation counts and the operation ledger change deliberately; update `package-scpctl` isolation expectations if a new crate is added.

## Phase 1 as implemented

- `ProfileRuntime::start_boundary_auth_with_options(cancel, BoundaryAuthOptions { allow_browser_oidc })`; `start_boundary_auth` (used by MCP) keeps `allow_browser_oidc: false`, so headless callers still get `webview_oidc_desktop_only` / `browser_oidc_unverified` (message now names `scpctl boundary auth --interactive`; reason codes and `nextAction` unchanged).
- `scpctl boundary auth --interactive` passes the flag. It runs `boundary authenticate oidc -auth-method-id <id> -format json` through the same supervised launch as password login (`BOUNDARY_ADDR`, `BOUNDARY_TOKEN_NAME=scpctl-<ns>`, no password injection); the existing exit hook captures the token into the profile app token store. Timeout 5 minutes interactive, 30 seconds otherwise. Interactive OIDC prints "Complete the Boundary login in your browser (waiting up to 5 min)..." to stderr (stdout stays pure JSON). The supervisor inherits the parent environment, so `DISPLAY`/D-Bus reach the browser launcher (checked with a fake Boundary).
- Tests: `slice2_boundary.rs` (`boundary_oidc_login_runs_stock_cli_when_interactive_opted_in`, `boundary_oidc_interactive_requires_complete_config`) and `cli_slice2.rs::cli_boundary_auth_modes` use a fake Boundary executable. **Not yet exercised against the real controller with this code path** (an earlier manual run of the stock command proved the stock CLI side).

## Decisions

Decided by the maintainer:

1. **Use `curl`** for Phase 2's HTTP, as an external prerequisite (like `ssh`, `boundary`, `openvpn`), instead of a Rust HTTP crate. The dependency-isolation checks (`FORBIDDEN_CRATES`) therefore stay unchanged.
2. **Storing/using the OIDC password and TOTP secret in the CLI profile: accepted** (same exposure as the desktop's automated mode).
3. **Scope: both phases.** Phase 1 is implemented; Phase 2 follows.

Proposed, not yet reviewed by the maintainer (implementation rules for decision 1):

- Credentials and request bodies go to curl on stdin (`-K -`), never argv.
- `--proto =https`; plain `http` only for loopback hosts, so tests can use local fakes.
- Follow redirects manually (no `-L`); reject any hop to a different host or to `http`.
- Private cookie jar in a `0700` temp dir, deleted afterwards; never log bodies or cookies.

Still open:

4. Verifying Keycloak's OTP page against the real IdP requires one real login from a script (real password plus a live TOTP code). Needs the owner's explicit go-ahead; alternative is to build against fakes and have the owner run the first real login with the finished tool.
5. Confirm the CLI profile keyring holds `boundary_oidc_password` and `boundary_oidc_otp_secret` after import (presence-only probe, no values).
