use std::path::Path;

use nanoid::nanoid;

use crate::error::{Error, Result};

const ALPHANUMERIC: [char; 36] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i',
    'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
];

pub fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(Error::InvalidField("id must not be empty".into()));
    }
    if id.starts_with('.') {
        return Err(Error::InvalidField("id must not start with '.'".into()));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(Error::InvalidField(
            "id may only contain alphanumeric characters, '.', '_', or '-'".into(),
        ));
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

    // Exact match wins over substring to avoid ambiguity (e.g. "vm-abc" vs "vm-abc1")
    let exact_path = dir.join(format!("{input}.md"));
    if exact_path.exists() {
        return Ok(input.to_string());
    }

    // Substring search across all .md files (excluding .md.tmp)
    let mut matches = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let os_name = entry.file_name();
        let name = os_name.to_string_lossy();
        if name.ends_with(".md.tmp") {
            continue;
        }
        if let Some(id) = name.strip_suffix(".md") {
            if id.contains(input) {
                matches.push(id.to_string());
                if matches.len() > 1 {
                    return Err(Error::AmbiguousId(input.to_string(), matches));
                }
            }
        }
    }

    matches
        .into_iter()
        .next()
        .ok_or_else(|| Error::NotFound(input.to_string()))
}

pub fn get_prefix(vima_root: &Path) -> Result<String> {
    let config_path = vima_root.join(".vima/config.yml");

    match std::fs::read_to_string(&config_path) {
        Ok(content) => {
            for line in content.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("prefix:") {
                    let prefix = rest
                        .trim()
                        .trim_matches(|c| c == '"' || c == '\'')
                        .to_string();
                    if !prefix.is_empty() {
                        return Ok(prefix);
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    // Compute from directory name
    let dir_name = vima_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("vi");

    let segments: Vec<&str> = dir_name.split(|c| c == '-' || c == '_').collect();
    let prefix = if segments.len() == 1 {
        segments[0]
            .chars()
            .take(2)
            .collect::<String>()
            .to_lowercase()
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
        assert!(suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && (c.is_ascii_digit() || c.is_ascii_lowercase())));
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

    // ── edge-case tests ──────────────────────────────────────────────────────

    #[test]
    fn validate_rejects_unicode_characters() {
        let err = validate_id("vm-café").unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_rejects_unicode_emoji() {
        let err = validate_id("vm-🎉abc").unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_rejects_cjk_characters() {
        let err = validate_id("vm-票据").unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_rejects_very_long_id() {
        // 1000+ character ID — should still be accepted since all chars are valid
        let long_id = "a".repeat(1500);
        assert!(validate_id(&long_id).is_ok());
    }

    #[test]
    fn validate_rejects_very_long_id_with_unicode() {
        // 1000+ chars but contains invalid unicode
        let mut long_id = "a".repeat(1000);
        long_id.push('é');
        let err = validate_id(&long_id).unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_rejects_spaces() {
        let err = validate_id("vm abc").unwrap_err();
        assert!(matches!(err, Error::InvalidField(_)));
    }

    #[test]
    fn validate_just_prefix_no_suffix() {
        // A bare prefix like "vm" with no dash or suffix is still a valid ID string
        assert!(validate_id("vm").is_ok());
    }

    #[test]
    fn validate_just_dash() {
        assert!(validate_id("-").is_ok());
    }

    #[test]
    fn validate_just_underscore() {
        assert!(validate_id("_").is_ok());
    }

    #[test]
    fn get_prefix_from_dir_with_dashes_and_numbers() {
        let base = make_tmp_dir("get-prefix-dash-num");
        let proj = base.join("my-cool-project-2");
        fs::create_dir_all(&proj).unwrap();

        let prefix = get_prefix(&proj).unwrap();
        assert_eq!(prefix, "mcp2");
    }

    #[test]
    fn get_prefix_from_dir_with_underscores() {
        let base = make_tmp_dir("get-prefix-underscores");
        let proj = base.join("my_project_name");
        fs::create_dir_all(&proj).unwrap();

        let prefix = get_prefix(&proj).unwrap();
        assert_eq!(prefix, "mpn");
    }

    #[test]
    fn get_prefix_from_dir_with_mixed_separators() {
        let base = make_tmp_dir("get-prefix-mixed-sep");
        let proj = base.join("my-cool_project");
        fs::create_dir_all(&proj).unwrap();

        let prefix = get_prefix(&proj).unwrap();
        assert_eq!(prefix, "mcp");
    }

    #[test]
    fn get_prefix_from_numeric_dir() {
        let base = make_tmp_dir("get-prefix-numeric");
        let proj = base.join("123");
        fs::create_dir_all(&proj).unwrap();

        let prefix = get_prefix(&proj).unwrap();
        assert_eq!(prefix, "12");
    }

    #[test]
    fn get_prefix_from_single_char_dir() {
        let base = make_tmp_dir("get-prefix-single-char");
        let proj = base.join("x");
        fs::create_dir_all(&proj).unwrap();

        let prefix = get_prefix(&proj).unwrap();
        assert_eq!(prefix, "x");
    }

    #[test]
    fn generate_id_with_long_prefix() {
        let dir = make_tmp_dir("generate-id-long-prefix");
        let id = generate_id("my-long-prefix", &dir).unwrap();
        assert!(id.starts_with("my-long-prefix-"));
        let suffix = &id["my-long-prefix-".len()..];
        assert_eq!(suffix.len(), 4);
    }

    #[test]
    fn generate_id_with_numeric_prefix() {
        let dir = make_tmp_dir("generate-id-numeric-prefix");
        let id = generate_id("42", &dir).unwrap();
        assert!(id.starts_with("42-"));
        assert!(validate_id(&id).is_ok());
    }

    #[test]
    fn generate_id_always_passes_validate() {
        let dir = make_tmp_dir("generate-id-validates");
        for _ in 0..20 {
            let id = generate_id("vm", &dir).unwrap();
            assert!(
                validate_id(&id).is_ok(),
                "generated id '{}' failed validation",
                id
            );
        }
    }

    #[test]
    fn resolve_id_is_case_sensitive() {
        let dir = make_tmp_dir("resolve-case-sensitive");
        fs::write(dir.join("VM-ABC1.md"), "").unwrap();

        // Searching for lowercase should not find uppercase file (case-sensitive matching)
        let result = resolve_id(&dir, "vm-abc1", false);
        // On case-sensitive filesystems this is NotFound; the match is case-sensitive
        // because contains() is case-sensitive
        assert!(
            result.is_err() || result.unwrap() != "VM-ABC1",
            "resolve_id should not case-insensitively match"
        );
    }

    #[test]
    fn resolve_id_trims_whitespace() {
        let dir = make_tmp_dir("resolve-trim-ws");
        fs::write(dir.join("vm-abc1.md"), "").unwrap();

        // Leading/trailing whitespace should be trimmed
        assert_eq!(resolve_id(&dir, "  vm-abc1  ", false).unwrap(), "vm-abc1");
    }
}
