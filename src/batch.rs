use std::io::BufRead;
use std::path::Path;

use nanoid::nanoid;

use crate::deps;
use crate::error::{Error, Result};
use crate::filter;
use crate::id;
use crate::store::Store;
use crate::ticket::{self, Ticket};

const BATCH_MAX_LINES: usize = 1000;
const BATCH_MAX_LINE_BYTES: usize = 1024 * 1024; // 1 MB

const ALPHANUMERIC: [char; 36] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h',
    'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
];

/// Resolve a single back-reference value.
/// If `value` starts with `$`, parse the 1-based index and look up in `created_ids`.
/// Non-`$` values pass through unchanged.
pub fn resolve_backrefs(value: &str, created_ids: &[String]) -> Result<String> {
    if let Some(rest) = value.strip_prefix('$') {
        let index: usize = rest.parse().map_err(|_| {
            Error::InvalidBackref(format!("'{}' is not a valid back-reference", value))
        })?;
        if index == 0 || index > created_ids.len() {
            return Err(Error::InvalidBackref(format!(
                "'{}' is out of range (only {} ticket(s) created so far)",
                value,
                created_ids.len()
            )));
        }
        Ok(created_ids[index - 1].clone())
    } else {
        Ok(value.to_string())
    }
}

/// Resolve back-references within a specific field of a JSON spec object.
/// If the field is a string, resolve it as a single backref.
/// If the field is an array, resolve each string element.
pub fn resolve_value_backrefs(
    spec: &mut serde_json::Value,
    field: &str,
    created_ids: &[String],
) -> Result<()> {
    let current = match spec.get(field) {
        None => return Ok(()),
        Some(v) => v.clone(),
    };
    match current {
        serde_json::Value::String(s) => {
            let resolved = resolve_backrefs(&s, created_ids)?;
            spec[field] = serde_json::Value::String(resolved);
        }
        serde_json::Value::Array(arr) => {
            let mut new_arr = Vec::with_capacity(arr.len());
            for v in arr {
                let resolved_v = if let serde_json::Value::String(s) = v {
                    serde_json::Value::String(resolve_backrefs(&s, created_ids)?)
                } else {
                    v
                };
                new_arr.push(resolved_v);
            }
            spec[field] = serde_json::Value::Array(new_arr);
        }
        _ => {}
    }
    Ok(())
}

fn get_string_array(spec: &serde_json::Value, field: &str) -> Vec<String> {
    match spec.get(field) {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    }
}

fn generate_batch_id(
    prefix: &str,
    tickets_dir: &Path,
    created_ids: &[String],
) -> Result<String> {
    for _ in 0..10 {
        let suffix = nanoid!(4, &ALPHANUMERIC);
        let id = format!("{prefix}-{suffix}");
        if !tickets_dir.join(format!("{id}.md")).exists() && !created_ids.contains(&id) {
            return Ok(id);
        }
    }
    Err(Error::IdExists(
        "could not generate unique id after 10 attempts; use --id to specify one".into(),
    ))
}

fn wrap_batch_error(line_num: usize, inner: Error, created_ids: &[String]) -> Error {
    let base = format!("batch line {} failed: {}", line_num, inner);
    if created_ids.is_empty() {
        Error::InvalidField(base)
    } else {
        Error::InvalidField(format!("{}; already created: [{}]", base, created_ids.join(", ")))
    }
}

