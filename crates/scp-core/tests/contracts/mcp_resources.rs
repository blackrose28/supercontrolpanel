use super::{list_mcp_resources, resolve_mcp_resource};

#[test]
fn resource_discovery_preserves_exact_desktop_wire_contract() {
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("mcp_resources.json")).unwrap();
    let serialized: Vec<_> = list_mcp_resources()
        .into_iter()
        .map(|resource| serde_json::to_value(resource).unwrap())
        .collect();
    assert_eq!(
        serialized.as_slice(),
        expected.as_array().unwrap().as_slice()
    );
    assert_eq!(serialized.len(), 5);
}

#[test]
fn resource_descriptors_are_owned_and_cloneable() {
    let mut resources = list_mcp_resources();
    let original = resources[0].clone();
    resources[0].uri.clear();
    resources[0].title.clear();
    resources[0].category.clear();
    let fresh = list_mcp_resources().remove(0);
    assert_eq!(fresh.uri, original.uri);
    assert_eq!(fresh.title, original.title);
    assert_eq!(fresh.category, original.category);
}

#[test]
fn resource_resolution_preserves_all_five_exact_tool_mappings() {
    let expected = [
        ("scp://status/functions", "function_status_list"),
        ("scp://status/openvpn", "openvpn_get_status"),
        ("scp://status/boundary", "boundary_get_status"),
        ("scp://servers/list", "servers_list"),
        ("scp://config/paths", "config_get_paths"),
    ];
    for ((uri, tool), descriptor) in expected.into_iter().zip(list_mcp_resources()) {
        assert_eq!(descriptor.uri, uri);
        assert_eq!(resolve_mcp_resource(uri), Some(tool));
    }
}

#[test]
fn unknown_resource_uris_are_not_normalized_or_interpreted() {
    for uri in [
        "",
        "scp://status",
        "SCP://status/functions",
        "scp://status/functions/",
        " scp://status/functions",
        "scp://status/functions\n",
        "scp://servers/list?x=1",
        "scp://config/paths#fragment",
        "function_status_list",
        "scp://config/file",
    ] {
        assert_eq!(resolve_mcp_resource(uri), None, "{uri:?}");
    }
}
