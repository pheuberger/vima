use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    NotFound(String),
    AmbiguousId(String, Vec<String>),
    Cycle(Vec<String>),
    InvalidBackref(String),
    IdExists(String),
    Stale {
        id: String,
        expected: Option<String>,
        actual: Option<String>,
    },
    AlreadyClaimed {
        id: String,
        current_assignee: String,
    },
    InvalidField(String),
    NoVimaDir,
    Io(std::io::Error),
    Yaml(String),
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Error::NotFound(_) => "not_found",
            Error::AmbiguousId(_, _) => "ambiguous_id",
            Error::Cycle(_) => "cycle",
            Error::InvalidBackref(_) => "invalid_backref",
            Error::IdExists(_) => "id_exists",
            Error::Stale { .. } => "stale",
            Error::AlreadyClaimed { .. } => "already_claimed",
            Error::InvalidField(_) => "invalid_field",
            Error::NoVimaDir => "no_vima_dir",
            Error::Io(_) => "io_error",
            Error::Yaml(_) => "yaml_error",
        }
    }

    pub fn suggestion(&self) -> &'static str {
        match self {
            Error::NotFound(_) => "run `vima list --pluck id` to see available tickets",
            Error::AmbiguousId(_, _) => "use --exact or provide more characters of the ID",
            Error::Cycle(_) => {
                "run `vima dep cycle` to inspect the cycle, then `vima undep` to break it"
            }
            Error::InvalidBackref(_) => {
                "back-references use $1, $2, etc. matching the order of items in the batch array"
            }
            Error::IdExists(_) => "choose a different --id or omit it to auto-generate",
            Error::Stale { .. } => {
                "re-read the ticket with `vima show` and retry your operation"
            }
            Error::AlreadyClaimed { .. } => {
                "the ticket is being worked on — coordinate with the current assignee or use `vima update --status open` to release it"
            }
            Error::InvalidField(_) => {
                "run `vima help <command> --json` to see valid fields and values"
            }
            Error::NoVimaDir => "run `vima init` to create a .vima/ store in this directory",
            Error::Io(_) => "check file permissions and disk space",
            Error::Yaml(_) => "check input for valid YAML/JSON syntax",
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Cycle(_) => 2,
            Error::NotFound(_) => 3,
            Error::AmbiguousId(_, _) => 3,
            Error::IdExists(_) => 4,
            Error::Stale { .. } => 5,
            Error::AlreadyClaimed { .. } => 6,
            Error::InvalidBackref(_) => 1,
            Error::InvalidField(_) => 1,
            Error::NoVimaDir => 1,
            Error::Io(_) => 1,
            Error::Yaml(_) => 1,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound(id) => write!(f, "ticket not found: {id}"),
            Error::AmbiguousId(prefix, matches) => {
                write!(f, "ambiguous id '{prefix}': matches {}", matches.join(", "))
            }
            Error::Cycle(path) => write!(f, "dependency cycle detected: {}", path.join(" -> ")),
            Error::InvalidBackref(r) => write!(f, "invalid batch back-reference: {r}"),
            Error::IdExists(id) => write!(f, "ticket id already exists: {id}"),
            Error::Stale {
                id,
                expected,
                actual,
            } => write!(
                f,
                "version conflict on {id}: expected {expected:?}, found {actual:?}"
            ),
            Error::AlreadyClaimed {
                id,
                current_assignee,
            } => write!(f, "ticket {id} is already claimed by {current_assignee}"),
            Error::InvalidField(msg) => write!(f, "invalid field: {msg}"),
            Error::NoVimaDir => write!(f, "no .vima/ directory found"),
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Yaml(msg) => write!(f, "yaml parse error: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::NotFound(_) => None,
            Error::AmbiguousId(_, _) => None,
            Error::Cycle(_) => None,
            Error::InvalidBackref(_) => None,
            Error::IdExists(_) => None,
            Error::Stale { .. } => None,
            Error::AlreadyClaimed { .. } => None,
            Error::InvalidField(_) => None,
            Error::NoVimaDir => None,
            Error::Yaml(_) => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Yaml(e.to_string())
    }
}

