use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn vima_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vima"))
}

fn setup_store() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let vima_dir = tmp.path().join(".vima");
    let tickets_dir = vima_dir.join("tickets");
    fs::create_dir_all(&tickets_dir).unwrap();
    fs::write(vima_dir.join("config.yml"), "prefix: pl\n").unwrap();
    tmp
}

fn vima_cmd(tmp: &tempfile::TempDir) -> Command {
    let mut cmd = vima_bin();
    cmd.env("VIMA_DIR", tmp.path().join(".vima"));
    cmd
}

// ── Unknown command produces structured JSON error ──────────────────────

#[test]
fn unknown_command_returns_structured_error() {
    let tmp = setup_store();

    let output = vima_cmd(&tmp)
        .args(["nonexistent-command-xyz"])
        .output()
        .unwrap();

    assert!(!output.status.success(), "unknown command should fail");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let err: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(err["error"], "invalid_field");
    assert!(
        err["message"].as_str().unwrap().contains("unknown command"),
        "error should mention unknown command"
    );
}

// ── Plugin found on PATH gets executed ──────────────────────────────────

#[test]
fn plugin_on_path_gets_executed() {
    let tmp = setup_store();
    let plugin_dir = tempfile::tempdir().unwrap();

    // Create a plugin script that writes a marker file
    let marker = tmp.path().join("plugin-ran");
    let plugin_path = plugin_dir.path().join("vima-testplug");
    fs::write(
        &plugin_path,
        format!("#!/bin/sh\ntouch {}\n", marker.display()),
    )
    .unwrap();
    let mut perms = fs::metadata(&plugin_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&plugin_path, perms).unwrap();

    // Add plugin dir to PATH
    let original_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", plugin_dir.path().display(), original_path);

    let output = vima_cmd(&tmp)
        .env("PATH", &new_path)
        .args(["testplug"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "plugin should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        marker.exists(),
        "plugin should have executed and created marker file"
    );
}

// ── Plugin receives VIMA_DIR env var ────────────────────────────────────

#[test]
fn plugin_receives_vima_dir_env() {
    let tmp = setup_store();
    let plugin_dir = tempfile::tempdir().unwrap();

    // Create a plugin that writes VIMA_DIR to a file
    let output_file = tmp.path().join("vima-dir-output");
    let plugin_path = plugin_dir.path().join("vima-envcheck");
    fs::write(
        &plugin_path,
        format!(
            "#!/bin/sh\necho \"$VIMA_DIR\" > {}\n",
            output_file.display()
        ),
    )
    .unwrap();
    let mut perms = fs::metadata(&plugin_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&plugin_path, perms).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", plugin_dir.path().display(), original_path);

    let output = vima_cmd(&tmp)
        .env("PATH", &new_path)
        .args(["envcheck"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "plugin should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let vima_dir_value = fs::read_to_string(&output_file)
        .expect("plugin should have written VIMA_DIR")
        .trim()
        .to_string();
    assert!(
        !vima_dir_value.is_empty(),
        "VIMA_DIR should be set for plugins"
    );
    assert!(
        vima_dir_value.contains(".vima"),
        "VIMA_DIR should point to .vima directory, got: {}",
        vima_dir_value
    );
}

// ── Plugin receives arguments ───────────────────────────────────────────

#[test]
fn plugin_receives_arguments() {
    let tmp = setup_store();
    let plugin_dir = tempfile::tempdir().unwrap();

    let output_file = tmp.path().join("args-output");
    let plugin_path = plugin_dir.path().join("vima-argtest");
    fs::write(
        &plugin_path,
        format!("#!/bin/sh\necho \"$@\" > {}\n", output_file.display()),
    )
    .unwrap();
    let mut perms = fs::metadata(&plugin_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&plugin_path, perms).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", plugin_dir.path().display(), original_path);

    let output = vima_cmd(&tmp)
        .env("PATH", &new_path)
        .args(["argtest", "hello", "world"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let args_value = fs::read_to_string(&output_file)
        .expect("plugin should have written args")
        .trim()
        .to_string();
    assert_eq!(args_value, "hello world");
}

// ── Plugin failure produces structured JSON error ───────────────────────

#[test]
fn plugin_not_executable_returns_structured_error() {
    let tmp = setup_store();

    // Without any plugin on PATH, unknown command should error
    let output = vima_cmd(&tmp)
        .env("PATH", "") // empty PATH so no plugins found
        .args(["someplugin"])
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let err: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert!(
        err["error"].as_str().is_some(),
        "error should have an error code"
    );
}