/// Build and write a single ticket from a JSON spec.
///
/// Backrefs in dep/blocks/parent should already be resolved before calling this.
/// `created_ids` is used only for in-batch ID collision detection.
pub fn create_from_spec(
    store: &Store,
    spec: &serde_json::Value,
    created_ids: &[String],
    exact: bool,
) -> Result<Ticket> {
    let title = spec
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidField("title is required".into()))?
        .to_string();

    let priority = if let Some(p) = spec.get("priority") {
        let p = p
            .as_u64()
            .ok_or_else(|| Error::InvalidField("priority must be a number".into()))? as u8;
        if p > filter::MAX_PRIORITY {
            return Err(Error::InvalidField(format!(
                "priority must be 0-{}",
                filter::MAX_PRIORITY
            )));
        }
        p
    } else {
        2
    };

    let ticket_type = if let Some(t) = spec.get("type") {
        serde_json::from_value::<ticket::TicketType>(t.clone())
            .map_err(|_| Error::InvalidField(format!("unknown ticket type: {}", t)))?
    } else {
        ticket::TicketType::Task
    };

    let tags: Vec<String> = match spec.get("tags") {
        Some(serde_json::Value::String(s)) => crate::parse_tags(s),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    };

    let assignee = spec
        .get("assignee")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let estimate = spec
        .get("estimate")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let description = spec
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let design = spec
        .get("design")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let acceptance = spec
        .get("acceptance")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // dep — backrefs already resolved; now fuzzy/exact resolve against store
    let deps: Vec<String> = match spec.get("dep") {
        Some(serde_json::Value::String(s)) => vec![store.resolve_id(s, exact)?],
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| store.resolve_id(s, exact))
            .collect::<Result<Vec<_>>>()?,
        _ => vec![],
    };

    // parent — backref already resolved
    let parent = match spec.get("parent").and_then(|v| v.as_str()) {
        Some(p) => Some(store.resolve_id(p, exact)?),
        None => None,
    };

    let tickets_dir = store.tickets_dir().to_path_buf();
    let ticket_id = if let Some(id_str) = spec.get("id").and_then(|v| v.as_str()) {
        id::validate_id(id_str)?;
        if tickets_dir.join(format!("{}.md", id_str)).exists() {
            return Err(Error::IdExists(id_str.to_string()));
        }
        if created_ids.contains(&id_str.to_string()) {
            return Err(Error::IdExists(id_str.to_string()));
        }
        id_str.to_string()
    } else {
        let project_root = store
            .root()
            .parent()
            .ok_or_else(|| Error::InvalidField("could not determine project root".into()))?;
        let prefix = id::get_prefix(project_root)?;
        generate_batch_id(&prefix, &tickets_dir, created_ids)?
    };

    // Cycle detection for deps
    {
        let tickets = store.read_all()?;
        for dep in &deps {
            if let Some(cycle_path) = deps::would_create_cycle(&tickets, &ticket_id, dep) {
                return Err(Error::Cycle(cycle_path));
            }
        }
    }

    let ticket = Ticket {
        id: ticket_id.clone(),
        title,
        status: ticket::Status::Open,
        ticket_type,
        priority,
        tags,
        assignee,
        estimate,
        deps,
        links: vec![],
        parent,
        created: jiff::Timestamp::now().to_string(),
        description,
        design,
        acceptance,
        notes: vec![],
        body: None,
        blocks: vec![],
        children: vec![],
    };

    store.write_ticket(&ticket)?;
    Ok(ticket)
}

