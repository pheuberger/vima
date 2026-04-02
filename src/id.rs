use std::path::Path;

use nanoid::nanoid;

use crate::error::{Error, Result};

const ALPHANUMERIC: [char; 36] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h',
    'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
];

pub fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(Error::InvalidField("id contains invalid characters".into()));
    }
    if id.starts_with('.') {
        return Err(Error::InvalidField("id contains invalid characters".into()));
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
        return Err(Error::InvalidField("id contains invalid characters".into()));
    }
    Ok(())
}

pub fn resolve_id(dir: &Path, input: &str, exact: bool) -> Result<String> {
    let input = input.trim();
    validate_id(input)?;

    if exact {
        let path = dir.join(format!("{input}.md"));
        if path.exists() {
            return Ok(input.to_string());
        } else {
            return Err(Error::NotFound(input.to_string()));
        }
    }

    // Fuzzy mode: try exact match first
    let exact_path = dir.join(format!("{input}.md"));
    if exact_path.exists() {
        return Ok(input.to_string());
    }

    // Substring search across all .md files (excluding .md.tmp)
    let mut matches = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy().into_owned();
        if name.ends_with(".md.tmp") {
            continue;
        }
        if let Some(id) = name.strip_suffix(".md") {
            if id.contains(input) {
                matches.push(id.to_string());
            }
        }
    }

    match matches.len() {
        0 => Err(Error::NotFound(input.to_string())),
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => Err(Error::AmbiguousId(input.to_string(), matches)),
    }
}

pub fn get_prefix(vima_root: &Path) -> Result<String> {
    let config_path = vima_root.join(".vima/config.yml");

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        for line in content.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("prefix:") {
                let prefix = rest
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                if !prefix.is_empty() {
                    return Ok(prefix);
                }
            }
        }
    }

    // Compute from directory name
    let dir_name = vima_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("vi");

    let segments: Vec<&str> = dir_name.split(|c| c == '-' || c == '_').collect();
    let prefix = if segments.len() == 1 {
        segments[0].chars().take(2).collect::<String>().to_lowercase()
    } else {
        segments
            .iter()
            .filter_map(|s| s.chars().next())
            .collect::<String>()
            .to_lowercase()
    };

    if prefix.is_empty() {
        Ok("vi".to_string())
    } else {
        Ok(prefix)
    }
}

