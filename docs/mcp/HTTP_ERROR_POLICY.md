# Standards-preserving HTTP error response policy

Status: narrow response policy implemented and qualified in the isolated rmcp
3.10.0 spike, 2026-11-13. Production transport activation remains separate.

## Selected contract

Use `http_error_policy::with_http_error_policy` immediately outside the SDK route,
inside separately owned admission/authentication middleware. It reuses the bounded
negotiation mapper rather than the blanket HTTP-status experiment. Never compose
it with `with_fixed_http_errors`, in either order: that experiment destroys MCP
error envelopes. Authentication and admission must own their own fixed responses;
this layer deliberately does not rewrite their statuses, bodies or headers.

| Response | Policy |
| --- | --- |
| HTTP 400, exact `application/json`, exact body size at most 64 KiB, deserializable SDK JsonRpcError, code -32020, -32022, -32600 or -32602 | Preserve HTTP status, error code and ID value/form (integer, string, explicit null or omitted); replace message with `MCP request negotiation failed`; omit error data; rebuild response headers with JSON content type |
| Modern HTTP 404 correlated protocol error | Preserve body, status and headers, including correlation and fixed adapter error |
| Legacy HTTP 200 protocol error, including initialized SSE | Preserve body, status, headers and streaming |
| Success, accepted notification, other status/content type/code, malformed envelope, unknown or excessive body size | Pass through unchanged |

No SSE stream is buffered or parsed. Only the selected finite JSON bodies are
read. The mapper's fixed `MCP negotiation response unavailable` HTTP 400 fallback
covers body-read failure without formatting native errors; an unreadable envelope
cannot preserve correlation. The pinned SDK producers use Infallible Full JSON
bodies, so this fallback is not claimed as SDK production-path evidence.

The predicate is intentionally the existing SDK-deserializable error contract,
not a new strict JSON-envelope validator. This is not blanket response sanitization:
excluded values and headers may contain supplied data. IDs and successful results
are not sanitized. Neither this layer nor the adapter suppresses SDK tracing.
Parser/header/session errors outside the selected predicate, auth/Origin/session
ownership, logging, time/volume bounds, native output and third-party client
qualification remain independent release gates.

## Pinned source review

Inspected `rmcp-3.10.0/src/transport/streamable_http_server/tower.rs` from the locked
published crate, specifically `invalid_request_jsonrpc_response`,
`invalid_params_jsonrpc_response`, `header_mismatch_jsonrpc_response`,
`jsonrpc_http_status` and JSON response construction. Negotiation producers return
HTTP 400 JSON; modern METHOD_NOT_FOUND maps to HTTP 404; legacy requests bypass
that mapping and retain HTTP 200. These facts require preserving envelopes rather
than treating every HTTP failure as a plain-text transport failure.

The existing overview adapter supplies fixed custom-method errors. This response
policy preserves those errors; it does not generalize that adapter's error safety
to arbitrary handlers. Existing raw negotiation/error experiments remain available
as characterization, including tests proving destructive blanket composition.

## Behavioral qualification

Focused RED used an identity implementation at the new entry point. Real SDK
negotiation failed on reflected synthetic secret text; the finite-envelope test
failed on retained diagnostic headers. Existing protocol/exclusion checks passed.
GREEN delegates to the existing mapper. Shared assertion helpers retain the old
experiment's tests and apply the same contracts to the new policy.

Five added tests cover:

- Raw real SDK negotiation matrix, then custom-request errors with exact modern
  404 versus legacy 200 status, JSON versus initialized SSE, correlation and a
  usable catalog afterward. Rejected requests never enter the synthetic host.
- Official-client protocol error decoding and catalog recovery in both revisions
  and response modes.
- Official-client negotiation error decoding followed by successful host-backed
  overview, including progress while a legacy stream is live.
- Every selected code with integer/string/null/omitted ID, fixed message, omitted
  data and rebuilt headers.
- Byte/header preservation across excluded responses, including SSE, notifications,
  admission-owned errors, oversized/malformed JSON and unselected codes.

Reintroducing the blanket HTTP wrapper in each order fails the official-client
protocol test: the modern correlated error can no longer be decoded. Restoring
policy passes. No SDK upgrade, dependency/lock changes, desktop route changes or
CLI MCP activation are part of this increment.

Validation commands:

```sh
cargo test --manifest-path spikes/mcp-sdk/Cargo.toml --locked
cargo clippy --manifest-path spikes/mcp-sdk/Cargo.toml --locked --all-targets -- -D warnings
cargo fmt --manifest-path spikes/mcp-sdk/Cargo.toml -- --check
```

Next work is production HTTP host integration with independently reviewed
admission/authentication, Origin/session ownership and logging policy. The response
contract above is ready for that integration within its explicit limits. CLI
operation execution, serialized import/recovery and native cleanup remain separate.
