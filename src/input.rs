use std::io::Read;

use crate::error::{Error, Result};

/// Resolves long-form flag values that support `@PATH` (file substitution) and
/// `-` (stdin) sentinels. Tracks whether stdin has been consumed so that a
/// single invocation cannot read it twice.
#[derive(Default)]
pub struct InputResolver {
    stdin_consumed: bool,
}

impl InputResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a single flag value.
    ///
    /// - `"-"`           → read from stdin (only once per invocation)
    /// - `"@@..."`       → literal `"@..."` (escape)
    /// - `"@path"`       → file contents at `path`
    /// - anything else   → returned unchanged
    pub fn resolve(&mut self, value: String) -> Result<String> {
        // Match sentinels on the trimmed view so stray whitespace from shell
        // substitutions / heredoc continuations (e.g. `"- "`, `" -"`) still
        // resolve as stdin/file. Plain values pass through untouched so a
        // legitimate description like `"  hello  "` keeps its whitespace.
        let trimmed = value.trim();
        if trimmed == "-" {
            if self.stdin_consumed {
                return Err(Error::InvalidArgument(
                    "only one flag may read from stdin (-) per invocation; \
                     pass every field as one object via `--json -`, \
                     or read the other fields from files with @PATH"
                        .into(),
                ));
            }
            self.stdin_consumed = true;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            return Ok(buf);
        }
        if let Some(rest) = trimmed.strip_prefix("@@") {
            return Ok(format!("@{rest}"));
        }
        if let Some(path) = trimmed.strip_prefix('@') {
            return std::fs::read_to_string(path).map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => Error::FileNotFound(path.to_string()),
                _ => Error::Io(e),
            });
        }
        Ok(value)
    }

    pub fn resolve_opt(&mut self, value: Option<String>) -> Result<Option<String>> {
        match value {
            Some(v) => Ok(Some(self.resolve(v)?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn plain_string_passthrough() {
        let mut r = InputResolver::new();
        assert_eq!(r.resolve("hello world".into()).unwrap(), "hello world");
    }

    #[test]
    fn empty_string_passthrough() {
        let mut r = InputResolver::new();
        assert_eq!(r.resolve(String::new()).unwrap(), "");
    }

    #[test]
    fn double_at_unescapes_to_single_at() {
        let mut r = InputResolver::new();
        assert_eq!(r.resolve("@@literal".into()).unwrap(), "@literal");
        assert_eq!(r.resolve("@@".into()).unwrap(), "@");
        assert_eq!(r.resolve("@@/path".into()).unwrap(), "@/path");
    }

    #[test]
    fn at_path_reads_file_contents() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "## Heading\nbody with `backticks` and \"quotes\"\n").unwrap();
        let path = tmp.path().to_str().unwrap();

        let mut r = InputResolver::new();
        let out = r.resolve(format!("@{path}")).unwrap();
        assert_eq!(out, "## Heading\nbody with `backticks` and \"quotes\"\n");
    }

    #[test]
    fn at_path_missing_returns_file_not_found() {
        let mut r = InputResolver::new();
        let err = r.resolve("@/nonexistent/path/xyz.md".into()).unwrap_err();
        assert!(matches!(err, Error::FileNotFound(_)));
        assert_eq!(err.code(), "not_found");
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn resolve_opt_passes_through_none() {
        let mut r = InputResolver::new();
        assert_eq!(r.resolve_opt(None).unwrap(), None);
    }

    #[test]
    fn resolve_opt_resolves_some() {
        let mut r = InputResolver::new();
        assert_eq!(
            r.resolve_opt(Some("@@x".into())).unwrap(),
            Some("@x".into())
        );
    }

    #[test]
    fn second_stdin_flag_errors() {
        // We can't easily mock stdin in unit tests for the first read, but we
        // can pre-set the consumed flag and verify the second call fails.
        let mut r = InputResolver {
            stdin_consumed: true,
        };
        let err = r.resolve("-".into()).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)));
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn second_stdin_error_points_to_json_and_at_path() {
        // Agents hit this wall; the message must name the fix, not just the
        // failure (CLAUDE.md actionable-errors principle).
        let mut r = InputResolver {
            stdin_consumed: true,
        };
        let Error::InvalidArgument(msg) = r.resolve("-".into()).unwrap_err() else {
            panic!("expected InvalidArgument");
        };
        assert!(
            msg.contains("--json -"),
            "message should suggest `--json -`: {msg}"
        );
        assert!(
            msg.contains("@PATH"),
            "message should suggest @PATH fallback: {msg}"
        );
    }

    #[test]
    fn whitespace_around_dash_still_triggers_stdin_sentinel() {
        // Stray whitespace from heredocs / $(...) substitutions should not
        // turn `-` into a literal value.
        let mut r = InputResolver {
            stdin_consumed: true,
        };
        for v in ["- ", " -", "  -  ", "\t-\n"] {
            let err = r.resolve(v.into()).unwrap_err();
            assert!(
                matches!(err, Error::InvalidArgument(_)),
                "expected stdin sentinel match for {v:?}"
            );
        }
    }

    #[test]
    fn whitespace_around_at_path_still_reads_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "hello").unwrap();
        let path = tmp.path().to_str().unwrap();

        let mut r = InputResolver::new();
        let out = r.resolve(format!("  @{path}\n")).unwrap();
        assert_eq!(out, "hello");
    }

    #[test]
    fn plain_value_whitespace_is_preserved() {
        // Only sentinel matching is whitespace-tolerant; plain values must
        // pass through byte-for-byte.
        let mut r = InputResolver::new();
        assert_eq!(r.resolve("  hello  ".into()).unwrap(), "  hello  ");
        assert_eq!(r.resolve("\nbody\n".into()).unwrap(), "\nbody\n");
    }

    #[test]
    fn binary_file_with_invalid_utf8_errors() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        // 0xFF is invalid as a leading UTF-8 byte
        tmp.write_all(&[0xFF, 0xFE, 0xFD]).unwrap();
        let path = tmp.path().to_str().unwrap();

        let mut r = InputResolver::new();
        let err = r.resolve(format!("@{path}")).unwrap_err();
        // read_to_string returns InvalidData for non-UTF-8 → Error::Io
        assert!(matches!(err, Error::Io(_)));
    }
}
