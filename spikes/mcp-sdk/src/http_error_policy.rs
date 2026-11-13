//! Standards-preserving response policy candidate for the pinned SDK.
//!
//! Qualified only in the isolated spike; production transport activation and
//! admission, logging, parser/header/session and native-output audits are separate.
use axum::Router;

/// Mount directly outside the SDK route, inside independently owned admission.
///
/// Map only finite, exact-size (<= 64 KiB), application/json HTTP 400 negotiation
/// errors with codes -32020, -32022, -32600 or -32602. Retain status/code/ID form,
/// replace message with fixed text, omit data and rebuild response headers.
/// Preserve every excluded response, notably modern HTTP 404 correlated errors,
/// legacy HTTP 200 errors/SSE, successes and accepted notifications.
///
/// Do not compose with `with_fixed_http_errors`: either ordering erases required
/// protocol envelopes. Exclusions pass through; this is not a general redactor,
/// authentication policy or guarantee of safe arbitrary headers/error bodies.
/// The inherited body-read failure response is fixed but cannot preserve an
/// unreadable envelope; pinned SDK Full JSON bodies are Infallible.
pub fn with_http_error_policy(router: Router) -> Router {
    crate::negotiation_errors::with_fixed_negotiation_errors(router)
}
