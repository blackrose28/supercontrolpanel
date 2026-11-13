//! Compile/run the published SDK server alongside the real desktop dependency graph.
//! No listener, app state, credentials or production startup wiring is created.
#[test]
fn published_sdk_router_coexists_with_desktop_dependencies() {
    let _router = scp_mcp_sdk_spike::fixture_router(Default::default());
}
