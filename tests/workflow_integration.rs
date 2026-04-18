use std::fs;
use std::process::Command;

fn vima_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vima"))
}

fn setup_store() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let vima_dir = tmp.path().join(".vima");
    let tickets_dir = vima_dir.join("tickets");
    fs::create_dir_all(&tickets_dir).unwrap();
    fs::write(vima_dir.join("config.yml"), "prefix: wf\n").unwrap();
    tmp
}

fn vima_cmd(tmp: &tempfile::TempDir) -> Command {
    let mut cmd = vima_bin();
    cmd.env("VIMA_DIR", tmp.path().join(".vima"));
    cmd.env("VIMA_EXACT", "true");
    cmd
}

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

fn run_ok(cmd: &mut Command) -> serde_json::Value {
    let output = cmd.output().expect("failed to run command");
    assert!(
        output.status.success(),
        "command failed (exit {}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or(serde_json::Value::Null)
}

// ── Full lifecycle: create → show → start → close → reopen ──────────────

#[test]
fn full_lifecycle_create_show_start_close_reopen() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "Lifecycle test");

    // Show returns the ticket
    let json = run_ok(vima_cmd(&tmp).args(["show", &id]));
    assert_eq!(json["title"], "Lifecycle test");
    assert_eq!(json["status"], "open");

    // Start transitions to in_progress
    run_ok(vima_cmd(&tmp).args(["start", &id]));
    let json = run_ok(vima_cmd(&tmp).args(["show", &id]));
    assert_eq!(json["status"], "in_progress");

    // Close transitions to closed
    run_ok(vima_cmd(&tmp).args(["close", &id]));
    let json = run_ok(vima_cmd(&tmp).args(["show", &id]));
    assert_eq!(json["status"], "closed");

    // Reopen transitions back to open
    run_ok(vima_cmd(&tmp).args(["reopen", &id]));
    let json = run_ok(vima_cmd(&tmp).args(["show", &id]));
    assert_eq!(json["status"], "open");
}

// ── List filters correctly ──────────────────────────────────────────────

