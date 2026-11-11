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
