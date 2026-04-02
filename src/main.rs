mod cli;
mod error;
mod filter;
mod id;
mod store;
mod ticket;

use clap::Parser;
use cli::{Cli, Commands};
use error::{Error, Result};

const CLAUDE_MD_CONTENT: &str = r#"# vima — ticket tracker

`vima` is this project's ticket tracker. Tickets live in `.vima/tickets/`.

## Common commands

```
vima create "Title" [-t task|bug|feature] [-p 0-4] [--dep ID] [--tags foo,bar]
vima list [--tag foo] [--type bug] [--priority 0-2]
vima ready                    # tickets with no open deps
vima show ID
vima update ID --title "..." --description "..."
vima close ID [--reason "..."]
vima start ID                 # set status → in_progress
```

## Output format

All output is newline-delimited JSON (one object per line). Use `--pluck FIELD`
to extract a single field and `--count` to get a count.

```
vima list --pluck id          # print IDs only
vima list --count             # print number of open tickets
```

## Batch create with back-references

```
vima create --batch <<'EOF'
[
  {"title": "Task A", "id": "a"},
  {"title": "Task B", "dep": ["a"]}
]
EOF
```

## Dependencies

```
vima dep add ID DEP_ID        # ID depends on DEP_ID
vima dep add ID DEP_ID --blocks  # ID blocks DEP_ID
vima is-ready ID              # exits 0 if ready, 1 if blocked
```

## Automation tips

- Set `VIMA_EXACT=1` (or `--exact`) to disable partial ID matching.
- All commands exit 0 on success, non-zero on error.
"#;

fn cmd_init(args: cli::InitArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let vima_dir = cwd.join(".vima");
    let tickets_dir = vima_dir.join("tickets");

    std::fs::create_dir_all(&tickets_dir)?;

    let config_path = vima_dir.join("config.yml");
    if !config_path.exists() {
        let prefix = id::get_prefix(&cwd)?;
        std::fs::write(&config_path, format!("prefix: {}\n", prefix))?;
    }

    if args.with_instructions {
        let claude_md = cwd.join("CLAUDE.md");
        if claude_md.exists() {
            return Err(Error::InvalidField(
                "CLAUDE.md already exists — merge manually".into(),
            ));
        }
        std::fs::write(&claude_md, CLAUDE_MD_CONTENT)?;
        eprintln!("Created CLAUDE.md with vima usage instructions");
    }

    eprintln!("Initialized vima in .vima/");
    Ok(())
}

fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Create(_) => Err(Error::InvalidField("not implemented: create".into())),
        Commands::Show(_) => Err(Error::InvalidField("not implemented: show".into())),
        Commands::List(_) => Err(Error::InvalidField("not implemented: list".into())),
        Commands::Ready(_) => Err(Error::InvalidField("not implemented: ready".into())),
        Commands::Blocked(_) => Err(Error::InvalidField("not implemented: blocked".into())),
        Commands::Closed(_) => Err(Error::InvalidField("not implemented: closed".into())),
        Commands::Update(_) => Err(Error::InvalidField("not implemented: update".into())),
        Commands::Start(_) => Err(Error::InvalidField("not implemented: start".into())),
        Commands::Close(_) => Err(Error::InvalidField("not implemented: close".into())),
        Commands::Reopen(_) => Err(Error::InvalidField("not implemented: reopen".into())),
        Commands::IsReady(_) => Err(Error::InvalidField("not implemented: is-ready".into())),
        Commands::AddNote(_) => Err(Error::InvalidField("not implemented: add-note".into())),
        Commands::Dep(_) => Err(Error::InvalidField("not implemented: dep".into())),
        Commands::Undep(_) => Err(Error::InvalidField("not implemented: undep".into())),
        Commands::Link(_) => Err(Error::InvalidField("not implemented: link".into())),
        Commands::Unlink(_) => Err(Error::InvalidField("not implemented: unlink".into())),
        Commands::Init(args) => cmd_init(args),
        Commands::Help(_) => Err(Error::InvalidField("not implemented: help".into())),
        Commands::External(args) => Err(Error::InvalidField(format!("not implemented: {}", args[0]))),
    }
}

fn main() {
    let cli = Cli::parse();

    if let Err(err) = dispatch(cli) {
        error::log_error(&err);
        std::process::exit(err.exit_code());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn init_args(with_instructions: bool) -> cli::InitArgs {
        cli::InitArgs { with_instructions }
    }

    #[test]
    #[serial(env)]
    fn init_creates_vima_directory_structure() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init(init_args(false)).unwrap();

        assert!(tmp.path().join(".vima").is_dir());
        assert!(tmp.path().join(".vima/tickets").is_dir());
        assert!(tmp.path().join(".vima/config.yml").exists());
    }

    #[test]
    #[serial(env)]
    fn init_computes_prefix_from_dir_name() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("my-project");
        std::fs::create_dir(&project_dir).unwrap();
        std::env::set_current_dir(&project_dir).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init(init_args(false)).unwrap();

        let config = std::fs::read_to_string(project_dir.join(".vima/config.yml")).unwrap();
        assert!(config.contains("prefix: mp"), "config was: {config}");
    }

    #[test]
    #[serial(env)]
    fn init_idempotent_does_not_overwrite_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init(init_args(false)).unwrap();

        // Overwrite config with custom prefix
        std::fs::write(tmp.path().join(".vima/config.yml"), "prefix: custom\n").unwrap();

        // Run init again — must not overwrite config
        cmd_init(init_args(false)).unwrap();

        let config = std::fs::read_to_string(tmp.path().join(".vima/config.yml")).unwrap();
        assert!(config.contains("prefix: custom"), "config was: {config}");
    }

    #[test]
    #[serial(env)]
    fn init_idempotent_no_error_on_second_run() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init(init_args(false)).unwrap();
        cmd_init(init_args(false)).unwrap();
    }

    #[test]
    #[serial(env)]
    fn init_without_flag_does_not_create_claude_md() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init(init_args(false)).unwrap();

        assert!(!tmp.path().join("CLAUDE.md").exists());
    }

    #[test]
    #[serial(env)]
    fn init_with_instructions_creates_claude_md() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init(init_args(true)).unwrap();

        let claude_md = tmp.path().join("CLAUDE.md");
        assert!(claude_md.exists());
        let content = std::fs::read_to_string(&claude_md).unwrap();
        assert!(content.contains("create"), "missing 'create': {content}");
        assert!(content.contains("list"), "missing 'list': {content}");
        assert!(content.contains("ready"), "missing 'ready': {content}");
        assert!(content.contains("close"), "missing 'close': {content}");
        assert!(content.contains("vima"), "missing 'vima': {content}");
    }

    #[test]
    #[serial(env)]
    fn init_with_instructions_errors_if_claude_md_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        let claude_md = tmp.path().join("CLAUDE.md");
        std::fs::write(&claude_md, "existing content").unwrap();

        let result = cmd_init(init_args(true));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("CLAUDE.md already exists"),
            "unexpected error: {err_msg}"
        );

        // File must be unchanged
        let content = std::fs::read_to_string(&claude_md).unwrap();
        assert_eq!(content, "existing content");
    }
}