/// Batch-create tickets from a JSON-lines reader.
/// Each non-empty line must be a JSON object representing a ticket spec.
fn batch_create_reader<R: BufRead>(store: &Store, reader: R, exact: bool) -> Result<Vec<Ticket>> {
    // Read at most 1001 raw lines to detect overflow
    let mut raw_lines: Vec<String> = Vec::new();
    for result in reader.lines().take(BATCH_MAX_LINES + 1) {
        raw_lines.push(result.map_err(Error::IoError)?);
    }

    if raw_lines.len() > BATCH_MAX_LINES {
        return Err(Error::InvalidField("batch exceeds 1000 line limit".into()));
    }

    let mut created_ids: Vec<String> = Vec::new();
    let mut created_tickets: Vec<Ticket> = Vec::new();

    for (idx, line) in raw_lines.iter().enumerate() {
        let line_num = idx + 1;

        // Check raw line length (before trimming)
        if line.len() > BATCH_MAX_LINE_BYTES {
            return Err(Error::InvalidField(format!(
                "batch line {} exceeds 1MB limit",
                line_num
            )));
        }

        // Skip empty lines
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Parse JSON
        let mut spec: serde_json::Value =
            serde_json::from_str(trimmed).map_err(|e| {
                wrap_batch_error(line_num, Error::YamlError(e.to_string()), &created_ids)
            })?;

        // Resolve back-references in dep, blocks, parent
        for field in &["dep", "blocks", "parent"] {
            resolve_value_backrefs(&mut spec, field, &created_ids)
                .map_err(|e| wrap_batch_error(line_num, e, &created_ids))?;
        }

        // Create and write ticket
        let ticket = create_from_spec(store, &spec, &created_ids, exact)
            .map_err(|e| wrap_batch_error(line_num, e, &created_ids))?;

        // Push ticket_id before the blocks loop so that if blocks processing
        // fails (e.g. cycle detected), wrap_batch_error correctly lists this
        // ticket among the already-created IDs.
        created_ids.push(ticket.id.clone());
        created_tickets.push(ticket);

        // Handle "blocks": add ticket_id to each target's deps
        let blocks = get_string_array(&spec, "blocks");
        if !blocks.is_empty() {
            let ticket_id = created_ids.last().unwrap().as_str();
            // Read once for all cycle checks on this line's block targets.
            // Re-reading after each add_dep is unnecessary: the newly added edges
            // (target → ticket_id) cannot affect cycle detection for other targets
            // because ticket_id's spec deps are immutable and were already checked.
            let tickets = store
                .read_all()
                .map_err(|e| wrap_batch_error(line_num, e, &created_ids))?;
            for block_target in &blocks {
                if let Some(cycle_path) =
                    deps::would_create_cycle(&tickets, block_target, ticket_id)
                {
                    return Err(wrap_batch_error(
                        line_num,
                        Error::Cycle(cycle_path),
                        &created_ids,
                    ));
                }
                store
                    .add_dep(block_target, ticket_id)
                    .map_err(|e| wrap_batch_error(line_num, e, &created_ids))?;
            }
        }
    }

    Ok(created_tickets)
}