#[test]
fn list_filters_by_status() {
    let tmp = setup_store();
    let id1 = create_ticket(&tmp, "Open ticket");
    let id2 = create_ticket(&tmp, "Closed ticket");
    run_ok(vima_cmd(&tmp).args(["close", &id2]));

    // List with --status open should only include id1
    let json = run_ok(vima_cmd(&tmp).args(["list", "--status", "open"]));
    let arr = json.as_array().unwrap();
    let ids: Vec<&str> = arr.iter().map(|v| v["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&id1.as_str()));
    assert!(!ids.contains(&id2.as_str()));
}

// ── Dependency chain: add deps → check blocked/ready ────────────────────

#[test]
fn dependency_chain_blocked_and_ready() {
    let tmp = setup_store();
    let a = create_ticket(&tmp, "Task A");
    let b = create_ticket(&tmp, "Task B depends on A");

    // Add dep: B depends on A
    run_ok(vima_cmd(&tmp).args(["dep", "add", &b, &a]));

    // B should be blocked (is-ready exits non-zero)
    let output = vima_cmd(&tmp).args(["is-ready", &b]).output().unwrap();
    assert_ne!(output.status.code().unwrap(), 0, "B should be blocked");

    // A should be ready
    let output = vima_cmd(&tmp).args(["is-ready", &a]).output().unwrap();
    assert_eq!(output.status.code().unwrap(), 0, "A should be ready");

    // Close A, now B should be ready
    run_ok(vima_cmd(&tmp).args(["close", &a]));
    let output = vima_cmd(&tmp).args(["is-ready", &b]).output().unwrap();
    assert_eq!(
        output.status.code().unwrap(),
        0,
        "B should be ready after A closed"
    );
}

// ── Cycle detection ─────────────────────────────────────────────────────

#[test]
fn dep_cycle_rejected_with_exit_2() {
    let tmp = setup_store();
    let a = create_ticket(&tmp, "Cycle A");
    let b = create_ticket(&tmp, "Cycle B");

    run_ok(vima_cmd(&tmp).args(["dep", "add", &a, &b]));
    // Adding B → A should create a cycle
    let output = vima_cmd(&tmp)
        .args(["dep", "add", &b, &a])
        .output()
        .unwrap();
    assert_eq!(output.status.code().unwrap(), 2, "cycle should exit 2");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let err: serde_json::Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(err["error"], "cycle");
}

// ── Dep tree output ─────────────────────────────────────────────────────

#[test]
fn dep_tree_shows_transitive_deps() {
    let tmp = setup_store();
    let a = create_ticket(&tmp, "Root");
    let b = create_ticket(&tmp, "Mid");
    let c = create_ticket(&tmp, "Leaf");

    run_ok(vima_cmd(&tmp).args(["dep", "add", &a, &b]));
    run_ok(vima_cmd(&tmp).args(["dep", "add", &b, &c]));

    let json = run_ok(vima_cmd(&tmp).args(["dep", "tree", &a]));
    let tree_str = serde_json::to_string(&json).unwrap();
    // The tree should contain all three ticket IDs
    assert!(tree_str.contains(&a), "tree should contain root");
    assert!(tree_str.contains(&b), "tree should contain mid");
    assert!(tree_str.contains(&c), "tree should contain leaf");
}

// ── Link/unlink workflow ────────────────────────────────────────────────

#[test]
fn link_and_unlink_workflow() {
    let tmp = setup_store();
    let a = create_ticket(&tmp, "Link A");
    let b = create_ticket(&tmp, "Link B");

    run_ok(vima_cmd(&tmp).args(["link", &a, &b]));

    // Both should show the link
    let json_a = run_ok(vima_cmd(&tmp).args(["show", &a]));
    let json_b = run_ok(vima_cmd(&tmp).args(["show", &b]));
    assert!(json_a["links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == &id_str(&b)));
    assert!(json_b["links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == &id_str(&a)));

    // Unlink
    run_ok(vima_cmd(&tmp).args(["unlink", &a, &b]));
    let json_a = run_ok(vima_cmd(&tmp).args(["show", &a]));
    assert!(json_a["links"].as_array().unwrap().is_empty());
}

fn id_str(id: &str) -> serde_json::Value {
    serde_json::Value::String(id.to_string())
}

// ── Add note workflow ───────────────────────────────────────────────────

#[test]
fn add_note_appears_in_show() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "Note test");

    run_ok(vima_cmd(&tmp).args(["add-note", &id, "First note"]));
    run_ok(vima_cmd(&tmp).args(["add-note", &id, "Second note"]));

    let json = run_ok(vima_cmd(&tmp).args(["show", &id]));
    let notes = json["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0]["text"], "First note");
    assert_eq!(notes[1]["text"], "Second note");
}

// ── Update workflow ─────────────────────────────────────────────────────

#[test]
fn update_modifies_fields() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "Update test");

    run_ok(vima_cmd(&tmp).args([
        "update",
        &id,
        "--title",
        "Updated title",
        "--priority",
        "3",
        "--tags",
        "alpha,beta",
    ]));

    let json = run_ok(vima_cmd(&tmp).args(["show", &id]));
    assert_eq!(json["title"], "Updated title");
    assert_eq!(json["priority"], 3);
    let tags: Vec<&str> = json["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(tags.contains(&"alpha"));
    assert!(tags.contains(&"beta"));
}

// ── Batch create workflow ───────────────────────────────────────────────

#[test]
fn batch_create_with_back_references() {
    let tmp = setup_store();

    let batch_json = "{\"title\": \"Batch parent\", \"id\": \"wf-bp01\"}\n{\"title\": \"Batch child\", \"id\": \"wf-bc01\", \"dep\": [\"$1\"]}\n";

    let mut child = vima_cmd(&tmp)
        .args(["create", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn batch create");

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(batch_json.as_bytes()).unwrap();
        // stdin is closed when dropped here
    }

    let output = child
        .wait_with_output()
        .expect("failed to wait on batch create");

    assert!(
        output.status.success(),
        "batch create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let results: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("batch output not JSON array");
    assert_eq!(results.len(), 2);

    let parent_id = results[0]["id"].as_str().unwrap();
    let child_id = results[1]["id"].as_str().unwrap();

    // Child should depend on parent
    let child_json = run_ok(vima_cmd(&tmp).args(["show", child_id]));
    let deps: Vec<&str> = child_json["deps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(deps.contains(&parent_id), "child should depend on parent");
}

// ── Multi-ticket close ──────────────────────────────────────────────────

#[test]
fn close_multiple_tickets_at_once() {
    let tmp = setup_store();
    let a = create_ticket(&tmp, "Multi close A");
    let b = create_ticket(&tmp, "Multi close B");

    run_ok(vima_cmd(&tmp).args(["close", &a, &b]));

    let json_a = run_ok(vima_cmd(&tmp).args(["show", &a]));
    let json_b = run_ok(vima_cmd(&tmp).args(["show", &b]));
    assert_eq!(json_a["status"], "closed");
    assert_eq!(json_b["status"], "closed");
}

// ── Pluck and count ─────────────────────────────────────────────────────

#[test]
fn pluck_returns_only_requested_fields() {
    let tmp = setup_store();
    create_ticket(&tmp, "Pluck test");

    let json = run_ok(vima_cmd(&tmp).args(["list", "--pluck", "id,title"]));
    let arr = json.as_array().unwrap();
    assert!(!arr.is_empty());
    let first = &arr[0];
    assert!(first.get("id").is_some());
    assert!(first.get("title").is_some());
    // Should not have other fields like priority, status
    assert!(first.get("priority").is_none());
    assert!(first.get("status").is_none());
}

#[test]
fn count_returns_number() {
    let tmp = setup_store();
    create_ticket(&tmp, "Count 1");
    create_ticket(&tmp, "Count 2");
    create_ticket(&tmp, "Count 3");

    let json = run_ok(vima_cmd(&tmp).args(["list", "--count"]));
    assert_eq!(json.as_i64().unwrap(), 3);
}

// ── Dry run does not persist ────────────────────────────────────────────

#[test]
fn dry_run_does_not_create_ticket() {
    let tmp = setup_store();

    let output = vima_cmd(&tmp)
        .args(["--dry-run", "create", "Ghost ticket"])
        .output()
        .unwrap();
    assert!(output.status.success());

    // List should be empty
    let json = run_ok(vima_cmd(&tmp).args(["list", "--count"]));
    assert_eq!(json.as_i64().unwrap(), 0);
}

// ── Error handling: not found ───────────────────────────────────────────

#[test]
fn show_nonexistent_returns_exit_3() {
    let tmp = setup_store();

    let output = vima_cmd(&tmp).args(["show", "wf-0000"]).output().unwrap();
    assert_eq!(output.status.code().unwrap(), 3);

    let stderr = String::from_utf8_lossy(&output.stderr);
    let err: serde_json::Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(err["error"], "not_found");
    assert!(err["suggestion"].as_str().unwrap().contains("vima list"));
}

// ── Undep removes dependency ────────────────────────────────────────────

#[test]
fn undep_removes_dependency() {
    let tmp = setup_store();
    let a = create_ticket(&tmp, "Undep A");
    let b = create_ticket(&tmp, "Undep B");

    run_ok(vima_cmd(&tmp).args(["dep", "add", &a, &b]));
    run_ok(vima_cmd(&tmp).args(["undep", &a, &b]));

    let json = run_ok(vima_cmd(&tmp).args(["show", &a]));
    assert!(json["deps"].as_array().unwrap().is_empty());
}

// ── JSON mode emits no stderr (agent-friendly) ─────────────────────────

#[test]
fn json_mode_create_emits_no_stderr() {
    let tmp = setup_store();
    let output = vima_cmd(&tmp)
        .args(["create", "No stderr please", "--type", "task"])
        .output()
        .expect("failed to run vima create");
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.is_empty(),
        "JSON mode should not emit to stderr, got: {stderr}"
    );
    // stdout must still be valid JSON with the ticket id
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout not valid JSON");
    assert!(json["id"].as_str().is_some());
}

#[test]
fn pretty_mode_create_emits_stderr_confirmation() {
    let tmp = setup_store();
    let output = vima_cmd(&tmp)
        .args(["--pretty", "create", "With stderr", "--type", "task"])
        .output()
        .expect("failed to run vima create");
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Created "),
        "pretty mode should emit 'Created' to stderr, got: {stderr}"
    );
}

#[test]
fn json_mode_update_emits_no_stderr() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "Update me");
    let output = vima_cmd(&tmp)
        .args(["update", &id, "--priority", "3"])
        .output()
        .expect("failed to run vima update");
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.is_empty(),
        "JSON mode update should not emit to stderr, got: {stderr}"
    );
}

#[test]
fn json_mode_close_emits_no_stderr() {
    let tmp = setup_store();
    let id = create_ticket(&tmp, "Close me");
    let output = vima_cmd(&tmp)
        .args(["close", &id])
        .output()
        .expect("failed to run vima close");
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.is_empty(),
        "JSON mode close should not emit to stderr, got: {stderr}"
    );
}
