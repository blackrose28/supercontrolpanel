use super::McpConfigGetPathsResult;
use serde_json::json;

#[test]
fn config_paths_preserves_the_single_field_without_path_normalization() {
    for path in [
        "",
        "/home/user/.config/supercontrolpanel/config.json",
        r"C:\Users\User Name\AppData\Roaming\supercontrolpanel\config.json",
        "/Users/user/Library/Application Support/supercontrolpanel/config.json",
        "  relative/../雪\n\u{1b}\0/�  ",
    ] {
        let result = McpConfigGetPathsResult {
            app_config_path: path.to_owned(),
        };
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({"appConfigPath": path})
        );
    }
}
