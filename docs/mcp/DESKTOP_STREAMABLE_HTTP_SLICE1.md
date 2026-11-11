# Desktop HTTP MCP Slice 1: policy and safe contracts

Slice 1 implements the shared policy foundation on the existing legacy REST and Tauri MCP paths. It does **not** enable standard `/mcp`, change SDK dependencies, or claim native Windows/macOS qualification.

## Boundary

`contracts.rs` owns the 40-operation registry, exhaustive typed host-handler identifiers, categories, sensitivity (`!read_only`), capability gates, lifetime metadata, five resource mappings, application errors, and audit/redaction rules. `validation.rs` defines the input/output schema subset and validates it. `public_dto.rs` contains dedicated public config, settings, function-status, and transfer outputs; `desktop_dto.rs` constructs these field by field from runtime models. These contract/schema/DTO modules have no Tauri/HTTP imports. The desktop dispatcher remains the runtime owner.

Both REST and Tauri MCP tool calls authorize and validate before host dispatch. Resources resolve through the same registry and tool call. Discovery filters calls that policy or capability would deny. Local desktop configuration/enrollment commands retain their authority; MCP commands do not acquire that authority by sharing an action.

## Deliberate compatibility corrections

- Tool names, resource URIs, legacy descriptor fields, and `ok/message/data/warnings/timestamp` envelopes remain. Application errors use stable uppercase code prefixes in the legacy `message`; the internal error DTO also defines `code`, `message`, and `retryable: false`. Arbitrary host errors become `OPERATION_FAILED`. Clients should query state before retrying mutations.
- Only `require-ui-approval` and `allow-with-token` are recognized. Unknown policies/categories and missing operation metadata fail closed. Explicit empty categories mean no permissions; omitted settings retain serde defaults. All launches, stops, writes, and browser open/close are sensitive. Browser list/stat are reads in the `file-transfer` category. There is no approval queue: use local desktop controls or deliberately change local policy.
- Inputs must be objects. Unknown fields, wrong types, null patches, empty required IDs, strings over configured limits, NULs, oversized service arrays, and dimensions outside 1–65535 are rejected. The helper's previously ignored `hidden` argument is now rejected, rather than silently accepted; automated OIDC selection remains a later reviewed operation change.
- MCP local-settings patches accept only `repoRoot`; MCP enable/bind/port/token/categories/policy, CC/enrollment fields, executable/profile paths, and Caddy folder remain local. The whole patch is rejected before writing if it contains a forbidden field. This path saves under the existing writer/recovery gate without restarting the listener or CC connection.
- App-config patches reject unknown fields and local launch fields (`openvpn.command/executablePath/ovpnPath/extraArgs`, `boundary.cliPath/connectSshOptions`). Full replacements may retain those fields only when their parsed values equal the current configuration, including defaults applied by deserialization. Omission cannot erase a nonempty protected value. Credential writes still go through existing keyring-backed save actions.
- `config_sync_pull`, `config_git_pull`, and `config_git_push` fail with `CAPABILITY_UNAVAILABLE` and are hidden from discovery. The existing direct-copy importer and git hooks/config have no protected staging boundary. Blocking these paths is intentional; staged/validated imports and reviewed git execution are required before re-enabling them. Local desktop actions remain available. Shared-config push remains sensitive and uses the existing secret-free disk representation.
- Config outputs omit credential fields and launch command/options/path fields. Settings outputs omit MCP/CC tokens. Status messages and transfer title/detail/source/destination are suppressed; IDs, states, timestamps, progress/session association, and Boundary App/CLI source remain. Direct SCP result text is replaced with a static acknowledgement. These are deliberate reductions in remote diagnostics; inspect the desktop for runtime details.
- Public results are schema-checked before return. Recursive secret-key redaction is additional protection, not the public DTO boundary. The documented `boundaryTokenSource` provenance is retained. HTTP diagnostics redact listener errors; error envelopes never echo raw parser/runtime text.
- HTTP audit records contain only fixed call kind and a registry-resolved operation name. No argument values, unrecognized names, terminal output, credentials, or raw errors are logged by this audit path.

## TDD evidence

Focused command: `cargo test --manifest-path src-tauri/Cargo.toml --bin super-control-panel mcp`.

Observed RED → GREEN gates:

| Gate | Expected RED | GREEN coverage |
| --- | --- | --- |
| Sensitivity and settings credentials | Launch/mutation lacked approval; `ccAgentToken` leaked | `tools::slice1_tests` |
| Public allowlists | A future secret field survived masking | Explicit public DTO allowlists and schemas |
| Policy/validation foundation | New contract API absent | Invalid policy/category, unknown operation, denied resource, malformed/protected patch and unavailable import/git calls never invoke the supplied host |
| Full replacement | Protected launch field changed | Comparison rejects changes and erasure for every protected key |
| Legacy HTTP | Raw keyring diagnostic appeared in socket response | Safe error envelope with original legacy fields |
| Settings defaults | Explicit empty categories became default permissions | Empty policy/categories preserved; omitted fields default |
| Output schemas | Public fixture did not conform to unimplemented schema | Safe DTO fixtures conform; injected credential fields fail |

Final validation on Linux:

- `cargo test --manifest-path src-tauri/Cargo.toml`: 112 desktop tests passed; helper test target compiled (0 tests).
- `npm run build`: passed.
- `npx playwright test`: 36 passed, 15 existing skips. Firefox/WebKit and their host dependencies were installed after the first run could not launch those browsers.
- `git diff --check`: passed.
- Ripwire quality delta: no unacknowledged gating regressions. Four narrowly recorded exceptions in `.ripwire_quality_acks` retain explicit public DTO projections and independent settings/inventory decoding. The remaining nongating graph findings include Rust trait/test entry points the index does not resolve.
- Ripwire's test graph gate still reports an untested transitive radius (170 symbols at review); it does not discover the inline Rust tests. The executed Rust/Playwright results are the verification evidence, not a claim that this graph gate is clean.

These checks do not substitute for native runtime/keyring/elevation tests or integrated standard MCP client tests.

## Following slices

Keep standard transport, session/cancellation ownership, revocation/queued authorization, bounded execution, UI/client setup, and native client qualification in their planned slices. Raw-store status filtering and per-family runtime acceptance tests remain part of vertical/family migration. Internal schemas describe the current legacy `data` payloads, including arrays/null; the standard adapter must deliberately wrap those into structured object results. Do not advertise this REST service as standard MCP.
