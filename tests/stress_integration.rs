use std::fs;
use std::process::Command;
use std::time::Instant;

fn vima_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vima"))
}

fn setup_store(prefix: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let vima_dir = tmp.path().join(".vima");
    let tickets_dir = vima_dir.join("tickets");
    fs::create_dir_all(&tickets_dir).unwrap();
    fs::write(
        vima_dir.join("config.yml"),
        format!("prefix: {}\n", prefix),
    )
    .unwrap();
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

/// Create N tickets via batch stdin, returns their IDs.
fn batch_create_n(tmp: &tempfile::TempDir, n: usize) -> Vec<String> {
    let mut lines = String::new();
    for i in 0..n {
        let priority = i % 5;
        let tag = match i % 3 {
            0 => "alpha",
            1 => "beta",
            _ => "gamma",
        };
        lines.push_str(&format!(
            "{{\"title\": \"Ticket {}\", \"priority\": {}, \"tags\": [\"{}\"]}}\n",
            i, priority, tag
        ));
    }

    let mut child = vima_cmd(tmp)
        .args(["create", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn batch create");

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(lines.as_bytes()).unwrap();
    }

    let output = child.wait_with_output().expect("batch create failed");
    assert!(
        output.status.success(),
        "batch create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let results: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("batch output not JSON");
    results
        .iter()
        .map(|v| v["id"].as_str().unwrap().to_string())
        .collect()
}

// ── Stress: list 500 tickets ────────────────────────────────────────────

#[test]
fn stress_list_500_tickets() {
    let tmp = setup_store("s1");
    let ids = batch_create_n(&tmp, 500);
    assert_eq!(ids.len(), 500);

    let start = Instant::now();
    let json = run_ok(vima_cmd(&tmp).args(["list", "--count"]));
    let elapsed = start.elapsed();

    assert_eq!(json.as_i64().unwrap(), 500);
    assert!(
        elapsed.as_secs() < 10,
        "listing 500 tickets took {:?}, expected < 10s",
        elapsed
    );
}

// ── Stress: filter by tag across 500 tickets ────────────────────────────

#[test]
fn stress_filter_by_tag_500_tickets() {
    let tmp = setup_store("s2");
    batch_create_n(&tmp, 500);

    let start = Instant::now();
    let json = run_ok(vima_cmd(&tmp).args(["list", "--tag", "alpha", "--count"]));
    let elapsed = start.elapsed();

    // ~167 tickets should have tag "alpha" (every 3rd)
    let count = json.as_i64().unwrap();
    assert!(
        (160..=170).contains(&count),
        "expected ~167 alpha tickets, got {}",
        count
    );
    assert!(
        elapsed.as_secs() < 10,
        "filtering 500 tickets took {:?}",
        elapsed
    );
}

// ── Stress: filter by priority range ────────────────────────────────────

#[test]
fn stress_filter_by_priority_500_tickets() {
    let tmp = setup_store("s3");
    batch_create_n(&tmp, 500);

    let start = Instant::now();
    let json = run_ok(vima_cmd(&tmp).args(["list", "--priority", "0-1", "--count"]));
    let elapsed = start.elapsed();

    // Priorities 0 and 1: 200 out of 500
    let count = json.as_i64().unwrap();
    assert_eq!(count, 200, "expected 200 tickets with priority 0-1");
    assert!(
        elapsed.as_secs() < 10,
        "priority filter on 500 tickets took {:?}",
        elapsed
    );
}

// ── Stress: dependency chain of 100 tickets ─────────────────────────────

#[test]
fn stress_dependency_chain_100() {
    let tmp = setup_store("s4");

    // Create 100 tickets with a chain: each depends on the previous
    let mut lines = String::new();
    lines.push_str("{\"title\": \"Chain 0\", \"id\": \"s4-c000\"}\n");
    for i in 1..100 {
        lines.push_str(&format!(
            "{{\"title\": \"Chain {}\", \"id\": \"s4-c{:03}\", \"dep\": [\"${}\"]}}\n",
            i, i, i // $i references the i-th ticket (1-indexed)
        ));
    }

    let mut child = vima_cmd(&tmp)
        .args(["create", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(lines.as_bytes())
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "chain batch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Dep tree from the last ticket should include all 100
    let start = Instant::now();
    let output = vima_cmd(&tmp)
        .args(["dep", "tree", "s4-c099", "--full"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let elapsed = start.elapsed();

    // Tree is deeply nested (100 levels) which exceeds serde_json recursion limit,
    // so we check the raw string output instead of parsing as JSON.
    let tree_str = String::from_utf8_lossy(&output.stdout);
    assert!(tree_str.contains("s4-c000"), "tree should reach root");
    assert!(tree_str.contains("s4-c099"), "tree should include last");
    assert!(
        elapsed.as_secs() < 10,
        "dep tree on 100-chain took {:?}",
        elapsed
    );
}

// ── Stress: dep cycle detection on 100 tickets ─────────────────────────

#[test]
fn stress_dep_cycle_detection_100() {
    let tmp = setup_store("s5");

    // Create 100 tickets in a chain
    let mut lines = String::new();
    lines.push_str("{\"title\": \"Cyc 0\", \"id\": \"s5-y000\"}\n");
    for i in 1..100 {
        lines.push_str(&format!(
            "{{\"title\": \"Cyc {}\", \"id\": \"s5-y{:03}\", \"dep\": [\"${}\"]}}\n",
            i, i, i
        ));
    }

    let mut child = vima_cmd(&tmp)
        .args(["create", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(lines.as_bytes())
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());

    // Try to add dep from first to last (creating a cycle)
    let start = Instant::now();
    let output = vima_cmd(&tmp)
        .args(["dep", "add", "s5-y000", "s5-y099"])
        .output()
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(
        output.status.code().unwrap(),
        2,
        "cycle should be detected with exit 2"
    );
    assert!(
        elapsed.as_secs() < 10,
        "cycle detection on 100-chain took {:?}",
        elapsed
    );
}

// ── Stress: pluck on 500 tickets ────────────────────────────────────────

#[test]
fn stress_pluck_500_tickets() {
    let tmp = setup_store("s6");
    batch_create_n(&tmp, 500);

    let start = Instant::now();
    let json = run_ok(vima_cmd(&tmp).args(["list", "--pluck", "id,title"]));
    let elapsed = start.elapsed();

    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 500);
    // Each entry should only have id and title
    let first = &arr[0];
    assert!(first.get("id").is_some());
    assert!(first.get("title").is_some());
    assert!(first.get("priority").is_none());
    assert!(
        elapsed.as_secs() < 10,
        "pluck on 500 tickets took {:?}",
        elapsed
    );
}

// ── Stress: ready command with many blocked tickets ─────────────────────

#[test]
fn stress_ready_with_deps_200() {
    let tmp = setup_store("s7");

    // Create 100 blocker tickets + 100 blocked tickets
    let mut lines = String::new();
    for i in 0..100 {
        lines.push_str(&format!(
            "{{\"title\": \"Blocker {}\", \"id\": \"s7-b{:03}\"}}\n",
            i, i
        ));
    }
    for i in 0..100 {
        // Each blocked ticket depends on the corresponding blocker
        lines.push_str(&format!(
            "{{\"title\": \"Blocked {}\", \"id\": \"s7-d{:03}\", \"dep\": [\"${}\"]}}\n",
            i, i, i + 1
        ));
    }

    let mut child = vima_cmd(&tmp)
        .args(["create", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(lines.as_bytes())
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "batch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let start = Instant::now();
    let json = run_ok(vima_cmd(&tmp).args(["ready", "--count"]));
    let elapsed = start.elapsed();

    // Only the 100 blockers should be ready (no deps of their own)
    assert_eq!(json.as_i64().unwrap(), 100);
    assert!(
        elapsed.as_secs() < 10,
        "ready on 200 tickets took {:?}",
        elapsed
    );
}