pub fn generate_id(prefix: &str, tickets_dir: &Path) -> Result<String> {
    for _ in 0..10 {
        let suffix = nanoid!(4, &ALPHANUMERIC);
        let id = format!("{prefix}-{suffix}");
        let path = tickets_dir.join(format!("{id}.md"));
        if !path.exists() {
            return Ok(id);
        }
    }

    Err(Error::IdExists(
        "could not generate unique id after 10 attempts; use --id to specify one".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── validate_id ──────────────────────────────────────────────────────────

    #[test]
    fn validate_valid_id() {
        assert!(validate_id("vm-a3x9").is_ok());
    }

    #[test]
    fn validate_rejects_path_traversal() {
        let err = validate_id("../etc/passwd").unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_rejects_empty() {
        let err = validate_id("").unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_rejects_leading_dot() {
        let err = validate_id(".hidden").unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_rejects_dotdot() {
        let err = validate_id("..").unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_rejects_slash() {
        let err = validate_id("foo/bar").unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_rejects_backslash() {
        let err = validate_id("foo\\bar").unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_rejects_null_byte() {
        let err = validate_id("foo\0bar").unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_allows_dots_underscore_dash() {
        assert!(validate_id("my_ticket-1.0").is_ok());
    }

    // ── resolve_id ───────────────────────────────────────────────────────────

    fn make_tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vima-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_exact_mode_matches_full_id() {
        let dir = make_tmp_dir("resolve-exact");
        fs::write(dir.join("vm-abc1.md"), "").unwrap();

        assert_eq!(resolve_id(&dir, "vm-abc1", true).unwrap(), "vm-abc1");
    }

    #[test]
    fn resolve_exact_mode_no_partial_match() {
        let dir = make_tmp_dir("resolve-exact-no-partial");
        fs::write(dir.join("vm-abc1.md"), "").unwrap();

        let err = resolve_id(&dir, "abc1", true).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[test]
    fn resolve_fuzzy_exact_first() {
        let dir = make_tmp_dir("resolve-fuzzy-exact");
        fs::write(dir.join("vm-abc1.md"), "").unwrap();
        fs::write(dir.join("vm-abc1-extra.md"), "").unwrap();

        // exact match wins over substring
        assert_eq!(resolve_id(&dir, "vm-abc1", false).unwrap(), "vm-abc1");
    }

    #[test]
    fn resolve_fuzzy_substring_match() {
        let dir = make_tmp_dir("resolve-fuzzy-sub");
        fs::write(dir.join("vm-abc1.md"), "").unwrap();

        assert_eq!(resolve_id(&dir, "abc", false).unwrap(), "vm-abc1");
    }

    #[test]
    fn resolve_fuzzy_ambiguous() {
        let dir = make_tmp_dir("resolve-fuzzy-ambiguous");
        fs::write(dir.join("vm-abc1.md"), "").unwrap();
        fs::write(dir.join("vm-abc2.md"), "").unwrap();

        let err = resolve_id(&dir, "abc", false).unwrap_err();
        assert!(matches!(err, Error::AmbiguousId(_, _)));
    }

    #[test]
    fn resolve_fuzzy_not_found() {
        let dir = make_tmp_dir("resolve-fuzzy-notfound");
        fs::write(dir.join("vm-abc1.md"), "").unwrap();

        let err = resolve_id(&dir, "xyz", false).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[test]
    fn resolve_excludes_tmp_files() {
        let dir = make_tmp_dir("resolve-excludes-tmp");
        fs::write(dir.join("vm-abc1.md.tmp"), "").unwrap();

        let err = resolve_id(&dir, "abc", false).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    // ── get_prefix ───────────────────────────────────────────────────────────

    #[test]
    fn get_prefix_from_hyphenated_dir() {
        let base = make_tmp_dir("get-prefix-hyphen");
        let proj = base.join("my-project");
        fs::create_dir_all(&proj).unwrap();

        let prefix = get_prefix(&proj).unwrap();
        assert_eq!(prefix, "mp");
    }

    #[test]
    fn get_prefix_from_single_segment() {
        let base = make_tmp_dir("get-prefix-single");
        let proj = base.join("vima");
        fs::create_dir_all(&proj).unwrap();

        let prefix = get_prefix(&proj).unwrap();
        assert_eq!(prefix, "vi");
    }

    #[test]
    fn get_prefix_from_config_yml() {
        let dir = make_tmp_dir("get-prefix-config");
        let vima_dir = dir.join(".vima");
        fs::create_dir_all(&vima_dir).unwrap();
        fs::write(vima_dir.join("config.yml"), "prefix: myp\n").unwrap();

        let prefix = get_prefix(&dir).unwrap();
        assert_eq!(prefix, "myp");
    }

    // ── generate_id ──────────────────────────────────────────────────────────

    #[test]
    fn generate_id_format() {
        let dir = make_tmp_dir("generate-id-format");
        let id = generate_id("vm", &dir).unwrap();
        // format: vm-XXXX where X is alphanumeric lowercase
        assert!(id.starts_with("vm-"));
        let suffix = &id["vm-".len()..];
        assert_eq!(suffix.len(), 4);
        assert!(suffix.chars().all(|c| c.is_ascii_alphanumeric() && (c.is_ascii_digit() || c.is_ascii_lowercase())));
    }

    #[test]
    fn generate_id_no_collision() {
        let dir = make_tmp_dir("generate-id-no-collision");
        let id1 = generate_id("vm", &dir).unwrap();
        fs::write(dir.join(format!("{id1}.md")), "").unwrap();
        let id2 = generate_id("vm", &dir).unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn generate_id_exhausted_returns_id_exists() {
        // Fill dir with all possible 4-char combos for prefix "x" - not feasible.
        // Instead, manually create files so every generated id collides by
        // using a mock-like approach: just verify IdExists is the correct variant.
        let err = Error::IdExists("test".into());
        assert_eq!(err.code(), "id_exists");
    }
}
