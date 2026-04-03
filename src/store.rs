use std::fs;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use gray_matter::engine::YAML;
use gray_matter::Matter;
use sha2::{Digest, Sha256};

use crate::deps::compute_reverse_fields;
use crate::error::{Error, Result};
use crate::id;
use crate::ticket::{Note, Ticket};

/// Compute a 16-char hex version hash from ticket content (excluding the version field itself).
fn compute_version(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    result[..8].iter().map(|b| format!("{:02x}", b)).collect()
}

/// Extract the version field from raw YAML frontmatter without full deserialization.
fn extract_version_from_yaml(content: &str) -> Option<String> {
    let mut in_frontmatter = false;
    for line in content.lines() {
        if line == "---" {
            if in_frontmatter {
                break; // closing ---
            }
            in_frontmatter = true;
            continue;
        }
        if in_frontmatter {
            if let Some(v) = line.strip_prefix("version: ") {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Insert `version: <hash>` line after the `id:` line in serialized ticket content.
fn insert_version_line(content: &str, version: &str) -> String {
    // Content starts with "---\nid: ...\n" — insert after the id line
    let after_sep = match content.find('\n') {
        Some(pos) => pos + 1,
        None => return content.to_string(),
    };
    // Find end of id line
    let id_end = match content[after_sep..].find('\n') {
        Some(pos) => after_sep + pos + 1,
        None => return content.to_string(),
    };
    let mut result = String::with_capacity(content.len() + 30);
    result.push_str(&content[..id_end]);
    result.push_str(&format!("version: \"{}\"\n", version));
    result.push_str(&content[id_end..]);
    result
}

pub(crate) fn needs_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if s != s.trim() {
        return true;
    }
    if s.contains('\t') {
        return true;
    }
    let first = s.chars().next().unwrap();
    if "-*&!|>%@`,[]{}#?'\"".contains(first) {
        return true;
    }
    if s.contains(':')
        || s.contains('#')
        || s.contains('\n')
        || s.contains('"')
        || s.contains('\'')
        || s.contains(',')
        || s.contains(']')
        || s.contains('[')
    {
        return true;
    }
    if s.len() <= 5 {
        let lower = s.to_lowercase();
        if matches!(
            lower.as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off"
        ) {
            return true;
        }
    }
    if s.parse::<f64>().is_ok() {
        return true;
    }
    false
}

pub(crate) fn yaml_scalar(s: &str) -> String {
    if needs_quoting(s) {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\t', "\\t")
            .replace('\r', "\\r");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
}

pub(crate) fn write_yaml_array(out: &mut String, key: &str, items: &[String]) {
    if items.is_empty() {
        out.push_str(&format!("{}: []\n", key));
        return;
    }
    let items_str: Vec<String> = items.iter().map(|s| yaml_scalar(s)).collect();
    out.push_str(&format!("{}: [{}]\n", key, items_str.join(", ")));
}

pub(crate) fn write_yaml_notes(out: &mut String, notes: &[Note]) {
    if notes.is_empty() {
        out.push_str("notes: []\n");
        return;
    }
    out.push_str("notes:\n");
    for note in notes {
        out.push_str(&format!(
            "  - timestamp: {}\n",
            yaml_scalar(&note.timestamp)
        ));
        if note.text.contains('\n') {
            out.push_str("    text: |-\n");
            for line in note.text.lines() {
                out.push_str(&format!("        {}\n", line));
            }
        } else {
            out.push_str(&format!("    text: {}\n", yaml_scalar(&note.text)));
        }
    }
}

pub fn find_vima_root() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("VIMA_DIR") {
        let path = PathBuf::from(&dir);
        if path.is_dir() {
            return path.canonicalize().map_err(Error::Io);
        }
        return Err(Error::InvalidField(
            "VIMA_DIR points to non-existent directory".into(),
        ));
    }

    let cwd = std::env::current_dir()?;
    let mut current = cwd.as_path();
    loop {
        let candidate = current.join(".vima");
        if candidate.is_dir() {
            return candidate.canonicalize().map_err(Error::Io);
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return Err(Error::NoVimaDir),
        }
    }
}

pub struct Store {
    root: PathBuf,
    tickets: PathBuf,
}

/// RAII guard that holds a file lock. The lock is released when the guard is dropped.
pub struct LockGuard {
    _file: fs::File,
}

impl Store {
    pub fn open() -> Result<Self> {
        let root = find_vima_root()?;
        let tickets = root.join("tickets");
        fs::create_dir_all(&tickets)?;
        Ok(Store { root, tickets })
    }

    /// Acquire an exclusive (write) advisory lock on `.vima/lock`.
    /// Returns a guard that releases the lock on drop.
    pub fn lock_exclusive(&self) -> Result<LockGuard> {
        let lock_path = self.root.join("lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;
        file.lock_exclusive()?;
        Ok(LockGuard { _file: file })
    }

    /// Acquire a shared (read) advisory lock on `.vima/lock`.
    /// Returns a guard that releases the lock on drop.
    pub fn lock_shared(&self) -> Result<LockGuard> {
        let lock_path = self.root.join("lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;
        file.lock_shared()?;
        Ok(LockGuard { _file: file })
    }

    pub fn read_ticket(&self, id: &str) -> Result<Ticket> {
        let path = self.tickets.join(format!("{}.md", id));
        let contents = fs::read_to_string(&path)?;

        let parsed = Matter::<YAML>::new()
            .parse::<Ticket>(&contents)
            .map_err(|e| Error::Yaml(e.to_string()))?;

        let mut ticket = parsed
            .data
            .ok_or_else(|| Error::Yaml(format!("missing frontmatter in {}.md", id)))?;

        let body = parsed.content.trim();
        if !body.is_empty() {
            ticket.body = Some(body.to_string());
        }

        Ok(ticket)
    }

    pub fn read_all(&self) -> Result<Vec<Ticket>> {
        let mut tickets = Vec::new();
        for entry in fs::read_dir(&self.tickets)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if name.ends_with(".md.tmp") || !name.ends_with(".md") {
                continue;
            }
            let id = name.strip_suffix(".md").unwrap();
            match self.read_ticket(id) {
                Ok(ticket) => tickets.push(ticket),
                Err(e) => {
                    use std::io::Write;
                    let _ = writeln!(std::io::stderr(), "warning: skipping {}: {}", name, e);
                }
            }
        }
        Ok(tickets)
    }

    pub fn write_ticket(&self, ticket: &Ticket) -> Result<()> {
        let strip = |s: &str| s.replace('\0', "");

        let mut out = String::new();
        out.push_str("---\n");

        out.push_str(&format!("id: {}\n", yaml_scalar(&strip(&ticket.id))));
        out.push_str(&format!("title: {}\n", yaml_scalar(&strip(&ticket.title))));
        out.push_str(&format!("status: {}\n", ticket.status.as_str()));
        out.push_str(&format!("type: {}\n", ticket.ticket_type.as_str()));
        out.push_str(&format!("priority: {}\n", ticket.priority));

        let tags: Vec<String> = ticket.tags.iter().map(|s| strip(s)).collect();
        write_yaml_array(&mut out, "tags", &tags);

        match &ticket.assignee {
            Some(a) => out.push_str(&format!("assignee: {}\n", yaml_scalar(&strip(a)))),
            None => out.push_str("assignee: null\n"),
        }
        match ticket.estimate {
            Some(e) => out.push_str(&format!("estimate: {}\n", e)),
            None => out.push_str("estimate: null\n"),
        }

        let deps: Vec<String> = ticket.deps.iter().map(|s| strip(s)).collect();
        write_yaml_array(&mut out, "deps", &deps);

        let links: Vec<String> = ticket.links.iter().map(|s| strip(s)).collect();
        write_yaml_array(&mut out, "links", &links);

        match &ticket.parent {
            Some(p) => out.push_str(&format!("parent: {}\n", yaml_scalar(&strip(p)))),
            None => out.push_str("parent: null\n"),
        }

        out.push_str(&format!(
            "created: {}\n",
            yaml_scalar(&strip(&ticket.created))
        ));

        for (key, val) in &[
            ("description", &ticket.description),
            ("design", &ticket.design),
            ("acceptance", &ticket.acceptance),
        ] {
            if let Some(v) = val {
                let stripped = strip(v);
                if stripped.contains('\n') {
                    out.push_str(&format!("{}: |-\n", key));
                    for line in stripped.lines() {
                        out.push_str(&format!("  {}\n", line));
                    }
                } else {
                    out.push_str(&format!("{}: {}\n", key, yaml_scalar(&stripped)));
                }
            }
        }

        let stripped_notes: Vec<Note> = ticket
            .notes
            .iter()
            .map(|n| Note {
                timestamp: strip(&n.timestamp),
                text: strip(&n.text),
            })
            .collect();
        write_yaml_notes(&mut out, &stripped_notes);

        out.push_str("---\n");

        if let Some(body) = &ticket.body {
            let stripped = strip(body);
            if !stripped.is_empty() {
                out.push_str(&stripped);
                if !stripped.ends_with('\n') {
                    out.push('\n');
                }
            }
        }

        // Compute version hash from content (without version line)
        let new_version = compute_version(&out);

        // Conflict detection: if file exists, compare versions
        let final_path = self.tickets.join(format!("{}.md", ticket.id));
        if final_path.exists() {
            let on_disk_content = fs::read_to_string(&final_path)?;
            let on_disk_version = extract_version_from_yaml(&on_disk_content);
            // Only check if on-disk ticket has a version (skip for legacy tickets)
            if on_disk_version.is_some() && on_disk_version != ticket.version {
                return Err(Error::Stale {
                    id: ticket.id.clone(),
                    expected: ticket.version.clone(),
                    actual: on_disk_version,
                });
            }
        }

        // Insert version into content and write atomically
        let final_content = insert_version_line(&out, &new_version);
        let tmp_path = self.tickets.join(format!("{}.md.tmp", ticket.id));
        fs::write(&tmp_path, &final_content)?;
        fs::rename(&tmp_path, &final_path)?;

        Ok(())
    }

    pub fn resolve_id(&self, input: &str, exact: bool) -> Result<String> {
        id::resolve_id(&self.tickets, input, exact)
    }

    pub fn load_and_compute(&self, id: &str) -> Result<Ticket> {
        let mut tickets = self.read_all()?;
        compute_reverse_fields(&mut tickets);
        tickets
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| Error::NotFound(id.to_string()))
    }

    /// Add `dep_id` to the deps list of `ticket_id` (no-op if already present).
    pub fn add_dep(&self, ticket_id: &str, dep_id: &str) -> Result<()> {
        let mut ticket = self.read_ticket(ticket_id)?;
        if !ticket.deps.contains(&dep_id.to_string()) {
            ticket.deps.push(dep_id.to_string());
            self.write_ticket(&ticket)?;
        }
        Ok(())
    }

    pub fn tickets_dir(&self) -> &Path {
        &self.tickets
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

    fn make_temp() -> TempDir {
        tempfile::tempdir().expect("create tempdir")
    }

    #[test]
    #[serial(env)]
    fn find_vima_root_finds_vima_dir() {
        let tmp = make_temp();
        let vima = tmp.path().join(".vima");
        fs::create_dir(&vima).unwrap();

        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        let found = find_vima_root().unwrap();
        assert_eq!(found, vima.canonicalize().unwrap());
    }

    #[test]
    #[serial(env)]
    fn find_vima_root_walks_up_to_parent() {
        let tmp = make_temp();
        let vima = tmp.path().join(".vima");
        fs::create_dir(&vima).unwrap();

        let subdir = tmp.path().join("sub").join("deep");
        fs::create_dir_all(&subdir).unwrap();
        std::env::set_current_dir(&subdir).unwrap();
        std::env::remove_var("VIMA_DIR");

        let found = find_vima_root().unwrap();
        assert_eq!(found, vima.canonicalize().unwrap());
    }

    #[test]
    #[serial(env)]
    fn find_vima_root_respects_vima_dir_env() {
        let tmp = make_temp();
        let vima = tmp.path().join("custom_vima");
        fs::create_dir(&vima).unwrap();

        std::env::set_var("VIMA_DIR", vima.to_str().unwrap());
        let found = find_vima_root().unwrap();
        assert_eq!(found, vima.canonicalize().unwrap());
        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn find_vima_root_returns_no_vima_dir() {
        let tmp = tempfile::tempdir_in("/tmp").expect("create tempdir under /tmp");
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        let result = find_vima_root();
        assert!(matches!(result, Err(Error::NoVimaDir)));
    }

    fn make_store() -> (TempDir, Store, PathBuf) {
        let tmp = make_temp();
        let vima = tmp.path().join(".vima");
        fs::create_dir_all(vima.join("tickets")).unwrap();
        std::env::set_var("VIMA_DIR", vima.to_str().unwrap());
        let store = Store::open().unwrap();
        std::env::remove_var("VIMA_DIR");
        let tickets_dir = store.tickets_dir().to_path_buf();
        (tmp, store, tickets_dir)
    }

    fn store_with_ticket(content: &str) -> (TempDir, Store, String) {
        let (tmp, store, tickets_dir) = make_store();
        let id = "test-abc";
        fs::write(tickets_dir.join(format!("{}.md", id)), content).unwrap();
        (tmp, store, id.to_string())
    }

    const VALID_TICKET: &str = r#"---
id: test-abc
title: My Test Ticket
status: open
type: task
priority: 2
created: "2026-04-02T00:00:00Z"
---
This is the **markdown** body.
"#;

    #[test]
    #[serial(env)]
    fn read_ticket_parses_valid_ticket() {
        let (_tmp, store, id) = store_with_ticket(VALID_TICKET);
        let ticket = store.read_ticket(&id).unwrap();
        assert_eq!(ticket.id, "test-abc");
        assert_eq!(ticket.title, "My Test Ticket");
        assert_eq!(ticket.priority, 2);
    }

    #[test]
    #[serial(env)]
    fn read_ticket_preserves_body() {
        let (_tmp, store, id) = store_with_ticket(VALID_TICKET);
        let ticket = store.read_ticket(&id).unwrap();
        assert!(ticket.body.is_some());
        assert!(ticket.body.unwrap().contains("markdown"));
    }

    #[test]
    #[serial(env)]
    fn read_all_skips_unparseable_files() {
        let (_tmp, store, tickets_dir) = make_store();
        fs::write(tickets_dir.join("good.md"), VALID_TICKET).unwrap();
        fs::write(
            tickets_dir.join("bad.md"),
            "not frontmatter at all\njust text",
        )
        .unwrap();
        let tickets = store.read_all().unwrap();
        assert_eq!(tickets.len(), 1);
        assert_eq!(tickets[0].id, "test-abc");
    }

    #[test]
    #[serial(env)]
    fn read_all_excludes_tmp_files() {
        let (_tmp, store, tickets_dir) = make_store();
        fs::write(tickets_dir.join("good.md"), VALID_TICKET).unwrap();
        fs::write(tickets_dir.join("good.md.tmp"), VALID_TICKET).unwrap();
        let tickets = store.read_all().unwrap();
        assert_eq!(tickets.len(), 1);
    }

    // --- resolve_id ---

    #[test]
    #[serial(env)]
    fn store_resolve_id_exact_match() {
        let (_tmp, store, tickets_dir) = make_store();
        fs::write(tickets_dir.join("test-abc.md"), VALID_TICKET).unwrap();
        let result = store.resolve_id("test-abc", true).unwrap();
        assert_eq!(result, "test-abc");
    }

    #[test]
    #[serial(env)]
    fn store_resolve_id_fuzzy_match() {
        let (_tmp, store, tickets_dir) = make_store();
        fs::write(tickets_dir.join("test-abc.md"), VALID_TICKET).unwrap();
        let result = store.resolve_id("abc", false).unwrap();
        assert_eq!(result, "test-abc");
    }

    #[test]
    #[serial(env)]
    fn store_resolve_id_not_found() {
        let (_tmp, store, _tickets_dir) = make_store();
        let err = store.resolve_id("xyz", false).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    // --- needs_quoting ---

    #[test]
    fn needs_quoting_plain_word() {
        assert!(!needs_quoting("hello"));
    }

    #[test]
    fn needs_quoting_true_keyword() {
        assert!(needs_quoting("true"));
    }

    #[test]
    fn needs_quoting_colon_in_value() {
        assert!(needs_quoting(": colon"));
    }

    #[test]
    fn needs_quoting_empty() {
        assert!(needs_quoting(""));
    }

    #[test]
    fn needs_quoting_false_keyword() {
        assert!(needs_quoting("false"));
    }

    #[test]
    fn needs_quoting_null_keyword() {
        assert!(needs_quoting("null"));
    }

    #[test]
    fn needs_quoting_numeric() {
        assert!(needs_quoting("42"));
        assert!(needs_quoting("3.14"));
    }

    #[test]
    fn needs_quoting_leading_whitespace() {
        assert!(needs_quoting(" hello"));
    }

    #[test]
    fn needs_quoting_trailing_whitespace() {
        assert!(needs_quoting("hello "));
    }

    #[test]
    fn needs_quoting_contains_hash() {
        assert!(needs_quoting("foo#bar"));
    }

    #[test]
    fn needs_quoting_starts_with_dash() {
        assert!(needs_quoting("-item"));
    }

    #[test]
    fn needs_quoting_interior_comma() {
        assert!(needs_quoting("bug, critical"));
    }

    #[test]
    fn needs_quoting_interior_close_bracket() {
        assert!(needs_quoting("foo]bar"));
    }

    #[test]
    fn needs_quoting_interior_open_bracket() {
        assert!(needs_quoting("foo[bar"));
    }

    // --- yaml_scalar ---

    #[test]
    fn yaml_scalar_plain() {
        assert_eq!(yaml_scalar("hello"), "hello");
    }

    #[test]
    fn yaml_scalar_quotes_special() {
        assert_eq!(yaml_scalar("has \"quotes\""), "\"has \\\"quotes\\\"\"");
    }

    #[test]
    fn yaml_scalar_newline_escaped() {
        assert_eq!(yaml_scalar("line1\nline2"), "\"line1\\nline2\"");
    }

    // --- write_ticket round-trip ---

    use crate::ticket::{Note, Status, Ticket, TicketType};

    fn make_full_ticket() -> Ticket {
        Ticket {
            id: "rt-abc1".to_string(),
            version: None,
            title: "Round-trip: title with special chars".to_string(),
            status: Status::InProgress,
            ticket_type: TicketType::Bug,
            priority: 1,
            tags: vec![],
            assignee: Some("Alice Smith".to_string()),
            estimate: Some(90),
            deps: vec!["rt-dep1".to_string()],
            links: vec!["https://example.com".to_string()],
            parent: Some("rt-parent1".to_string()),
            created: "2026-04-02T10:00:00Z".to_string(),
            description: Some("First line\nSecond line".to_string()),
            design: Some("Design notes".to_string()),
            acceptance: Some("First criterion\nSecond criterion".to_string()),
            notes: vec![
                Note {
                    timestamp: "2026-04-02T10:01:00Z".to_string(),
                    text: "simple note".to_string(),
                },
                Note {
                    timestamp: "2026-04-02T10:02:00Z".to_string(),
                    text: "note with : colon\nnote # hash\nnote \"quotes\"".to_string(),
                },
            ],
            body: Some("Markdown body content.".to_string()),
            blocks: vec!["computed-block".to_string()],
            children: vec!["computed-child".to_string()],
        }
    }

    #[test]
    #[serial(env)]
    fn write_ticket_round_trip() {
        let (_tmp, store, _tickets_dir) = make_store();
        let original = make_full_ticket();
        store.write_ticket(&original).unwrap();
        let read_back = store.read_ticket(&original.id).unwrap();

        assert_eq!(read_back.id, original.id);
        assert_eq!(read_back.title, original.title);
        assert_eq!(read_back.status, original.status);
        assert_eq!(read_back.ticket_type, original.ticket_type);
        assert_eq!(read_back.priority, original.priority);
        assert_eq!(read_back.tags, original.tags);
        assert_eq!(read_back.assignee, original.assignee);
        assert_eq!(read_back.estimate, original.estimate);
        assert_eq!(read_back.deps, original.deps);
        assert_eq!(read_back.links, original.links);
        assert_eq!(read_back.parent, original.parent);
        assert_eq!(read_back.created, original.created);
        assert_eq!(read_back.description, original.description);
        assert_eq!(read_back.design, original.design);
        assert_eq!(read_back.acceptance, original.acceptance);
        assert_eq!(read_back.notes.len(), original.notes.len());
        assert_eq!(read_back.notes[0].timestamp, original.notes[0].timestamp);
        assert_eq!(read_back.notes[0].text, original.notes[0].text);
        assert_eq!(read_back.notes[1].timestamp, original.notes[1].timestamp);
        assert_eq!(read_back.notes[1].text, original.notes[1].text);
        assert_eq!(read_back.body, original.body);
    }

    #[test]
    #[serial(env)]
    fn write_ticket_no_blocks_or_children() {
        let (_tmp, store, tickets_dir) = make_store();
        let ticket = make_full_ticket();
        store.write_ticket(&ticket).unwrap();
        let content = fs::read_to_string(tickets_dir.join("rt-abc1.md")).unwrap();
        assert!(!content.contains("blocks:"));
        assert!(!content.contains("children:"));
    }

    #[test]
    #[serial(env)]
    fn write_ticket_no_tmp_file_on_success() {
        let (_tmp, store, tickets_dir) = make_store();
        let ticket = make_full_ticket();
        store.write_ticket(&ticket).unwrap();
        assert!(!tickets_dir.join("rt-abc1.md.tmp").exists());
        assert!(tickets_dir.join("rt-abc1.md").exists());
    }

    #[test]
    #[serial(env)]
    fn write_ticket_strips_null_bytes() {
        let (_tmp, store, tickets_dir) = make_store();
        let mut ticket = make_full_ticket();
        ticket.title = "Title\0with\0nulls".to_string();
        ticket.description = Some("Desc\0ription".to_string());
        store.write_ticket(&ticket).unwrap();
        let content = fs::read_to_string(tickets_dir.join("rt-abc1.md")).unwrap();
        assert!(!content.contains('\0'));
        assert!(content.contains("Titlewithnulls"));
    }

    #[test]
    #[serial(env)]
    fn write_ticket_tags_with_comma_and_bracket_round_trip() {
        let (_tmp, store, _tickets_dir) = make_store();
        let mut ticket = make_full_ticket();
        ticket.tags = vec!["bug, critical".to_string(), "needs ]".to_string()];
        store.write_ticket(&ticket).unwrap();
        let read_back = store.read_ticket(&ticket.id).unwrap();
        assert_eq!(read_back.tags, ticket.tags);
    }

    #[test]
    #[serial(env)]
    fn write_ticket_tag_with_open_bracket_round_trip() {
        let (_tmp, store, _tickets_dir) = make_store();
        let mut ticket = make_full_ticket();
        ticket.tags = vec!["open[bracket".to_string()];
        store.write_ticket(&ticket).unwrap();
        let read_back = store.read_ticket(&ticket.id).unwrap();
        assert_eq!(read_back.tags, ticket.tags);
    }

    #[test]
    #[serial(env)]
    fn load_and_compute_returns_ticket_with_reverse_fields() {
        let (_tmp, store, tickets_dir) = make_store();

        let blocker = r#"---
id: blocker
title: Blocker ticket
status: open
type: task
priority: 2
created: "2026-04-02T00:00:00Z"
deps: []
---
"#;
        let dependent = r#"---
id: dependent
title: Dependent ticket
status: open
type: task
priority: 2
created: "2026-04-02T00:00:00Z"
deps: [blocker]
---
"#;
        let child = r#"---
id: child
title: Child ticket
status: open
type: task
priority: 2
created: "2026-04-02T00:00:00Z"
parent: blocker
---
"#;

        fs::write(tickets_dir.join("blocker.md"), blocker).unwrap();
        fs::write(tickets_dir.join("dependent.md"), dependent).unwrap();
        fs::write(tickets_dir.join("child.md"), child).unwrap();

        let ticket = store.load_and_compute("blocker").unwrap();
        assert_eq!(ticket.blocks, vec!["dependent"]);
        assert_eq!(ticket.children, vec!["child"]);
    }

    #[test]
    #[serial(env)]
    fn load_and_compute_not_found() {
        let (_tmp, store, _tickets_dir) = make_store();
        let err = store.load_and_compute("nonexistent").unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    // --- round-trip tests for notes, links, parent ---

    fn make_minimal_ticket(id: &str) -> Ticket {
        Ticket {
            id: id.to_string(),
            version: None,
            title: "Minimal ticket".to_string(),
            status: Status::Open,
            ticket_type: TicketType::Task,
            priority: 2,
            tags: vec![],
            assignee: None,
            estimate: None,
            deps: vec![],
            links: vec![],
            parent: None,
            created: "2026-04-02T00:00:00Z".to_string(),
            description: None,
            design: None,
            acceptance: None,
            notes: vec![],
            body: None,
            blocks: vec![],
            children: vec![],
        }
    }

    #[test]
    #[serial(env)]
    fn round_trip_notes_with_timestamps_and_content() {
        let (_tmp, store, _tickets_dir) = make_store();
        let mut ticket = make_minimal_ticket("rt-n001");
        ticket.notes = vec![
            Note {
                timestamp: "2026-04-02T08:00:00Z".to_string(),
                text: "First note".to_string(),
            },
            Note {
                timestamp: "2026-04-02T09:30:00Z".to_string(),
                text: "Second note with special: chars".to_string(),
            },
            Note {
                timestamp: "2026-04-02T10:00:00Z".to_string(),
                text: "Multi-line note\nwith second line\nand third".to_string(),
            },
        ];
        store.write_ticket(&ticket).unwrap();
        let read_back = store.read_ticket(&ticket.id).unwrap();

        assert_eq!(read_back.notes.len(), 3);
        assert_eq!(read_back.notes[0].timestamp, "2026-04-02T08:00:00Z");
        assert_eq!(read_back.notes[0].text, "First note");
        assert_eq!(read_back.notes[1].timestamp, "2026-04-02T09:30:00Z");
        assert_eq!(read_back.notes[1].text, "Second note with special: chars");
        assert_eq!(read_back.notes[2].timestamp, "2026-04-02T10:00:00Z");
        assert_eq!(
            read_back.notes[2].text,
            "Multi-line note\nwith second line\nand third"
        );
    }

    #[test]
    #[serial(env)]
    fn round_trip_links() {
        let (_tmp, store, _tickets_dir) = make_store();
        let mut ticket = make_minimal_ticket("rt-l001");
        ticket.links = vec![
            "https://example.com/issue/1".to_string(),
            "https://docs.example.com/spec".to_string(),
        ];
        store.write_ticket(&ticket).unwrap();
        let read_back = store.read_ticket(&ticket.id).unwrap();

        assert_eq!(read_back.links.len(), 2);
        assert_eq!(read_back.links[0], "https://example.com/issue/1");
        assert_eq!(read_back.links[1], "https://docs.example.com/spec");
    }

    #[test]
    #[serial(env)]
    fn round_trip_parent() {
        let (_tmp, store, _tickets_dir) = make_store();
        let mut ticket = make_minimal_ticket("rt-p001");
        ticket.parent = Some("rt-epic1".to_string());
        store.write_ticket(&ticket).unwrap();
        let read_back = store.read_ticket(&ticket.id).unwrap();

        assert_eq!(read_back.parent, Some("rt-epic1".to_string()));
    }

    #[test]
    #[serial(env)]
    fn round_trip_empty_notes() {
        let (_tmp, store, _tickets_dir) = make_store();
        let ticket = make_minimal_ticket("rt-en01");
        store.write_ticket(&ticket).unwrap();
        let read_back = store.read_ticket(&ticket.id).unwrap();

        assert!(read_back.notes.is_empty());
    }

    #[test]
    #[serial(env)]
    fn round_trip_empty_links() {
        let (_tmp, store, _tickets_dir) = make_store();
        let ticket = make_minimal_ticket("rt-el01");
        store.write_ticket(&ticket).unwrap();
        let read_back = store.read_ticket(&ticket.id).unwrap();

        assert!(read_back.links.is_empty());
    }

    #[test]
    #[serial(env)]
    fn round_trip_all_optional_fields() {
        let (_tmp, store, _tickets_dir) = make_store();
        let mut ticket = make_minimal_ticket("rt-af01");
        ticket.notes = vec![Note {
            timestamp: "2026-04-02T12:00:00Z".to_string(),
            text: "progress update".to_string(),
        }];
        ticket.links = vec!["https://design.example.com".to_string()];
        ticket.parent = Some("rt-epic2".to_string());
        ticket.assignee = Some("bob".to_string());
        ticket.estimate = Some(120);
        ticket.description = Some("A detailed description".to_string());
        ticket.design = Some("Design doc".to_string());
        ticket.acceptance = Some("Must pass CI".to_string());
        ticket.tags = vec!["backend".to_string(), "urgent".to_string()];
        ticket.body = Some("Extended body content here.".to_string());

        store.write_ticket(&ticket).unwrap();
        let read_back = store.read_ticket(&ticket.id).unwrap();

        assert_eq!(read_back.notes.len(), 1);
        assert_eq!(read_back.notes[0].timestamp, "2026-04-02T12:00:00Z");
        assert_eq!(read_back.notes[0].text, "progress update");
        assert_eq!(read_back.links, vec!["https://design.example.com"]);
        assert_eq!(read_back.parent, Some("rt-epic2".to_string()));
        assert_eq!(read_back.assignee, Some("bob".to_string()));
        assert_eq!(read_back.estimate, Some(120));
        assert_eq!(
            read_back.description,
            Some("A detailed description".to_string())
        );
        assert_eq!(read_back.design, Some("Design doc".to_string()));
        assert_eq!(read_back.acceptance, Some("Must pass CI".to_string()));
        assert_eq!(read_back.tags, vec!["backend", "urgent"]);
        assert_eq!(
            read_back.body,
            Some("Extended body content here.".to_string())
        );
    }

    #[test]
    fn write_yaml_notes_serializes_empty() {
        let mut out = String::new();
        write_yaml_notes(&mut out, &[]);
        assert_eq!(out, "notes: []\n");
    }

    #[test]
    fn write_yaml_notes_serializes_single() {
        let mut out = String::new();
        let notes = vec![Note {
            timestamp: "2026-04-02T00:00:00Z".to_string(),
            text: "hello".to_string(),
        }];
        write_yaml_notes(&mut out, &notes);
        assert!(out.contains("notes:\n"));
        assert!(out.contains("timestamp:"));
        assert!(out.contains("text: hello"));
    }

    #[test]
    fn write_yaml_array_empty_produces_brackets() {
        let mut out = String::new();
        write_yaml_array(&mut out, "links", &[]);
        assert_eq!(out, "links: []\n");
    }

    #[test]
    fn write_yaml_array_with_items() {
        let mut out = String::new();
        let items = vec!["a".to_string(), "b".to_string()];
        write_yaml_array(&mut out, "tags", &items);
        assert_eq!(out, "tags: [a, b]\n");
    }

    // --- IO error and corrupt data tests ---

    #[test]
    #[serial(env)]
    fn read_ticket_corrupt_yaml_frontmatter() {
        let content = "---\nid: [invalid yaml\ntitle: broken\n---\n";
        let (_tmp, store, id) = store_with_ticket(content);
        let err = store.read_ticket(&id).unwrap_err();
        assert!(
            matches!(err, Error::Yaml(_)),
            "expected YamlError, got: {:?}",
            err
        );
    }

    #[test]
    #[serial(env)]
    fn read_ticket_no_yaml_frontmatter() {
        let content = "This is just plain text with no frontmatter at all.\n";
        let (_tmp, store, id) = store_with_ticket(content);
        let err = store.read_ticket(&id).unwrap_err();
        assert!(
            matches!(err, Error::Yaml(_)),
            "expected YamlError, got: {:?}",
            err
        );
    }

    #[test]
    #[serial(env)]
    fn read_ticket_truncated_yaml_missing_closing_dashes() {
        let content = "---\nid: test-abc\ntitle: Truncated\nstatus: open\n";
        let (_tmp, store, id) = store_with_ticket(content);
        let result = store.read_ticket(&id);
        assert!(
            result.is_err(),
            "expected an error for truncated YAML, got: {:?}",
            result
        );
    }

    #[test]
    #[serial(env)]
    fn read_ticket_valid_yaml_missing_required_fields() {
        let content = "---\nid: test-abc\npriority: 2\n---\n";
        let (_tmp, store, id) = store_with_ticket(content);
        let err = store.read_ticket(&id).unwrap_err();
        assert!(
            matches!(err, Error::Yaml(_)),
            "expected YamlError for missing required fields, got: {:?}",
            err
        );
    }

    #[test]
    #[serial(env)]
    fn read_ticket_invalid_enum_value() {
        let content = r#"---
id: test-abc
title: Bad Status
status: banana
type: task
priority: 2
created: "2026-04-02T00:00:00Z"
---
"#;
        let (_tmp, store, id) = store_with_ticket(content);
        let err = store.read_ticket(&id).unwrap_err();
        assert!(
            matches!(err, Error::Yaml(_)),
            "expected YamlError for invalid enum, got: {:?}",
            err
        );
    }

    #[test]
    #[serial(env)]
    fn read_ticket_nonexistent_file_returns_io_error() {
        let (_tmp, store, _tickets_dir) = make_store();
        let err = store.read_ticket("does-not-exist").unwrap_err();
        assert!(
            matches!(err, Error::Io(_)),
            "expected IoError for missing file, got: {:?}",
            err
        );
    }

    #[test]
    #[serial(env)]
    fn read_all_skips_corrupt_keeps_valid() {
        let (_tmp, store, tickets_dir) = make_store();
        fs::write(tickets_dir.join("good-aaaa.md"), VALID_TICKET).unwrap();
        fs::write(
            tickets_dir.join("bad-bbbb.md"),
            "---\nid: [broken yaml\n---\n",
        )
        .unwrap();
        fs::write(tickets_dir.join("bad-cccc.md"), "---\nid: bad-cccc\n---\n").unwrap();
        fs::write(
            tickets_dir.join("bad-dddd.md"),
            "Just plain text, no frontmatter.\n",
        )
        .unwrap();

        let tickets = store.read_all().unwrap();
        assert_eq!(tickets.len(), 1);
        assert_eq!(tickets[0].id, "test-abc");
    }

    #[test]
    #[serial(env)]
    fn write_ticket_readonly_directory_fails() {
        use std::os::unix::fs::PermissionsExt;

        let (_tmp, store, tickets_dir) = make_store();
        let ticket = make_full_ticket();

        fs::set_permissions(&tickets_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let result = store.write_ticket(&ticket);
        assert!(
            result.is_err(),
            "expected error writing to read-only directory"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, Error::Io(_)),
            "expected IoError for permission denied, got: {:?}",
            err
        );

        fs::set_permissions(&tickets_dir, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    #[serial(env)]
    fn read_ticket_invalid_utf8() {
        let (_tmp, store, tickets_dir) = make_store();
        let id = "bad-utf8";
        let path = tickets_dir.join(format!("{}.md", id));
        let invalid_bytes: Vec<u8> = vec![
            0x2d, 0x2d, 0x2d, 0x0a, // ---\n
            0x69, 0x64, 0x3a, 0x20, // id:
            0xff, 0xfe, 0x0a, // invalid UTF-8 bytes + newline
            0x2d, 0x2d, 0x2d, 0x0a, // ---\n
        ];
        fs::write(&path, invalid_bytes).unwrap();

        let err = store.read_ticket(id).unwrap_err();
        assert!(
            matches!(err, Error::Io(_)),
            "expected IoError for invalid UTF-8, got: {:?}",
            err
        );
    }

    // ── add_dep tests ───────────────────────────────────────────────────────

    fn write_minimal_ticket(tickets_dir: &Path, id: &str) {
        let content = format!(
            "---\nid: {id}\ntitle: Ticket {id}\nstatus: open\ntype: task\npriority: 2\ntags: []\ncreated: \"2025-01-01T00:00:00Z\"\n---\n"
        );
        fs::write(tickets_dir.join(format!("{id}.md")), content).unwrap();
    }

    #[test]
    #[serial(env)]
    fn add_dep_appends_dep_to_ticket() {
        let (_tmp, store, tickets_dir) = make_store();
        write_minimal_ticket(&tickets_dir, "ad-a");
        write_minimal_ticket(&tickets_dir, "ad-b");

        store.add_dep("ad-a", "ad-b").unwrap();

        let ticket = store.read_ticket("ad-a").unwrap();
        assert!(ticket.deps.contains(&"ad-b".to_string()));
    }

    #[test]
    #[serial(env)]
    fn add_dep_idempotent_no_duplicate() {
        let (_tmp, store, tickets_dir) = make_store();
        write_minimal_ticket(&tickets_dir, "ad-c");
        write_minimal_ticket(&tickets_dir, "ad-d");

        store.add_dep("ad-c", "ad-d").unwrap();
        store.add_dep("ad-c", "ad-d").unwrap();

        let ticket = store.read_ticket("ad-c").unwrap();
        let count = ticket.deps.iter().filter(|d| *d == "ad-d").count();
        assert_eq!(count, 1, "dep should appear exactly once");
    }

    #[test]
    #[serial(env)]
    fn add_dep_nonexistent_ticket_returns_error() {
        let (_tmp, store, tickets_dir) = make_store();
        write_minimal_ticket(&tickets_dir, "ad-e");

        let result = store.add_dep("no-such", "ad-e");
        assert!(result.is_err());
    }

    #[test]
    #[serial(env)]
    fn add_dep_multiple_deps() {
        let (_tmp, store, tickets_dir) = make_store();
        write_minimal_ticket(&tickets_dir, "ad-f");
        write_minimal_ticket(&tickets_dir, "ad-g");
        write_minimal_ticket(&tickets_dir, "ad-h");

        store.add_dep("ad-f", "ad-g").unwrap();
        store.add_dep("ad-f", "ad-h").unwrap();

        let ticket = store.read_ticket("ad-f").unwrap();
        assert_eq!(ticket.deps.len(), 2);
        assert!(ticket.deps.contains(&"ad-g".to_string()));
        assert!(ticket.deps.contains(&"ad-h".to_string()));
    }

    // ── version / optimistic concurrency tests ────────────────────────────

    #[test]
    #[serial(env)]
    fn write_ticket_adds_version_field() {
        let (_tmp, store, _tickets_dir) = make_store();
        let ticket = make_minimal_ticket("ver-001");
        store.write_ticket(&ticket).unwrap();
        let read_back = store.read_ticket("ver-001").unwrap();
        assert!(
            read_back.version.is_some(),
            "version should be set after write"
        );
        let v = read_back.version.unwrap();
        assert_eq!(v.len(), 16, "version should be 16 hex chars, got: {v}");
        assert!(
            v.chars().all(|c| c.is_ascii_hexdigit()),
            "version should be hex, got: {v}"
        );
    }

    #[test]
    #[serial(env)]
    fn write_ticket_version_changes_on_content_change() {
        let (_tmp, store, _tickets_dir) = make_store();
        let mut ticket = make_minimal_ticket("ver-002");
        store.write_ticket(&ticket).unwrap();
        let v1 = store.read_ticket("ver-002").unwrap().version.unwrap();

        ticket.title = "Changed title".to_string();
        // Update version to match on-disk so the write succeeds
        ticket.version = Some(v1.clone());
        store.write_ticket(&ticket).unwrap();
        let v2 = store.read_ticket("ver-002").unwrap().version.unwrap();

        assert_ne!(v1, v2, "version should change when content changes");
    }

    #[test]
    #[serial(env)]
    fn write_ticket_version_stable_for_same_content() {
        let (_tmp, store, _tickets_dir) = make_store();
        let ticket = make_minimal_ticket("ver-003");
        store.write_ticket(&ticket).unwrap();
        let v1 = store.read_ticket("ver-003").unwrap().version.unwrap();

        // Write the exact same content again (with matching version)
        let ticket2 = store.read_ticket("ver-003").unwrap();
        store.write_ticket(&ticket2).unwrap();
        let v2 = store.read_ticket("ver-003").unwrap().version.unwrap();

        assert_eq!(v1, v2, "version should be stable for identical content");
    }

    #[test]
    #[serial(env)]
    fn write_ticket_stale_version_returns_error() {
        let (_tmp, store, _tickets_dir) = make_store();
        let ticket = make_minimal_ticket("ver-004");
        store.write_ticket(&ticket).unwrap();

        // Simulate two agents: both read the same ticket
        let mut agent_a = store.read_ticket("ver-004").unwrap();
        let mut agent_b = store.read_ticket("ver-004").unwrap();

        // Agent A writes first — succeeds (version now changes on disk)
        agent_a.title = "Agent A change".to_string();
        store.write_ticket(&agent_a).unwrap();

        // Agent B tries to write with the old version — should fail
        agent_b.title = "Agent B change".to_string();
        let err = store.write_ticket(&agent_b).unwrap_err();
        assert!(
            matches!(err, Error::Stale { .. }),
            "expected Stale error, got: {:?}",
            err
        );
        assert_eq!(err.exit_code(), 5);
    }

    #[test]
    #[serial(env)]
    fn write_ticket_legacy_ticket_without_version_migrates() {
        let (_tmp, store, tickets_dir) = make_store();
        // Simulate a ticket written by old vima (no version field)
        let content = r#"---
id: ver-005
title: Legacy ticket
status: open
type: task
priority: 2
tags: []
assignee: null
estimate: null
deps: []
links: []
parent: null
created: "2026-04-02T00:00:00Z"
notes: []
---
"#;
        fs::write(tickets_dir.join("ver-005.md"), content).unwrap();
        let mut ticket = store.read_ticket("ver-005").unwrap();
        assert!(
            ticket.version.is_none(),
            "legacy ticket should have no version"
        );

        // Update it — should succeed and add a version
        ticket.title = "Updated legacy".to_string();
        store.write_ticket(&ticket).unwrap();
        let updated = store.read_ticket("ver-005").unwrap();
        assert!(
            updated.version.is_some(),
            "version should be added on first write"
        );
    }

    #[test]
    #[serial(env)]
    fn write_ticket_new_ticket_skips_version_check() {
        let (_tmp, store, _tickets_dir) = make_store();
        // Brand new ticket (file doesn't exist) — should never return stale
        let ticket = make_minimal_ticket("ver-006");
        store.write_ticket(&ticket).unwrap();
        let read_back = store.read_ticket("ver-006").unwrap();
        assert!(read_back.version.is_some());
    }

    #[test]
    #[serial(env)]
    fn write_ticket_round_trip_preserves_version() {
        let (_tmp, store, _tickets_dir) = make_store();
        let ticket = make_minimal_ticket("ver-007");
        store.write_ticket(&ticket).unwrap();
        let read1 = store.read_ticket("ver-007").unwrap();

        // Write it back unchanged — version should remain the same
        store.write_ticket(&read1).unwrap();
        let read2 = store.read_ticket("ver-007").unwrap();
        assert_eq!(read1.version, read2.version);
    }

    #[test]
    #[serial(env)]
    fn write_ticket_version_in_file_content() {
        let (_tmp, store, tickets_dir) = make_store();
        let ticket = make_minimal_ticket("ver-008");
        store.write_ticket(&ticket).unwrap();

        let content = fs::read_to_string(tickets_dir.join("ver-008.md")).unwrap();
        assert!(
            content.contains("version: "),
            "file should contain version field"
        );
    }

    // ── direct unit tests for version helpers ────────────────────────────

    #[test]
    fn compute_version_deterministic() {
        let input = "some ticket content";
        let v1 = compute_version(input);
        let v2 = compute_version(input);
        assert_eq!(v1, v2, "same input must produce same version");
        assert_eq!(v1.len(), 16);
        assert!(v1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn compute_version_different_for_different_input() {
        let v1 = compute_version("content A");
        let v2 = compute_version("content B");
        assert_ne!(v1, v2, "different input must produce different version");
    }

    #[test]
    fn extract_version_from_yaml_present() {
        let content = "---\nid: vi-0001\nversion: abcdef0123456789\ntitle: Test\n---\n";
        let v = extract_version_from_yaml(content);
        assert_eq!(v, Some("abcdef0123456789".to_string()));
    }

    #[test]
    fn extract_version_from_yaml_absent() {
        let content = "---\nid: vi-0001\ntitle: Test\n---\n";
        let v = extract_version_from_yaml(content);
        assert_eq!(v, None);
    }

    #[test]
    fn extract_version_from_yaml_quoted() {
        let content = "---\nid: vi-0001\nversion: \"abcdef0123456789\"\ntitle: Test\n---\n";
        let v = extract_version_from_yaml(content);
        assert_eq!(v, Some("abcdef0123456789".to_string()));
    }

    #[test]
    fn insert_version_line_places_after_id() {
        let content = "---\nid: vi-0001\ntitle: Test\nstatus: open\n---\n";
        let result = insert_version_line(content, "abcdef0123456789");
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "---");
        assert_eq!(lines[1], "id: vi-0001");
        assert_eq!(lines[2], "version: \"abcdef0123456789\"");
        assert_eq!(lines[3], "title: Test");
    }

    // ── write_ticket version match succeeds ──────────────────────────────

    #[test]
    #[serial(env)]
    fn write_ticket_succeeds_when_version_matches() {
        let (_tmp, store, _tickets_dir) = make_store();
        let ticket = make_minimal_ticket("ver-009");
        store.write_ticket(&ticket).unwrap();

        // Read back (gets current version), modify, write — should succeed
        let mut t = store.read_ticket("ver-009").unwrap();
        assert!(t.version.is_some());
        t.title = "Updated title".to_string();
        let result = store.write_ticket(&t);
        assert!(
            result.is_ok(),
            "write with matching version should succeed: {:?}",
            result
        );

        // Verify the version changed
        let t2 = store.read_ticket("ver-009").unwrap();
        assert_ne!(
            t.version, t2.version,
            "version should change after content change"
        );
    }

    // ── round-trip with modification ─────────────────────────────────────

    #[test]
    #[serial(env)]
    fn write_ticket_round_trip_modify_refreshes_version() {
        let (_tmp, store, _tickets_dir) = make_store();
        let ticket = make_minimal_ticket("ver-010");
        store.write_ticket(&ticket).unwrap();
        let v1 = store.read_ticket("ver-010").unwrap().version.unwrap();

        // Read → modify → write → read: version should change
        let mut t = store.read_ticket("ver-010").unwrap();
        t.title = "Modified".to_string();
        store.write_ticket(&t).unwrap();
        let v2 = store.read_ticket("ver-010").unwrap().version.unwrap();
        assert_ne!(v1, v2);

        // Read → modify again → write → read: version should change again
        let mut t2 = store.read_ticket("ver-010").unwrap();
        t2.title = "Modified again".to_string();
        store.write_ticket(&t2).unwrap();
        let v3 = store.read_ticket("ver-010").unwrap().version.unwrap();
        assert_ne!(v2, v3);
    }

    // ── File locking tests ────────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn lock_exclusive_creates_lock_file() {
        let tmp = make_temp();
        let vima_dir = tmp.path().join(".vima");
        fs::create_dir_all(vima_dir.join("tickets")).unwrap();
        unsafe { std::env::set_var("VIMA_DIR", &vima_dir) };

        let store = Store::open().unwrap();
        let _guard = store.lock_exclusive().unwrap();

        assert!(vima_dir.join("lock").exists());

        unsafe { std::env::remove_var("VIMA_DIR") };
    }

    #[test]
    #[serial(env)]
    fn lock_shared_creates_lock_file() {
        let tmp = make_temp();
        let vima_dir = tmp.path().join(".vima");
        fs::create_dir_all(vima_dir.join("tickets")).unwrap();
        unsafe { std::env::set_var("VIMA_DIR", &vima_dir) };

        let store = Store::open().unwrap();
        let _guard = store.lock_shared().unwrap();

        assert!(vima_dir.join("lock").exists());

        unsafe { std::env::remove_var("VIMA_DIR") };
    }

    #[test]
    #[serial(env)]
    fn multiple_shared_locks_do_not_block() {
        let tmp = make_temp();
        let vima_dir = tmp.path().join(".vima");
        fs::create_dir_all(vima_dir.join("tickets")).unwrap();
        unsafe { std::env::set_var("VIMA_DIR", &vima_dir) };

        let store = Store::open().unwrap();
        let _guard1 = store.lock_shared().unwrap();
        let _guard2 = store.lock_shared().unwrap();
        // Both acquired without blocking

        unsafe { std::env::remove_var("VIMA_DIR") };
    }

    #[test]
    #[serial(env)]
    fn lock_released_on_guard_drop() {
        let tmp = make_temp();
        let vima_dir = tmp.path().join(".vima");
        fs::create_dir_all(vima_dir.join("tickets")).unwrap();
        unsafe { std::env::set_var("VIMA_DIR", &vima_dir) };

        let store = Store::open().unwrap();
        {
            let _guard = store.lock_exclusive().unwrap();
            // Lock held here
        }
        // Lock released — we should be able to acquire again
        let _guard2 = store.lock_exclusive().unwrap();

        unsafe { std::env::remove_var("VIMA_DIR") };
    }

    #[test]
    #[serial(env)]
    fn exclusive_lock_blocks_second_exclusive() {
        use std::sync::{Arc, Barrier};

        let tmp = make_temp();
        let vima_dir = tmp.path().join(".vima");
        fs::create_dir_all(vima_dir.join("tickets")).unwrap();

        let lock_path = vima_dir.join("lock");
        let lock_path_clone = lock_path.clone();

        let barrier = Arc::new(Barrier::new(2));
        let barrier_clone = barrier.clone();

        // Thread 1: acquire exclusive lock, hold it until barrier
        let t1 = std::thread::spawn(move || {
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&lock_path_clone)
                .unwrap();
            file.lock_exclusive().unwrap();
            barrier_clone.wait(); // signal thread 2 that lock is held
            std::thread::sleep(std::time::Duration::from_millis(100));
            // Lock released on drop
        });

        // Thread 2: wait for thread 1 to acquire, then try non-blocking
        barrier.wait();
        let file2 = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        let result = file2.try_lock_exclusive();
        // Should fail because thread 1 holds the lock
        assert!(
            result.is_err(),
            "second exclusive lock should fail while first is held"
        );

        t1.join().unwrap();
        // Now it should succeed
        file2.lock_exclusive().unwrap();
    }
}
