use std::fs;
use std::process::Command;

fn vima_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vima"))
}

/// Create a temporary .vima store and return the temp dir (kept alive by caller).
fn setup_store() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let vima_dir = tmp.path().join(".vima");
    let tickets_dir = vima_dir.join("tickets");
    fs::create_dir_all(&tickets_dir).unwrap();
    fs::write(vima_dir.join("config.yml"), "prefix: ci\n").unwrap();
    tmp
}

fn vima_cmd(tmp: &tempfile::TempDir) -> Command {
    let mut cmd = vima_bin();
    cmd.env("VIMA_DIR", tmp.path().join(".vima"));
    cmd.env("VIMA_EXACT", "true");
    cmd
}

/// Helper: create a ticket and return its ID.
fn create_ticket(tmp: &tempfile::TempDir, title: &str) -> String {
    let output = vima_cmd(tmp)
        .args(["create", title])
        .output()
        .expect("failed to run vima create");
    assert!(
        output.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("create stdout not JSON");
    json["id"].as_str().unwrap().to_string()
}

/// Helper: show a ticket and return the parsed JSON.
fn show_ticket(tmp: &tempfile::TempDir, id: &str) -> serde_json::Value {
    let output = vima_cmd(tmp)
        .args(["show", id])
        .output()
        .expect("failed to run vima show");
    assert!(
        output.status.success(),
        "show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("show stdout not JSON")
}

// NOTE: Stale version (exit code 5) is not tested at the CLI level.
// Each CLI invocation does read->modify->write atomically in one process, so
// exit code 5 is only reachable via a true filesystem race between two processes.
// The unit tests in store.rs (write_ticket_stale_version_returns_error) cover
// the stale detection logic deterministically.

// ── Already claimed (exit code 6) ───────────────────────────────────────

#[test]
fn start_different_assignee_returns_exit_6_with_json_error() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "Claim collision test");

    // Agent A claims the ticket
    let output_a = vima_cmd(&tmp)
        .args(["start", &id, "--assignee", "agent-alpha"])
        .output()
        .unwrap();
    assert!(
        output_a.status.success(),
        "Agent A start should succeed: {}",
        String::from_utf8_lossy(&output_a.stderr)
    );

    // Agent B tries to claim the same ticket
    let output_b = vima_cmd(&tmp)
        .args(["start", &id, "--assignee", "agent-beta"])
        .output()
        .unwrap();

    let exit_code = output_b.status.code().unwrap();
    assert_eq!(
        exit_code, 6,
        "claim collision should exit with code 6, got {exit_code}"
    );

    // Verify stderr is valid JSON with the right structure
    let stderr = String::from_utf8_lossy(&output_b.stderr);
    let err_json: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be valid JSON");

    assert_eq!(
        err_json["error"], "already_claimed",
        "error code should be 'already_claimed'"
    );
    assert!(
        err_json["message"]
            .as_str()
            .unwrap()
            .contains("already claimed"),
        "message should mention already claimed"
    );
    assert_eq!(
        err_json["current_assignee"].as_str().unwrap(),
        "agent-alpha",
        "error should report who holds the ticket"
    );
    assert_eq!(
        err_json["id"].as_str().unwrap(),
        id,
        "error should include the ticket id"
    );
    assert!(
        err_json["suggestion"].as_str().is_some(),
        "error should include a recovery suggestion"
    );
}

#[test]
fn start_different_assignee_emits_nothing_on_stdout() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "Claim stdout test");

    vima_cmd(&tmp)
        .args(["start", &id, "--assignee", "agent-alpha"])
        .output()
        .unwrap();

    let output = vima_cmd(&tmp)
        .args(["start", &id, "--assignee", "agent-beta"])
        .output()
        .unwrap();

    assert_eq!(output.status.code().unwrap(), 6);
    assert!(
        output.stdout.is_empty(),
        "stdout must be empty on error -- agents parse stdout as success data"
    );
}

