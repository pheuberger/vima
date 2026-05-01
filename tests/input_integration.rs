use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

fn vima_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vima"))
}

fn setup_store() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let vima_dir = tmp.path().join(".vima");
    let tickets_dir = vima_dir.join("tickets");
    fs::create_dir_all(&tickets_dir).unwrap();
    fs::write(vima_dir.join("config.yml"), "prefix: in\n").unwrap();
    tmp
}

fn vima_cmd(tmp: &tempfile::TempDir) -> Command {
    let mut cmd = vima_bin();
    cmd.env("VIMA_DIR", tmp.path().join(".vima"));
    cmd.env("VIMA_EXACT", "true");
    cmd
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

// ── @PATH substitution on create ────────────────────────────────────────

#[test]
fn create_with_description_at_path_reads_file() {
    let tmp = setup_store();
    let body = "## Heading\n\nMulti-line body with `code`,\n\"quotes\", and `backticks`.\n";
    let f = tempfile::NamedTempFile::new().unwrap();
    fs::write(f.path(), body).unwrap();

    let json = run_ok(vima_cmd(&tmp).args([
        "create",
        "Title",
        "--description",
        &format!("@{}", f.path().display()),
    ]));
    assert_eq!(json["description"], body);
}

#[test]
fn create_resolves_all_three_long_form_fields_from_files() {
    let tmp = setup_store();
    let desc = "DESC body\n";
    let design = "DESIGN body\n";
    let acc = "ACC body\n";
    let fd = tempfile::NamedTempFile::new().unwrap();
    fs::write(fd.path(), desc).unwrap();
    let fg = tempfile::NamedTempFile::new().unwrap();
    fs::write(fg.path(), design).unwrap();
    let fa = tempfile::NamedTempFile::new().unwrap();
    fs::write(fa.path(), acc).unwrap();

    let json = run_ok(vima_cmd(&tmp).args([
        "create",
        "Title",
        "--description",
        &format!("@{}", fd.path().display()),
        "--design",
        &format!("@{}", fg.path().display()),
        "--acceptance",
        &format!("@{}", fa.path().display()),
    ]));
    assert_eq!(json["description"], desc);
    assert_eq!(json["design"], design);
    assert_eq!(json["acceptance"], acc);
}

#[test]
fn create_with_double_at_passes_literal() {
    let tmp = setup_store();
    let json =
        run_ok(vima_cmd(&tmp).args(["create", "Title", "--description", "@@literal-not-a-path"]));
    assert_eq!(json["description"], "@literal-not-a-path");
}

// ── Stdin sentinel ──────────────────────────────────────────────────────

#[test]
fn create_with_description_dash_reads_stdin() {
    let tmp = setup_store();
    let body = "from stdin\nwith newlines\n";

    let mut child = vima_cmd(&tmp)
        .args(["create", "Title", "--description", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stdin create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["description"], body);
}

// ── --json @PATH ─────────────────────────────────────────────────────────

#[test]
fn create_with_json_at_path_reads_file() {
    let tmp = setup_store();
    let payload = serde_json::json!({
        "title": "From JSON file",
        "priority": 0,
        "tags": ["a", "b"],
        "description": "json-described\n",
    });
    let f = tempfile::NamedTempFile::new().unwrap();
    fs::write(f.path(), payload.to_string()).unwrap();

    let json =
        run_ok(vima_cmd(&tmp).args(["create", "--json", &format!("@{}", f.path().display())]));
    assert_eq!(json["title"], "From JSON file");
    assert_eq!(json["priority"], 0);
    assert_eq!(json["description"], "json-described\n");
}

// ── Negative paths ──────────────────────────────────────────────────────

#[test]
fn missing_file_returns_not_found_exit_3() {
    let tmp = setup_store();
    let output = vima_cmd(&tmp)
        .args([
            "create",
            "Title",
            "--description",
            "@/nonexistent/path/should/not/exist.md",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code().unwrap(), 3);
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(err["error"], "not_found");
    assert!(err["message"]
        .as_str()
        .unwrap()
        .contains("/nonexistent/path/should/not/exist.md"));
}

#[test]
fn double_dash_stdin_returns_invalid_argument_exit_1() {
    let tmp = setup_store();
    let mut child = vima_cmd(&tmp)
        .args(["create", "Title", "--description", "-", "--acceptance", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"some content")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code().unwrap(), 1);
    let err: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(err["error"], "invalid_argument");
}

// ── Update command supports the same conventions ────────────────────────

#[test]
fn update_with_description_at_path() {
    let tmp = setup_store();
    let id = run_ok(vima_cmd(&tmp).args(["create", "Title"]))["id"]
        .as_str()
        .unwrap()
        .to_string();

    let body = "## Updated\n\nbody with `backticks`";
    let f = tempfile::NamedTempFile::new().unwrap();
    fs::write(f.path(), body).unwrap();

    let json = run_ok(vima_cmd(&tmp).args([
        "update",
        &id,
        "--description",
        &format!("@{}", f.path().display()),
    ]));
    assert_eq!(json["description"], body);
}

#[test]
fn update_with_dash_reads_stdin() {
    let tmp = setup_store();
    let id = run_ok(vima_cmd(&tmp).args(["create", "Title"]))["id"]
        .as_str()
        .unwrap()
        .to_string();

    let body = "stdin update";
    let mut child = vima_cmd(&tmp)
        .args(["update", &id, "--acceptance", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["acceptance"], body);
}

// ── Regression: existing inline-string usage unchanged ──────────────────

#[test]
fn inline_description_unchanged() {
    let tmp = setup_store();
    let json = run_ok(vima_cmd(&tmp).args([
        "create",
        "Title",
        "--description",
        "plain inline string with no sentinels",
    ]));
    assert_eq!(json["description"], "plain inline string with no sentinels");
}

#[test]
fn inline_description_starting_with_a_at_in_middle_unchanged() {
    // Only a leading @ triggers substitution; @ in the middle is preserved.
    let tmp = setup_store();
    let json = run_ok(vima_cmd(&tmp).args([
        "create",
        "Title",
        "--description",
        "see user@example.com for details",
    ]));
    assert_eq!(json["description"], "see user@example.com for details");
}
