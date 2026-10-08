use std::{path::Path, process::Command};

fn load(file: Option<&Path>) -> serde_json::Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_location-info"));
    command.env_remove("XMU_LOCATION_FILE");
    if let Some(file) = file {
        command.env("XMU_LOCATION_FILE", file);
    }
    let output = command.output().expect("location loader runs");
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).expect("loader prints JSON")
}

#[test]
fn embedded_catalog_is_available_without_configuration() {
    let value = load(None);
    assert_eq!(value["count"], 474);
    assert_eq!(value["external_file_loaded"], false);
}

#[test]
fn external_file_loading_and_invalid_file_fallback() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let file = std::env::temp_dir().join(format!(
        "xmu-location-test-{}-{nonce}.json",
        std::process::id()
    ));
    assert!(!file.exists());
    assert_eq!(load(Some(&file))["external_file_loaded"], false);
    let catalog = serde_json::json!({"schema_version":1,"coordinate_system":"xmu-map-native","locations":[
        {"region":"SiMing","name":"测试楼","longitude":118.10,"latitude":24.44}
    ]});
    std::fs::write(&file, catalog.to_string()).unwrap();
    let loaded = load(Some(&file));
    assert_eq!(loaded["count"], 1);
    assert_eq!(loaded["external_file_loaded"], true);
    for invalid in [
        "not JSON".to_owned(),
        catalog.to_string().replace("118.1", "24.44"),
        " ".repeat(1024 * 1024 + 1),
    ] {
        std::fs::write(&file, invalid).unwrap();
        let fallback = load(Some(&file));
        assert_eq!(fallback["count"], 474);
        assert_eq!(fallback["external_file_loaded"], false);
    }
    std::fs::remove_file(file).unwrap();
}
