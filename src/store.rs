use std::fs;
use std::path::{Path, PathBuf};

use gray_matter::engine::YAML;
use gray_matter::Matter;

use crate::error::{Error, Result};
use crate::id;
use crate::ticket::{Note, Ticket};

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
    if s.contains(':') || s.contains('#') || s.contains('\n') || s.contains('"') || s.contains('\'') || s.contains(',') || s.contains(']') {
        return true;
    }
    if s.len() <= 5 {
        let lower = s.to_lowercase();
        if matches!(lower.as_str(), "true" | "false" | "null" | "yes" | "no" | "on" | "off") {
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
        out.push_str(&format!("  - timestamp: {}\n", yaml_scalar(&note.timestamp)));
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
            return path.canonicalize().map_err(Error::IoError);
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
            return candidate.canonicalize().map_err(Error::IoError);
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

impl Store {
    pub fn open() -> Result<Self> {
        let root = find_vima_root()?;
        let tickets = root.join("tickets");
        fs::create_dir_all(&tickets)?;
        Ok(Store { root, tickets })
    }

    pub fn read_ticket(&self, id: &str) -> Result<Ticket> {
        let path = self.tickets.join(format!("{}.md", id));
        let contents = fs::read_to_string(&path)?;

        let parsed = Matter::<YAML>::new()
            .parse::<Ticket>(&contents)
            .map_err(|e| Error::YamlError(e.to_string()))?;

        let mut ticket = parsed
            .data
            .ok_or_else(|| Error::YamlError(format!("missing frontmatter in {}.md", id)))?;

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
                    let _ = writeln!(
                        std::io::stderr(),
                        "warning: skipping {}: {}",
                        name,
                        e
                    );
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

        out.push_str(&format!("created: {}\n", yaml_scalar(&strip(&ticket.created))));

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

        let tmp_path = self.tickets.join(format!("{}.md.tmp", ticket.id));
        let final_path = self.tickets.join(format!("{}.md", ticket.id));
        fs::write(&tmp_path, &out)?;
        fs::rename(&tmp_path, &final_path)?;

        Ok(())
    }

    pub fn resolve_id(&self, input: &str, exact: bool) -> Result<String> {
        id::resolve_id(&self.tickets, input, exact)
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
        fs::write(tickets_dir.join("bad.md"), "not frontmatter at all\njust text").unwrap();
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
}