/// Batch-create tickets by reading JSON lines from stdin.
pub fn batch_create(store: &Store, exact: bool) -> Result<Vec<Ticket>> {
    let stdin = std::io::stdin();
    batch_create_reader(store, stdin.lock(), exact)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::io::Cursor;

    fn setup_store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().expect("create tempdir");
        let vima = tmp.path().join(".vima");
        std::fs::create_dir_all(vima.join("tickets")).unwrap();
        std::fs::write(vima.join("config.yml"), "prefix: vi\n").unwrap();
        std::env::set_var("VIMA_DIR", vima.to_str().unwrap());
        let store = Store::open().unwrap();
        (tmp, store)
    }

    fn run_batch(store: &Store, input: &str) -> Result<Vec<Ticket>> {
        batch_create_reader(store, Cursor::new(input.as_bytes()), true)
    }

    // ── unit tests ────────────────────────────────────────────────────────────

    #[test]
    fn resolve_backrefs_valid() {
        let ids = vec!["id-a".to_string()];
        assert_eq!(resolve_backrefs("$1", &ids).unwrap(), "id-a");
    }

    #[test]
    fn resolve_backrefs_zero_index_is_error() {
        let ids: Vec<String> = vec![];
        let err = resolve_backrefs("$0", &ids).unwrap_err();
        assert!(matches!(err, Error::InvalidBackref(_)));
    }

    #[test]
    fn resolve_backrefs_out_of_range_is_error() {
        let ids = vec!["id-a".to_string()];
        let err = resolve_backrefs("$99", &ids).unwrap_err();
        assert!(matches!(err, Error::InvalidBackref(_)));
    }

    #[test]
    fn resolve_backrefs_non_dollar_passes_through() {
        let ids: Vec<String> = vec![];
        assert_eq!(resolve_backrefs("hello", &ids).unwrap(), "hello");
    }

    #[test]
    fn resolve_backrefs_invalid_syntax_is_error() {
        let ids: Vec<String> = vec![];
        let err = resolve_backrefs("$abc", &ids).unwrap_err();
        assert!(matches!(err, Error::InvalidBackref(_)));
    }

    #[test]
    fn resolve_value_backrefs_string_field() {
        let mut spec = serde_json::json!({"parent": "$1"});
        let ids = vec!["vi-abc1".to_string()];
        resolve_value_backrefs(&mut spec, "parent", &ids).unwrap();
        assert_eq!(spec["parent"], serde_json::json!("vi-abc1"));
    }

    #[test]
    fn resolve_value_backrefs_array_field() {
        let mut spec = serde_json::json!({"dep": ["$1", "other"]});
        let ids = vec!["vi-abc1".to_string()];
        resolve_value_backrefs(&mut spec, "dep", &ids).unwrap();
        assert_eq!(spec["dep"], serde_json::json!(["vi-abc1", "other"]));
    }

    #[test]
    fn resolve_value_backrefs_missing_field_is_noop() {
        let mut spec = serde_json::json!({"title": "test"});
        let ids: Vec<String> = vec![];
        resolve_value_backrefs(&mut spec, "dep", &ids).unwrap();
        assert!(spec.get("dep").is_none());
    }

    // ── integration tests ─────────────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn batch_create_two_tickets_returns_array() {
        let (_tmp, store) = setup_store();
        let input = r#"{"title": "Ticket One", "id": "t1"}
{"title": "Ticket Two", "id": "t2"}
"#;
        let tickets = run_batch(&store, input).unwrap();
        assert_eq!(tickets.len(), 2);
        assert_eq!(tickets[0].id, "t1");
        assert_eq!(tickets[1].id, "t2");
    }

    #[test]
    #[serial(env)]
    fn batch_dep_backref_resolves() {
        let (_tmp, store) = setup_store();
        let input = r#"{"title": "First", "id": "first"}
{"title": "Second", "id": "second", "dep": ["$1"]}
"#;
        let tickets = run_batch(&store, input).unwrap();
        assert_eq!(tickets.len(), 2);
        assert_eq!(tickets[1].deps, vec!["first"]);
    }

    #[test]
    #[serial(env)]
    fn batch_blocks_backref_updates_first_ticket_deps() {
        let (_tmp, store) = setup_store();
        let input = r#"{"title": "First", "id": "first"}
{"title": "Second", "id": "second", "blocks": ["$1"]}
"#;
        let tickets = run_batch(&store, input).unwrap();
        assert_eq!(tickets.len(), 2);

        // "first" should now have "second" in its deps
        let first = store.read_ticket("first").unwrap();
        assert!(
            first.deps.contains(&"second".to_string()),
            "expected first.deps to contain second, got: {:?}",
            first.deps
        );
    }

    #[test]
    #[serial(env)]
    fn batch_parent_backref_resolves() {
        let (_tmp, store) = setup_store();
        let input = r#"{"title": "Epic", "id": "epic", "type": "epic"}
{"title": "Child", "id": "child", "parent": "$1"}
"#;
        let tickets = run_batch(&store, input).unwrap();
        assert_eq!(tickets.len(), 2);
        assert_eq!(tickets[1].parent, Some("epic".to_string()));
    }

    #[test]
    #[serial(env)]
    fn batch_invalid_backref_returns_error_with_line_number() {
        let (_tmp, store) = setup_store();
        let input = r#"{"title": "First", "id": "t1"}
{"title": "Second", "dep": ["$99"]}
"#;
        let err = run_batch(&store, input).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 2"), "expected line 2 in error: {msg}");
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    #[serial(env)]
    fn batch_exceeding_1000_lines_returns_error() {
        let (_tmp, store) = setup_store();
        // Generate 1001 lines
        let lines: String = (0..1001)
            .map(|i| format!("{{\"title\": \"T{}\", \"id\": \"t{}\"}}\n", i, i))
            .collect();
        let err = run_batch(&store, &lines).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("1000 line limit"), "unexpected error: {msg}");
    }

    #[test]
    #[serial(env)]
    fn batch_cycle_detected_on_cyclic_line() {
        let (_tmp, store) = setup_store();
        // t2 depends on t1 AND t2 blocks t1 (= t1 depends on t2) → cycle
        let input = r#"{"title": "T1", "id": "t1"}
{"title": "T2", "id": "t2", "dep": ["$1"], "blocks": ["$1"]}
"#;
        let err = run_batch(&store, input).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 2"), "expected line 2 in error: {msg}");
        // The cycle error is wrapped in InvalidField
        assert!(matches!(err, Error::InvalidField(_)));
        assert!(msg.contains("cycle") || msg.contains("already created"), "unexpected: {msg}");

        // Both t1 and t2 are persisted on disk even though blocks processing failed.
        // The error message must list both IDs so the user knows what to clean up.
        assert!(
            msg.contains("t1"),
            "error should mention t1 in already-created list: {msg}"
        );
        assert!(
            msg.contains("t2"),
            "error should mention t2 in already-created list: {msg}"
        );

        // t2 should have dep on t1, but t1 must NOT have dep on t2 (blocks failed).
        let t2 = store.read_ticket("t2").unwrap();
        assert!(
            t2.deps.contains(&"t1".to_string()),
            "t2 should depend on t1: {:?}",
            t2.deps
        );
        let t1 = store.read_ticket("t1").unwrap();
        assert!(
            !t1.deps.contains(&"t2".to_string()),
            "t1 must NOT depend on t2 (blocks cycle was rejected): {:?}",
            t1.deps
        );
    }

    #[test]
    #[serial(env)]
    fn batch_partial_failure_first_ticket_remains_on_disk() {
        let (_tmp, store) = setup_store();
        let input = r#"{"title": "First", "id": "persisted"}
{"no_title_field": true}
"#;
        let err = run_batch(&store, input).unwrap_err();
        assert!(err.to_string().contains("line 2"), "expected line 2 in error: {}", err);

        // First ticket must still exist on disk
        let ticket = store.read_ticket("persisted").unwrap();
        assert_eq!(ticket.title, "First");
    }

    #[test]
    #[serial(env)]
    fn batch_empty_lines_skipped() {
        let (_tmp, store) = setup_store();
        let input = "\n{\"title\": \"A\", \"id\": \"a1\"}\n\n{\"title\": \"B\", \"id\": \"b1\"}\n\n";
        let tickets = run_batch(&store, input).unwrap();
        assert_eq!(tickets.len(), 2);
    }

    #[test]
    #[serial(env)]
    fn batch_id_collision_within_batch_is_error() {
        let (_tmp, store) = setup_store();
        let input = r#"{"title": "First", "id": "dup"}
{"title": "Dupe", "id": "dup"}
"#;
        let err = run_batch(&store, input).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 2"), "expected line 2: {msg}");
        assert!(msg.contains("already created") || msg.contains("dup"), "unexpected: {msg}");
    }

    #[test]
    #[serial(env)]
    fn batch_auto_generated_ids_are_unique() {
        let (_tmp, store) = setup_store();
        let input = r#"{"title": "Auto One"}
{"title": "Auto Two"}
"#;
        let tickets = run_batch(&store, input).unwrap();
        assert_eq!(tickets.len(), 2);
        assert_ne!(tickets[0].id, tickets[1].id);
    }

    // ── error-path tests ─────────────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn batch_malformed_json_returns_error() {
        let (_tmp, store) = setup_store();
        let input = "this is not json at all\n";
        let err = run_batch(&store, input).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 1"), "expected line 1 in error: {msg}");
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    #[serial(env)]
    fn batch_truncated_json_returns_error() {
        let (_tmp, store) = setup_store();
        // Missing closing brace — valid start but truncated
        let input = "{\"title\": \"incomplete\n";
        let err = run_batch(&store, input).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 1"), "expected line 1 in error: {msg}");
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    #[serial(env)]
    fn batch_missing_title_returns_error() {
        let (_tmp, store) = setup_store();
        let input = "{\"priority\": 1, \"id\": \"no-title\"}\n";
        let err = run_batch(&store, input).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("title is required"),
            "expected 'title is required' in: {msg}"
        );
        assert!(msg.contains("line 1"), "expected line 1 in error: {msg}");
    }

    #[test]
    #[serial(env)]
    fn batch_line_exceeds_max_bytes_returns_error() {
        let (_tmp, store) = setup_store();
        // Create a single line that exceeds BATCH_MAX_LINE_BYTES (1 MB)
        let huge_value = "x".repeat(BATCH_MAX_LINE_BYTES + 1);
        let input = format!("{{\"title\": \"{}\"}}\n", huge_value);
        let err = run_batch(&store, &input).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("1MB limit"), "expected '1MB limit' in: {msg}");
    }

    #[test]
    #[serial(env)]
    fn batch_empty_input_returns_empty_vec() {
        let (_tmp, store) = setup_store();
        let tickets = run_batch(&store, "").unwrap();
        assert!(tickets.is_empty(), "expected empty result for empty input");
    }

    #[test]
    #[serial(env)]
    fn batch_only_blank_lines_returns_empty_vec() {
        let (_tmp, store) = setup_store();
        let tickets = run_batch(&store, "\n\n  \n\n").unwrap();
        assert!(tickets.is_empty(), "expected empty result for blank-only input");
    }

    #[test]
    #[serial(env)]
    fn batch_unknown_fields_are_ignored() {
        let (_tmp, store) = setup_store();
        let input = r#"{"title": "Has extras", "id": "uf1", "bogus_field": 42, "another": "ignored"}
"#;
        let tickets = run_batch(&store, input).unwrap();
        assert_eq!(tickets.len(), 1);
        assert_eq!(tickets[0].title, "Has extras");
    }

    #[test]
    #[serial(env)]
    fn batch_malformed_json_after_valid_line_reports_correct_line() {
        let (_tmp, store) = setup_store();
        let input = r#"{"title": "Good", "id": "ok1"}
not valid json
"#;
        let err = run_batch(&store, input).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 2"), "expected line 2 in error: {msg}");
        // The first ticket should still exist on disk
        let ticket = store.read_ticket("ok1").unwrap();
        assert_eq!(ticket.title, "Good");
    }

    #[test]
    #[serial(env)]
    fn batch_forward_backref_out_of_range_is_error() {
        let (_tmp, store) = setup_store();
        // $2 referenced on line 1 when no tickets have been created yet
        let input = r#"{"title": "A", "id": "ca", "dep": ["$2"]}
{"title": "B", "id": "cb", "dep": ["$1"]}
"#;
        let err = run_batch(&store, input).unwrap_err();
        let msg = err.to_string();
        // $2 is out of range when processing line 1 (only 0 tickets created so far)
        assert!(msg.contains("line 1"), "expected line 1 in error: {msg}");
    }

    #[test]
    #[serial(env)]
    fn batch_invalid_priority_returns_error() {
        let (_tmp, store) = setup_store();
        let input = "{\"title\": \"Bad prio\", \"id\": \"bp1\", \"priority\": 99}\n";
        let err = run_batch(&store, input).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("priority"),
            "expected priority error: {msg}"
        );
    }

    #[test]
    #[serial(env)]
    fn batch_json_array_instead_of_object_returns_error() {
        let (_tmp, store) = setup_store();
        // A JSON array on a line instead of an object
        let input = "[{\"title\": \"inside array\"}]\n";
        let err = run_batch(&store, input).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("title is required"),
            "expected title error for non-object: {msg}"
        );
    }

}
