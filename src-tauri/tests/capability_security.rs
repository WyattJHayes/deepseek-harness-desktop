use serde_json::Value;
use std::path::Path;

fn read_json(relative_path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

#[test]
fn harness_remote_capability_has_no_direct_ipc_permissions() {
    let capability = read_json("capabilities/default.json");

    assert_eq!(capability["identifier"], "harness-remote");
    assert_eq!(capability["local"], false);
    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    assert_eq!(
        capability["remote"]["urls"],
        serde_json::json!(["http://127.0.0.1:*"])
    );
    assert_eq!(capability["permissions"], serde_json::json!([]));
}

#[test]
fn desktop_capability_keeps_privileged_permissions_local_only() {
    let capability = read_json("capabilities/desktop.json");
    let permissions = capability["permissions"]
        .as_array()
        .expect("desktop capability permissions must be an array");

    assert_eq!(capability["local"], true);
    assert!(capability.get("remote").is_none());
    assert!(permissions.contains(&serde_json::json!("core:default")));
    assert!(permissions.contains(&serde_json::json!("opener:default")));
    assert!(permissions.contains(&serde_json::json!("store:default")));
    assert!(permissions.contains(&serde_json::json!("clipboard-manager:allow-write-text")));
}

#[test]
fn csp_allows_the_dynamic_loopback_harness_frame_only() {
    let config = read_json("tauri.conf.json");
    let csp = config["app"]["security"]["csp"]
        .as_str()
        .expect("production CSP must be a string");
    let dev_csp = config["app"]["security"]["devCsp"]
        .as_str()
        .expect("development CSP must be a string");

    assert!(csp.contains("frame-src http://127.0.0.1:*"));
    assert!(dev_csp.contains("frame-src http://127.0.0.1:*"));
}
