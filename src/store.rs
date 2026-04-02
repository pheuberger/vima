use std::fs;
use std::path::{Path, PathBuf};

use gray_matter::engine::YAML;
use gray_matter::Matter;

use crate::error::{Error, Result};
use crate::ticket::Ticket;

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
        // Remove VIMA_DIR if set
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
}
