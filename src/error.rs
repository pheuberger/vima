use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    NotFound(String),
    AmbiguousId(String, Vec<String>),
    Cycle(Vec<String>),
    InvalidBackref(String),
    IdExists(String),
    InvalidField(String),
    NoVimaDir,
    IoError(std::io::Error),
    YamlError(String),
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Error::NotFound(_) => "not_found",
            Error::AmbiguousId(_, _) => "ambiguous_id",
            Error::Cycle(_) => "cycle",
            Error::InvalidBackref(_) => "invalid_backref",
            Error::IdExists(_) => "id_exists",
            Error::InvalidField(_) => "invalid_field",
            Error::NoVimaDir => "no_vima_dir",
            Error::IoError(_) => "io_error",
            Error::YamlError(_) => "yaml_error",
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Cycle(_) => 2,
            Error::NotFound(_) => 1,
            Error::AmbiguousId(_, _) => 1,
            Error::InvalidBackref(_) => 1,
            Error::IdExists(_) => 1,
            Error::InvalidField(_) => 1,
            Error::NoVimaDir => 1,
            Error::IoError(_) => 1,
            Error::YamlError(_) => 1,
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
            Error::InvalidField(msg) => write!(f, "invalid field: {msg}"),
            Error::NoVimaDir => write!(f, "no .vima/ directory found"),
            Error::IoError(e) => write!(f, "io error: {e}"),
            Error::YamlError(msg) => write!(f, "yaml parse error: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::IoError(e) => Some(e),
            Error::NotFound(_) => None,
            Error::AmbiguousId(_, _) => None,
            Error::Cycle(_) => None,
            Error::InvalidBackref(_) => None,
            Error::IdExists(_) => None,
            Error::InvalidField(_) => None,
            Error::NoVimaDir => None,
            Error::YamlError(_) => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::IoError(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::YamlError(e.to_string())
    }
}

pub fn error_json(err: &Error) -> serde_json::Value {
    let code = err.code();
    let message = err.to_string();

    let mut json = serde_json::json!({"error": code, "message": message});
    match err {
        Error::AmbiguousId(_, matches) => {
            json["matches"] = serde_json::json!(matches);
        }
        Error::Cycle(path) => {
            json["cycle"] = serde_json::json!(path);
        }
        Error::NotFound(_) => {}
        Error::InvalidBackref(_) => {}
        Error::IdExists(_) => {}
        Error::InvalidField(_) => {}
        Error::NoVimaDir => {}
        Error::IoError(_) => {}
        Error::YamlError(_) => {}
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
        assert_eq!(Error::AmbiguousId("x".into(), vec![]).code(), "ambiguous_id");
        assert_eq!(Error::InvalidBackref("$1".into()).code(), "invalid_backref");
        assert_eq!(Error::IdExists("abc".into()).code(), "id_exists");
        assert_eq!(Error::InvalidField("bad".into()).code(), "invalid_field");
        assert_eq!(Error::NoVimaDir.code(), "no_vima_dir");
        assert_eq!(Error::IoError(std::io::Error::new(std::io::ErrorKind::Other, "x")).code(), "io_error");
        assert_eq!(Error::YamlError("msg".into()).code(), "yaml_error");
    }

    #[test]
    fn all_variants_exit_code() {
        assert_eq!(Error::NotFound("x".into()).exit_code(), 1);
        assert_eq!(Error::AmbiguousId("x".into(), vec![]).exit_code(), 1);
        assert_eq!(Error::InvalidBackref("$1".into()).exit_code(), 1);
        assert_eq!(Error::IdExists("abc".into()).exit_code(), 1);
        assert_eq!(Error::InvalidField("bad".into()).exit_code(), 1);
        assert_eq!(Error::NoVimaDir.exit_code(), 1);
        assert_eq!(Error::IoError(std::io::Error::new(std::io::ErrorKind::Other, "x")).exit_code(), 1);
        assert_eq!(Error::YamlError("msg".into()).exit_code(), 1);
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::IoError(_)));
    }

    #[test]
    fn from_serde_json_error() {
        let json_err: serde_json::Error = serde_json::from_str::<serde_json::Value>("bad json").unwrap_err();
        let err: Error = json_err.into();
        assert!(matches!(err, Error::YamlError(_)));
    }

    #[test]
    fn log_error_ambiguous_id_contains_matches() {
        let err = Error::AmbiguousId("x".into(), vec!["a".into(), "b".into()]);
        let json = error_json(&err);
        assert_eq!(json["matches"], serde_json::json!(["a", "b"]));
        assert_eq!(json["error"], "ambiguous_id");
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
        assert_eq!(err.to_string(), "dependency cycle detected: a -> b -> c -> a");
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
        let err = Error::YamlError("unexpected token".into());
        assert_eq!(err.to_string(), "yaml parse error: unexpected token");
    }

    #[test]
    fn display_io_error() {
        let err = Error::IoError(std::io::Error::new(std::io::ErrorKind::NotFound, "file missing"));
        assert_eq!(err.to_string(), "io error: file missing");
    }
}
