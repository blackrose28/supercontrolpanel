use super::list_mcp_tools;
use std::collections::HashSet;

#[test]
fn discovery_preserves_exact_desktop_wire_contract() {
    let expected: serde_json::Value = serde_json::from_str(include_str!("mcp_tools.json")).unwrap();
    let tools = list_mcp_tools();
    assert_eq!(tools.len(), 40);
    assert_eq!(serde_json::to_value(tools.clone()).unwrap(), expected);
    let names: HashSet<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names.len(), tools.len(), "tool names must be unique");
}

#[test]
fn discovery_returns_independent_owned_descriptors() {
    let mut tools = list_mcp_tools();
    let original = tools[0].clone();
    tools[0].name.clear();
    tools[0].title.clear();
    tools[0].category.clear();
    tools[0].read_only = false;
    assert_eq!(
        serde_json::to_value(&list_mcp_tools()[0]).unwrap(),
        serde_json::to_value(original).unwrap()
    );
}
