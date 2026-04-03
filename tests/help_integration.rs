use std::process::Command;

fn vima_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vima"))
}

/// Regression test: clap special-cases a subcommand named "help" and
/// intercepts `help <command>` before our HelpArgs handler sees it.
/// We pre-process args in main() to work around this.  These tests
/// invoke the real binary so the full main() path is exercised.

#[test]
fn help_create_json_returns_create_schema() {
    let output = vima_bin()
        .args(["help", "create", "--json"])
        .output()
        .expect("failed to run vima");

    assert!(output.status.success(), "exit code was {:?}", output.status);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is not valid JSON");

    assert_eq!(
        json["name"], "create",
        "should return the create command schema"
    );
    assert!(json["args"].is_array(), "create schema should have args");
}

#[test]
fn help_json_returns_full_schema() {
    let output = vima_bin()
        .args(["help", "--json"])
        .output()
        .expect("failed to run vima");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is not valid JSON");

    assert_eq!(json["name"], "vima");
    assert!(json["commands"].is_array());
}

#[test]
fn help_brief_returns_compact_index() {
    let output = vima_bin()
        .args(["help", "--brief"])
        .output()
        .expect("failed to run vima");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("stdout is not valid JSON array");

    assert!(!json.is_empty());
    // Every entry should have name + about, nothing else
    for entry in &json {
        assert!(entry["name"].is_string());
    }
}

#[test]
fn help_unknown_command_json_exits_nonzero() {
    let output = vima_bin()
        .args(["help", "nonexistent", "--json"])
        .output()
        .expect("failed to run vima");

    assert!(!output.status.success());
}
