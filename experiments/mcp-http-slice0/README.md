# Desktop HTTP MCP Slice 0 fixture

Disposable protocol feasibility fixture, separate from the desktop application.
It binds an ephemeral IPv4 loopback port, prints its `/mcp` URL, and exposes only
`fixture_echo`, `fixture_slow`, `fixture://status`, and a test-only
`fixture://active` counter. It imports no application code and reads no desktop
settings/keyring. `slice0-fixture-only` is a public test credential.

From the repository root:

```sh
cargo build --locked --manifest-path experiments/mcp-http-slice0/Cargo.toml
npm ci --prefix experiments/mcp-http-slice0 --ignore-scripts --no-audit --no-fund
npm test --prefix experiments/mcp-http-slice0
```

The tests launch/stop their own fixture processes. Requirements: Rust 1.99.1 and
Node 22.23.4 were tested on Linux; see the report for declared dependency MSRVs.
`--ignore-scripts` installs the pinned Inspector without invoking its install
scripts; its CLI works in this configuration. No browser/OAuth setup is needed.
The Cargo and npm lockfiles are retained here; production manifests/locks are
not changed. Generated targets, dependencies, and local evidence are ignored.

For manual client inspection, run the binary with `json` or `sse` as its first
argument, copy its printed URL, then run the pinned Inspector:

```sh
node experiments/mcp-http-slice0/node_modules/@modelcontextprotocol/inspector/clients/launcher/build/index.js --cli --server-url http://127.0.0.1:PORT/mcp --transport http --protocol-era modern --header 'Authorization: Bearer slice0-fixture-only' --method tools/list --format json --quiet
```

Replace `PORT` with the actual port. Use `--protocol-era legacy` for the legacy
handshake. Stop the manual fixture process when finished. The `json` option is a
response preference: modern calls return JSON, while legacy session calls use
SSE even under that preference. `sse` exercises SSE in both eras. Legacy sessions
remain enabled because disabling them failed the server-work cancellation test.
This is not a production server or a policy implementation.

Tests also verify inventory coverage against current source registrations. The
fixture's synthetic denial is only a protocol `isError` check; it does not verify
the application's actual authorization policy or any real mutation.

See [the Slice 0 report](../../docs/mcp/DESKTOP_STREAMABLE_HTTP_SLICE0.md)
and [recorded test output](../../docs/mcp/evidence/SLICE0_COMPAT.tap).