#[test]
fn start_same_assignee_is_idempotent() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "Idempotent claim test");

    // First claim
    let output1 = vima_cmd(&tmp)
        .args(["start", &id, "--assignee", "agent-alpha"])
        .output()
        .unwrap();
    assert!(output1.status.success());

    // Same agent claims again -- must succeed (idempotent)
    let output2 = vima_cmd(&tmp)
        .args(["start", &id, "--assignee", "agent-alpha"])
        .output()
        .unwrap();
    assert!(
        output2.status.success(),
        "same assignee re-start should be idempotent, got exit {}: {}",
        output2.status.code().unwrap(),
        String::from_utf8_lossy(&output2.stderr)
    );

    // Should produce valid JSON on stdout with correct data
    let json: serde_json::Value =
        serde_json::from_slice(&output2.stdout).expect("idempotent start should emit JSON");
    assert_eq!(json["id"].as_str().unwrap(), id);
    assert_eq!(json["assignee"].as_str().unwrap(), "agent-alpha");
    assert_eq!(json["status"].as_str().unwrap(), "in_progress");
}

#[test]
fn start_no_assignee_on_claimed_ticket_returns_exit_6() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "No assignee on claimed test");

    // Agent A claims it
    vima_cmd(&tmp)
        .args(["start", &id, "--assignee", "agent-alpha"])
        .output()
        .unwrap();

    // Bare start without assignee on a claimed ticket
    let output = vima_cmd(&tmp)
        .args(["start", &id])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        6,
        "starting a claimed ticket without assignee should fail with exit 6"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    let err_json: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be valid JSON");
    assert_eq!(err_json["error"], "already_claimed");
    assert_eq!(
        err_json["current_assignee"].as_str().unwrap(),
        "agent-alpha"
    );
}

// ── Version advancement on successful writes ────────────────────────────

#[test]
fn successful_update_advances_version() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "Version advance test");

    let v1 = show_ticket(&tmp, &id)["version"]
        .as_str()
        .unwrap()
        .to_string();

    let update_output = vima_cmd(&tmp)
        .args(["update", &id, "--priority", "4"])
        .output()
        .unwrap();
    assert!(
        update_output.status.success(),
        "update should succeed: {}",
        String::from_utf8_lossy(&update_output.stderr)
    );

    let v2 = show_ticket(&tmp, &id)["version"]
        .as_str()
        .unwrap()
        .to_string();

    assert_ne!(v1, v2, "version should advance after content change");
}

#[test]
fn show_includes_version_field() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "Version in show test");

    let ticket = show_ticket(&tmp, &id);
    let version = ticket["version"].as_str().unwrap();

    assert_eq!(version.len(), 16, "version should be 16 hex chars");
    assert!(
        version.chars().all(|c| c.is_ascii_hexdigit()),
        "version should be hex"
    );
}

// ── Not found (exit code 3) -- contract test ────────────────────────────

#[test]
fn update_nonexistent_ticket_returns_exit_3_with_json_error() {
    let tmp = setup_store();

    let output = vima_cmd(&tmp)
        .args(["update", "ci-9999", "--priority", "4"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        3,
        "not found should exit with code 3"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    let err_json: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be valid JSON");
    assert_eq!(err_json["error"], "not_found");
    assert!(
        err_json["suggestion"]
            .as_str()
            .unwrap()
            .contains("vima list")
    );
}

#[test]
fn start_nonexistent_ticket_returns_exit_3() {
    let tmp = setup_store();

    let output = vima_cmd(&tmp)
        .args(["start", "ci-9999"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        3,
        "not found should exit with code 3"
    );
}

// ── Error contract: all errors are JSON on stderr ───────────────────────

#[test]
fn all_error_responses_have_error_message_suggestion_fields() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "Error contract test");

    // Claim for exit 6
    vima_cmd(&tmp)
        .args(["start", &id, "--assignee", "owner"])
        .output()
        .unwrap();

    let error_cases: Vec<(&str, Vec<&str>)> = vec![
        ("not_found", vec!["show", "ci-9999"]),
        (
            "already_claimed",
            vec!["start", &id, "--assignee", "intruder"],
        ),
    ];

    for (desc, args) in &error_cases {
        let output = vima_cmd(&tmp).args(args.clone()).output().unwrap();
        assert!(!output.status.success(), "{desc}: should have failed");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let err_json: serde_json::Value = serde_json::from_str(stderr.trim())
            .unwrap_or_else(|e| panic!("{desc}: stderr is not valid JSON: {e}\nstderr: {stderr}"));

        assert!(
            err_json["error"].is_string(),
            "{desc}: missing 'error' field"
        );
        assert!(
            err_json["message"].is_string(),
            "{desc}: missing 'message' field"
        );
        assert!(
            err_json["suggestion"].is_string(),
            "{desc}: missing 'suggestion' field"
        );
    }
}
