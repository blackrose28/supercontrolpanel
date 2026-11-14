use scp_mcp::contracts::OPERATIONS;

#[test]
fn feasibility_ledger_covers_every_current_operation_and_resource() {
    let ledger = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/mcp/STANDALONE_CLI_OPERATION_LEDGER.md"
    ))
    .expect("feasibility ledger must exist");
    for op in OPERATIONS {
        assert!(
            ledger.contains(&format!("`{}`", op.name)),
            "missing operation {}",
            op.name
        );
        if let Some((uri, _, _)) = op.resource {
            assert!(
                ledger.contains(&format!("`{uri}`")),
                "missing resource {uri}"
            );
        }
    }
}

#[test]
fn feasibility_ledger_covers_local_cli_workflows_and_resolved_linux_spikes() {
    let ledger = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/mcp/STANDALONE_CLI_OPERATION_LEDGER.md"
    ))
    .expect("feasibility ledger must exist");
    for entry in [
        "`scpctl schema`",
        "`scpctl capabilities`",
        "`scpctl settings export`",
        "`scpctl settings import --preview`",
        "`scpctl settings import --apply --interactive`",
        "`scpctl mcp serve --stdio`",
    ] {
        assert!(ledger.contains(entry), "missing local CLI entry {entry}");
    }
    for stale in [
        "Not run on any OS",
        "Transport not added",
        "Existing CLI state sharing not characterized",
    ] {
        assert!(
            !ledger.contains(stale),
            "stale Slice 0 blocker remains in ledger: {stale}"
        );
    }
}