pub fn error_json(err: &Error) -> serde_json::Value {
    let code = err.code();
    let message = err.to_string();

    let suggestion = err.suggestion();
    let mut json = serde_json::json!({"error": code, "message": message, "suggestion": suggestion});
    match err {
        Error::AmbiguousId(_, matches) => {
            json["matches"] = serde_json::json!(matches);
        }
        Error::Cycle(path) => {
            json["cycle"] = serde_json::json!(path);
        }
        Error::Stale {
            id,
            expected,
            actual,
        } => {
            json["id"] = serde_json::json!(id);
            json["expected_version"] = serde_json::json!(expected);
            json["actual_version"] = serde_json::json!(actual);
        }
        Error::AlreadyClaimed {
            id,
            current_assignee,
        } => {
            json["id"] = serde_json::json!(id);
            json["current_assignee"] = serde_json::json!(current_assignee);
        }
        Error::NotFound(_) => {}
        Error::InvalidBackref(_) => {}
        Error::IdExists(_) => {}
        Error::InvalidField(_) => {}
        Error::NoVimaDir => {}
        Error::Io(_) => {}
        Error::Yaml(_) => {}
    }
    json
}

pub fn log_error(err: &Error) {
    use std::io::Write;
    let json = error_json(err);
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let _ = writeln!(handle, "{}", json);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_code() {
        assert_eq!(Error::NotFound("x".into()).code(), "not_found");
    }

    #[test]
    fn cycle_exit_code() {
        assert_eq!(Error::Cycle(vec![]).exit_code(), 2);
    }

    #[test]
    fn all_variants_code() {
        assert_eq!(
            Error::AmbiguousId("x".into(), vec![]).code(),
            "ambiguous_id"
        );
        assert_eq!(Error::InvalidBackref("$1".into()).code(), "invalid_backref");
        assert_eq!(Error::IdExists("abc".into()).code(), "id_exists");
        assert_eq!(
            Error::Stale {
                id: "x".into(),
                expected: None,
                actual: None
            }
            .code(),
            "stale"
        );
        assert_eq!(
            Error::AlreadyClaimed {
                id: "x".into(),
                current_assignee: "a".into()
            }
            .code(),
            "already_claimed"
        );
        assert_eq!(Error::InvalidField("bad".into()).code(), "invalid_field");
        assert_eq!(Error::NoVimaDir.code(), "no_vima_dir");
        assert_eq!(
            Error::Io(std::io::Error::new(std::io::ErrorKind::Other, "x")).code(),
            "io_error"
        );
        assert_eq!(Error::Yaml("msg".into()).code(), "yaml_error");
    }

    #[test]
    fn all_variants_exit_code() {
        assert_eq!(Error::NotFound("x".into()).exit_code(), 3);
        assert_eq!(Error::AmbiguousId("x".into(), vec![]).exit_code(), 3);
        assert_eq!(Error::InvalidBackref("$1".into()).exit_code(), 1);
        assert_eq!(Error::IdExists("abc".into()).exit_code(), 4);
        assert_eq!(
            Error::Stale {
                id: "x".into(),
                expected: None,
                actual: None
            }
            .exit_code(),
            5
        );
        assert_eq!(
            Error::AlreadyClaimed {
                id: "x".into(),
                current_assignee: "a".into()
            }
            .exit_code(),
            6
        );
        assert_eq!(Error::InvalidField("bad".into()).exit_code(), 1);
        assert_eq!(Error::NoVimaDir.exit_code(), 1);
        assert_eq!(
            Error::Io(std::io::Error::new(std::io::ErrorKind::Other, "x")).exit_code(),
            1
        );
        assert_eq!(Error::Yaml("msg".into()).exit_code(), 1);
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn from_serde_json_error() {
        let json_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("bad json").unwrap_err();
        let err: Error = json_err.into();
        assert!(matches!(err, Error::Yaml(_)));
    }

    #[test]
    fn log_error_ambiguous_id_contains_matches() {
        let err = Error::AmbiguousId("x".into(), vec!["a".into(), "b".into()]);
        let json = error_json(&err);
        assert_eq!(json["matches"], serde_json::json!(["a", "b"]));
        assert_eq!(json["error"], "ambiguous_id");
    }

    // ── Suggestion tests ────────────────────────────────────────────────────

    #[test]
    fn all_variants_have_suggestions() {
        let variants: Vec<Error> = vec![
            Error::NotFound("x".into()),
            Error::AmbiguousId("x".into(), vec![]),
            Error::Cycle(vec![]),
            Error::InvalidBackref("$1".into()),
            Error::IdExists("abc".into()),
            Error::Stale {
                id: "x".into(),
                expected: Some("aaa".into()),
                actual: Some("bbb".into()),
            },
            Error::AlreadyClaimed {
                id: "x".into(),
                current_assignee: "agent-1".into(),
            },
            Error::InvalidField("bad".into()),
            Error::NoVimaDir,
            Error::Io(std::io::Error::new(std::io::ErrorKind::Other, "x")),
            Error::Yaml("msg".into()),
        ];
        for err in &variants {
            assert!(
                !err.suggestion().is_empty(),
                "suggestion empty for {:?}",
                err
            );
        }
    }

    #[test]
    fn error_json_includes_suggestion() {
        let err = Error::NotFound("vi-1234".into());
        let json = error_json(&err);
        assert!(json["suggestion"].is_string());
        assert!(json["suggestion"].as_str().unwrap().contains("vima list"));
    }

    #[test]
    fn suggestion_not_found() {
        assert!(Error::NotFound("x".into())
            .suggestion()
            .contains("vima list --pluck id"));
    }

    #[test]
    fn suggestion_no_vima_dir() {
        assert!(Error::NoVimaDir.suggestion().contains("vima init"));
    }

    // ── Display trait tests ─────────────────────────────────────────────────

    #[test]
    fn display_not_found() {
        let err = Error::NotFound("abc-1234".into());
        assert_eq!(err.to_string(), "ticket not found: abc-1234");
    }

    #[test]
    fn display_ambiguous_id() {
        let err = Error::AmbiguousId("ab".into(), vec!["ab-001".into(), "ab-002".into()]);
        assert_eq!(err.to_string(), "ambiguous id 'ab': matches ab-001, ab-002");
    }

    #[test]
    fn display_cycle() {
        let err = Error::Cycle(vec!["a".into(), "b".into(), "c".into(), "a".into()]);
        assert_eq!(
            err.to_string(),
            "dependency cycle detected: a -> b -> c -> a"
        );
    }

    #[test]
    fn display_invalid_backref() {
        let err = Error::InvalidBackref("$99".into());
        assert_eq!(err.to_string(), "invalid batch back-reference: $99");
    }

    #[test]
    fn display_id_exists() {
        let err = Error::IdExists("vi-abcd".into());
        assert_eq!(err.to_string(), "ticket id already exists: vi-abcd");
    }

    #[test]
    fn display_invalid_field() {
        let err = Error::InvalidField("priority must be 0-4".into());
        assert_eq!(err.to_string(), "invalid field: priority must be 0-4");
    }

    #[test]
    fn display_no_vima_dir() {
        let err = Error::NoVimaDir;
        assert_eq!(err.to_string(), "no .vima/ directory found");
    }

    #[test]
    fn display_yaml_error() {
        let err = Error::Yaml("unexpected token".into());
        assert_eq!(err.to_string(), "yaml parse error: unexpected token");
    }

    #[test]
    fn display_io_error() {
        let err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file missing",
        ));
        assert_eq!(err.to_string(), "io error: file missing");
    }

    // ── Stale error tests ──────────────────────────────────────────────────

    #[test]
    fn display_stale() {
        let err = Error::Stale {
            id: "vi-abcd".into(),
            expected: Some("aaa1".into()),
            actual: Some("bbb2".into()),
        };
        assert_eq!(
            err.to_string(),
            "version conflict on vi-abcd: expected Some(\"aaa1\"), found Some(\"bbb2\")"
        );
    }

    #[test]
    fn stale_error_json_includes_versions() {
        let err = Error::Stale {
            id: "vi-abcd".into(),
            expected: Some("aaa1".into()),
            actual: Some("bbb2".into()),
        };
        let json = error_json(&err);
        assert_eq!(json["error"], "stale");
        assert_eq!(json["id"], "vi-abcd");
        assert_eq!(json["expected_version"], "aaa1");
        assert_eq!(json["actual_version"], "bbb2");
        assert!(json["suggestion"].as_str().unwrap().contains("re-read"));
    }

    // ── AlreadyClaimed error tests ─────────────────────────────────────────

    #[test]
    fn display_already_claimed() {
        let err = Error::AlreadyClaimed {
            id: "vi-abcd".into(),
            current_assignee: "agent-1".into(),
        };
        assert_eq!(
            err.to_string(),
            "ticket vi-abcd is already claimed by agent-1"
        );
    }

    #[test]
    fn already_claimed_error_json_includes_assignee() {
        let err = Error::AlreadyClaimed {
            id: "vi-abcd".into(),
            current_assignee: "agent-1".into(),
        };
        let json = error_json(&err);
        assert_eq!(json["error"], "already_claimed");
        assert_eq!(json["id"], "vi-abcd");
        assert_eq!(json["current_assignee"], "agent-1");
    }
}
