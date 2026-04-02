mod cli;
mod deps;
mod error;
mod filter;
mod id;
mod output;
mod plugin;
mod store;
mod ticket;

use clap::{CommandFactory, Parser};
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

fn cmd_create(args: cli::CreateArgs, exact: bool) -> Result<()> {
    let title = args
        .title
        .ok_or_else(|| Error::InvalidField("title is required".into()))?;

    if let Some(p) = args.priority {
        if p > filter::MAX_PRIORITY {
            return Err(Error::InvalidField("priority must be 0-4".into()));
        }
    }

    let st = store::Store::open()?;
    let tickets_dir = st.tickets_dir().to_path_buf();

    let ticket_id = if let Some(explicit_id) = args.id {
        id::validate_id(&explicit_id)?;
        let path = tickets_dir.join(format!("{}.md", explicit_id));
        if path.exists() {
            return Err(Error::IdExists(explicit_id));
        }
        explicit_id
    } else {
        let project_root = st
            .root()
            .parent()
            .ok_or_else(|| Error::InvalidField("could not determine project root".into()))?;
        let prefix = id::get_prefix(project_root)?;
        id::generate_id(&prefix, &tickets_dir)?
    };

    let tags: Vec<String> = args
        .tags
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let deps = args
        .dep
        .iter()
        .map(|dep| st.resolve_id(dep, exact))
        .collect::<Result<Vec<_>>>()?;

    // Cycle detection for --dep: new ticket can't be in any existing chain,
    // so this is always a no-op today but enforces the invariant going forward.
    {
        let tickets = st.read_all()?;
        for dep in &deps {
            if let Some(cycle_path) = deps::would_create_cycle(&tickets, &ticket_id, dep) {
                return Err(Error::Cycle(cycle_path));
            }
        }
    }

    let parent = args
        .parent
        .map(|p| st.resolve_id(&p, exact))
        .transpose()?;

    let ticket = ticket::Ticket {
        id: ticket_id.clone(),
        title,
        status: ticket::Status::Open,
        ticket_type: args.ticket_type.unwrap_or(ticket::TicketType::Task),
        priority: args.priority.unwrap_or(2),
        tags,
        assignee: args.assignee,
        estimate: args.estimate,
        deps,
        links: vec![],
        parent,
        created: jiff::Timestamp::now().to_string(),
        description: args.description,
        design: args.design,
        acceptance: args.acceptance,
        notes: vec![],
        body: None,
        blocks: vec![],
        children: vec![],
    };

    st.write_ticket(&ticket)?;

    for block_target in &args.blocks {
        let resolved = st.resolve_id(block_target, exact)?;
        let mut target = st.read_ticket(&resolved)?;
        if !target.deps.contains(&ticket_id) {
            target.deps.push(ticket_id.clone());
        }
        st.write_ticket(&target)?;
    }

    eprintln!("Created {}", ticket_id);
    output::output_one(&ticket, &None)?;

    Ok(())
}

fn cmd_show(args: cli::ShowArgs, exact: bool) -> Result<()> {
    let st = store::Store::open()?;
    let resolved = st.resolve_id(&args.id, exact)?;
    let ticket = st.load_and_compute(&resolved)?;
    output::output_one(&ticket, &args.pluck)?;
    Ok(())
}

fn cmd_add_note(args: cli::AddNoteArgs, exact: bool) -> Result<()> {
    use std::io::Read;

    let st = store::Store::open()?;
    let resolved = st.resolve_id(&args.id, exact)?;

    let text = if let Some(t) = args.text {
        t
    } else {
        let mut buf = String::new();
        std::io::stdin().take(65536).read_to_string(&mut buf)?;
        buf.trim_end_matches('\n').to_string()
    };

    if text.is_empty() {
        return Err(Error::InvalidField("note text is empty".into()));
    }

    let mut ticket = st.read_ticket(&resolved)?;
    ticket.notes.push(ticket::Note {
        timestamp: jiff::Timestamp::now().to_string(),
        text,
    });
    st.write_ticket(&ticket)?;

    let updated = st.load_and_compute(&resolved)?;
    eprintln!("Added note to {}", resolved);
    output::output_one(&updated, &None)?;

    Ok(())
}

fn cmd_link(args: cli::LinkArgs, exact: bool) -> Result<()> {
    let st = store::Store::open()?;
    let id_a = st.resolve_id(&args.id_a, exact)?;
    let id_b = st.resolve_id(&args.id_b, exact)?;

    let mut ticket_a = st.read_ticket(&id_a)?;
    let mut ticket_b = st.read_ticket(&id_b)?;

    let mut changed = false;
    if !ticket_a.links.contains(&id_b) {
        ticket_a.links.push(id_b.clone());
        changed = true;
    }
    if !ticket_b.links.contains(&id_a) {
        ticket_b.links.push(id_a.clone());
        changed = true;
    }
    if changed {
        st.write_ticket(&ticket_a)?;
        st.write_ticket(&ticket_b)?;
    }

    let updated_a = st.load_and_compute(&id_a)?;
    let updated_b = st.load_and_compute(&id_b)?;
    eprintln!("Linked {} \u{2194} {}", id_a, id_b);
    output::output_many(&[updated_a, updated_b], &None, false)?;

    Ok(())
}

fn cmd_unlink(args: cli::LinkArgs, exact: bool) -> Result<()> {
    let st = store::Store::open()?;
    let id_a = st.resolve_id(&args.id_a, exact)?;
    let id_b = st.resolve_id(&args.id_b, exact)?;

    let mut ticket_a = st.read_ticket(&id_a)?;
    let mut ticket_b = st.read_ticket(&id_b)?;

    let had_link = ticket_a.links.contains(&id_b) || ticket_b.links.contains(&id_a);

    if had_link {
        ticket_a.links.retain(|x| x != &id_b);
        ticket_b.links.retain(|x| x != &id_a);
        st.write_ticket(&ticket_a)?;
        st.write_ticket(&ticket_b)?;
    }

    let updated_a = st.load_and_compute(&id_a)?;
    let updated_b = st.load_and_compute(&id_b)?;
    eprintln!("Unlinked {} \u{2194} {}", id_a, id_b);
    output::output_many(&[updated_a, updated_b], &None, false)?;

    Ok(())
}

fn cmd_dep_add(args: cli::AddDepArgs, exact: bool) -> Result<()> {
    let st = store::Store::open()?;
    let id = st.resolve_id(&args.id, exact)?;
    let dep_id = st.resolve_id(&args.dep_id, exact)?;

    // In --blocks mode: id blocks dep_id → add id to dep_id's deps list.
    // In normal mode:   id depends on dep_id → add dep_id to id's deps list.
    let (target_id, added_dep) = if args.blocks {
        (dep_id.clone(), id.clone())
    } else {
        (id.clone(), dep_id.clone())
    };

    let mut target = st.read_ticket(&target_id)?;

    // Duplicate check — no-op if dep already present
    if target.deps.contains(&added_dep) {
        let updated = st.load_and_compute(&target_id)?;
        output::output_one(&updated, &None)?;
        return Ok(());
    }

    // Cycle detection before write
    let tickets = st.read_all()?;
    if let Some(cycle_path) = deps::would_create_cycle(&tickets, &target_id, &added_dep) {
        return Err(Error::Cycle(cycle_path));
    }

    target.deps.push(added_dep.clone());
    st.write_ticket(&target)?;

    let updated = st.load_and_compute(&target_id)?;
    if args.blocks {
        eprintln!("Added dep {} to {}", id, dep_id);
    } else {
        eprintln!("Added dep {} to {}", dep_id, id);
    }
    output::output_one(&updated, &None)?;

    Ok(())
}

fn cmd_undep(args: cli::UndepArgs, exact: bool) -> Result<()> {
    let st = store::Store::open()?;
    let id = st.resolve_id(&args.id, exact)?;
    let dep_id = st.resolve_id(&args.dep_id, exact)?;

    let mut ticket = st.read_ticket(&id)?;

    if !ticket.deps.contains(&dep_id) {
        return Err(Error::InvalidField("dep not found".into()));
    }

    ticket.deps.retain(|d| d != &dep_id);
    st.write_ticket(&ticket)?;

    let updated = st.load_and_compute(&id)?;
    eprintln!("Removed dep {} from {}", dep_id, id);
    output::output_one(&updated, &None)?;

    Ok(())
}

fn cmd_dep_tree(args: cli::TreeArgs, exact: bool) -> Result<()> {
    let st = store::Store::open()?;
    let id = st.resolve_id(&args.id, exact)?;
    let tickets = st.read_all()?;
    let tree = deps::build_dep_tree(&tickets, &id, args.full)?;
    let value = serde_json::to_value(&tree)?;
    println!("{}", value);
    Ok(())
}

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

fn cmd_help() -> Result<()> {
    let mut cmd = Cli::command();
    cmd.print_help().map_err(|e| Error::InvalidField(e.to_string()))?;
    println!();

    let plugins = plugin::discover_plugins();
    if !plugins.is_empty() {
        println!("\nPlugin commands:");
        for (name, desc) in &plugins {
            match desc {
                Some(d) => println!("  {}    {}", name, d),
                None => println!("  {}", name),
            }
        }
    }
    Ok(())
}

fn dispatch(cli: Cli) -> Result<()> {
    let exact = cli.exact;
    match cli.command {
        Commands::Create(args) => cmd_create(args, exact),
        Commands::Show(args) => cmd_show(args, exact),
        Commands::List(_) => Err(Error::InvalidField("not implemented: list".into())),
        Commands::Ready(_) => Err(Error::InvalidField("not implemented: ready".into())),
        Commands::Blocked(_) => Err(Error::InvalidField("not implemented: blocked".into())),
        Commands::Closed(_) => Err(Error::InvalidField("not implemented: closed".into())),
        Commands::Update(_) => Err(Error::InvalidField("not implemented: update".into())),
        Commands::Start(_) => Err(Error::InvalidField("not implemented: start".into())),
        Commands::Close(_) => Err(Error::InvalidField("not implemented: close".into())),
        Commands::Reopen(_) => Err(Error::InvalidField("not implemented: reopen".into())),
        Commands::IsReady(_) => Err(Error::InvalidField("not implemented: is-ready".into())),
        Commands::AddNote(args) => cmd_add_note(args, exact),
        Commands::Dep(dep_args) => match dep_args.command {
            cli::DepCommands::Add(add_args) => cmd_dep_add(add_args, exact),
            cli::DepCommands::Tree(args) => cmd_dep_tree(args, exact),
            cli::DepCommands::Cycle => Err(Error::InvalidField("not implemented: dep cycle".into())),
        },
        Commands::Undep(args) => cmd_undep(args, exact),
        Commands::Link(args) => cmd_link(args, exact),
        Commands::Unlink(args) => cmd_unlink(args, exact),
        Commands::Init(args) => cmd_init(args),
        Commands::Help(_) => cmd_help(),
        Commands::External(args) => {
            let cmd = &args[0];
            let plugin_args = args[1..].to_vec();
            if plugin::try_plugin(cmd, &plugin_args).is_none() {
                return Err(Error::InvalidField(format!("unknown command: {}", cmd)));
            }
            Ok(())
        }
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

    // ── create command tests ─────────────────────────────────────────────────

    fn setup_vima(tmp: &tempfile::TempDir) {
        let vima = tmp.path().join(".vima");
        std::fs::create_dir_all(vima.join("tickets")).unwrap();
        std::fs::write(vima.join("config.yml"), "prefix: vi\n").unwrap();
        std::env::set_var("VIMA_DIR", vima.to_str().unwrap());
    }

    fn create_args(title: Option<&str>) -> cli::CreateArgs {
        cli::CreateArgs {
            title: title.map(|s| s.to_string()),
            ticket_type: None,
            priority: None,
            assignee: None,
            estimate: None,
            tags: None,
            description: None,
            design: None,
            acceptance: None,
            dep: vec![],
            blocks: vec![],
            parent: None,
            id: None,
            batch: false,
        }
    }

    #[test]
    #[serial(env)]
    fn create_basic_ticket_returns_json_with_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Fix auth"));
        args.ticket_type = Some(ticket::TicketType::Bug);
        args.priority = Some(1);

        let result = cmd_create(args, false);
        assert!(result.is_ok(), "create failed: {:?}", result);

        let tickets_dir = tmp.path().join(".vima/tickets");
        let entries: Vec<_> = std::fs::read_dir(&tickets_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
            .collect();
        assert_eq!(entries.len(), 1);

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket(
            entries[0]
                .file_name()
                .to_string_lossy()
                .strip_suffix(".md")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(ticket.title, "Fix auth");
        assert_eq!(ticket.ticket_type, ticket::TicketType::Bug);
        assert_eq!(ticket.priority, 1);
        assert_eq!(ticket.status, ticket::Status::Open);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_with_explicit_id() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Test"));
        args.id = Some("my-id-01".to_string());

        cmd_create(args, false).unwrap();

        let ticket_path = tmp.path().join(".vima/tickets/my-id-01.md");
        assert!(ticket_path.exists());

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("my-id-01").unwrap();
        assert_eq!(ticket.id, "my-id-01");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_without_title_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let args = create_args(None);
        let result = cmd_create(args, false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "invalid_field");
        assert!(err.to_string().contains("title is required"));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    fn create_with_traversal_id_returns_error() {
        let err = id::validate_id("../traversal").unwrap_err();
        assert_eq!(err.code(), "invalid_field");
    }

    #[test]
    #[serial(env)]
    fn create_with_invalid_priority_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("A"));
        args.priority = Some(5);

        let result = cmd_create(args, false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "invalid_field");
        assert!(err.to_string().contains("priority must be 0-4"));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_with_tags_populates_tags_field() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Tagged ticket"));
        args.tags = Some("backend,auth".to_string());
        args.id = Some("tagged-01".to_string());

        cmd_create(args, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("tagged-01").unwrap();
        assert_eq!(ticket.tags, vec!["backend", "auth"]);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_with_dep_populates_deps_field() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut dep_args = create_args(Some("Existing dep"));
        dep_args.id = Some("dep-01".to_string());
        cmd_create(dep_args, false).unwrap();

        let mut args = create_args(Some("Dependent"));
        args.id = Some("dep-02".to_string());
        args.dep = vec!["dep-01".to_string()];
        cmd_create(args, true).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("dep-02").unwrap();
        assert_eq!(ticket.deps, vec!["dep-01"]);

        std::env::remove_var("VIMA_DIR");
    }

    fn show_args(id: &str) -> cli::ShowArgs {
        cli::ShowArgs {
            id: id.to_string(),
            pluck: None,
        }
    }

    // ── show command tests ───────────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn show_returns_ticket_by_exact_id() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Show me"));
        args.id = Some("show-01".to_string());
        cmd_create(args, false).unwrap();

        let result = cmd_show(show_args("show-01"), true);
        assert!(result.is_ok(), "show failed: {:?}", result);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn show_resolves_partial_id() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Partial match"));
        args.id = Some("partial-01".to_string());
        cmd_create(args, false).unwrap();

        // Use prefix "partial" which should resolve to "partial-01"
        let result = cmd_show(show_args("partial"), false);
        assert!(result.is_ok(), "show with partial id failed: {:?}", result);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn show_with_exact_flag_rejects_partial_id() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Exact check"));
        args.id = Some("exact-01".to_string());
        cmd_create(args, false).unwrap();

        let result = cmd_show(show_args("exact"), true);
        assert!(result.is_err(), "expected error for partial id with --exact");
        let err = result.unwrap_err();
        assert_eq!(err.code(), "not_found");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn show_pluck_single_field() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Pluck me"));
        args.id = Some("pluck-01".to_string());
        cmd_create(args, false).unwrap();

        let mut sa = show_args("pluck-01");
        sa.pluck = Some("title".to_string());
        let result = cmd_show(sa, true);
        assert!(result.is_ok(), "show --pluck title failed: {:?}", result);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn show_pluck_multiple_fields() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Multi pluck"));
        args.id = Some("mpluck-01".to_string());
        cmd_create(args, false).unwrap();

        let mut sa = show_args("mpluck-01");
        sa.pluck = Some("title,priority".to_string());
        let result = cmd_show(sa, true);
        assert!(result.is_ok(), "show --pluck title,priority failed: {:?}", result);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn show_includes_computed_blocks_and_children() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        // Create parent and child tickets
        let mut parent_args = create_args(Some("Parent"));
        parent_args.id = Some("parent-01".to_string());
        cmd_create(parent_args, false).unwrap();

        // Create blocker and blocked ticket
        let mut blocker_args = create_args(Some("Blocker"));
        blocker_args.id = Some("blocker-01".to_string());
        cmd_create(blocker_args, false).unwrap();

        let mut blocked_args = create_args(Some("Blocked"));
        blocked_args.id = Some("blocked-01".to_string());
        blocked_args.dep = vec!["blocker-01".to_string()];
        blocked_args.parent = Some("parent-01".to_string());
        cmd_create(blocked_args, true).unwrap();

        // Show the blocker — its `blocks` should contain "blocked-01"
        let st = store::Store::open().unwrap();
        let ticket = st.load_and_compute("blocker-01").unwrap();
        assert!(
            ticket.blocks.contains(&"blocked-01".to_string()),
            "blocks field should contain blocked-01, got: {:?}",
            ticket.blocks
        );

        // Show the parent — its `children` should contain "blocked-01"
        let parent = st.load_and_compute("parent-01").unwrap();
        assert!(
            parent.children.contains(&"blocked-01".to_string()),
            "children field should contain blocked-01, got: {:?}",
            parent.children
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn show_nonexistent_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let result = cmd_show(show_args("nonexistent"), false);
        assert!(result.is_err(), "expected error for nonexistent id");
        let err = result.unwrap_err();
        assert_eq!(err.code(), "not_found");

        std::env::remove_var("VIMA_DIR");
    }

    fn add_note_args(id: &str, text: Option<&str>) -> cli::AddNoteArgs {
        cli::AddNoteArgs {
            id: id.to_string(),
            text: text.map(|s| s.to_string()),
        }
    }

    fn link_args(id_a: &str, id_b: &str) -> cli::LinkArgs {
        cli::LinkArgs {
            id_a: id_a.to_string(),
            id_b: id_b.to_string(),
        }
    }

    // ── add-note command tests ───────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn add_note_with_text_arg_saves_note() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Ticket with note"));
        args.id = Some("note-01".to_string());
        cmd_create(args, false).unwrap();

        cmd_add_note(add_note_args("note-01", Some("My note")), true).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("note-01").unwrap();
        assert_eq!(ticket.notes.len(), 1);
        assert_eq!(ticket.notes[0].text, "My note");
        assert!(!ticket.notes[0].timestamp.is_empty());

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn add_note_multiple_notes_appended() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Multi-note ticket"));
        args.id = Some("note-02".to_string());
        cmd_create(args, false).unwrap();

        cmd_add_note(add_note_args("note-02", Some("First note")), true).unwrap();
        cmd_add_note(add_note_args("note-02", Some("Second note")), true).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("note-02").unwrap();
        assert_eq!(ticket.notes.len(), 2);
        assert_eq!(ticket.notes[0].text, "First note");
        assert_eq!(ticket.notes[1].text, "Second note");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn add_note_with_empty_text_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Empty note ticket"));
        args.id = Some("note-03".to_string());
        cmd_create(args, false).unwrap();

        let result = cmd_add_note(add_note_args("note-03", Some("")), true);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "invalid_field");
        assert!(err.to_string().contains("note text is empty"));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn add_note_nonexistent_ticket_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let result = cmd_add_note(add_note_args("nonexistent", Some("note")), true);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "not_found");

        std::env::remove_var("VIMA_DIR");
    }

    // ── link command tests ───────────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn link_creates_symmetric_links() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("link-a".to_string());
        cmd_create(a, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("link-b".to_string());
        cmd_create(b, false).unwrap();

        cmd_link(link_args("link-a", "link-b"), true).unwrap();

        let st = store::Store::open().unwrap();
        let ta = st.read_ticket("link-a").unwrap();
        let tb = st.read_ticket("link-b").unwrap();
        assert!(ta.links.contains(&"link-b".to_string()));
        assert!(tb.links.contains(&"link-a".to_string()));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn link_idempotent_no_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("idem-a".to_string());
        cmd_create(a, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("idem-b".to_string());
        cmd_create(b, false).unwrap();

        cmd_link(link_args("idem-a", "idem-b"), true).unwrap();
        cmd_link(link_args("idem-a", "idem-b"), true).unwrap();

        let st = store::Store::open().unwrap();
        let ta = st.read_ticket("idem-a").unwrap();
        let tb = st.read_ticket("idem-b").unwrap();
        assert_eq!(ta.links.iter().filter(|x| *x == "idem-b").count(), 1);
        assert_eq!(tb.links.iter().filter(|x| *x == "idem-a").count(), 1);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn link_nonexistent_ticket_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("exists-a".to_string());
        cmd_create(a, false).unwrap();

        let result = cmd_link(link_args("exists-a", "does-not-exist"), true);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "not_found");

        // Verify exists-a was not modified
        let st = store::Store::open().unwrap();
        let ta = st.read_ticket("exists-a").unwrap();
        assert!(ta.links.is_empty());

        std::env::remove_var("VIMA_DIR");
    }

    // ── unlink command tests ─────────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn unlink_removes_symmetric_links() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("ul-a".to_string());
        cmd_create(a, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("ul-b".to_string());
        cmd_create(b, false).unwrap();

        cmd_link(link_args("ul-a", "ul-b"), true).unwrap();
        cmd_unlink(link_args("ul-a", "ul-b"), true).unwrap();

        let st = store::Store::open().unwrap();
        let ta = st.read_ticket("ul-a").unwrap();
        let tb = st.read_ticket("ul-b").unwrap();
        assert!(!ta.links.contains(&"ul-b".to_string()));
        assert!(!tb.links.contains(&"ul-a".to_string()));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn unlink_noop_when_not_linked() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("nul-a".to_string());
        cmd_create(a, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("nul-b".to_string());
        cmd_create(b, false).unwrap();

        // Unlink when never linked — should succeed without error
        let result = cmd_unlink(link_args("nul-a", "nul-b"), true);
        assert!(result.is_ok());

        std::env::remove_var("VIMA_DIR");
    }

    // ── dep add command tests ────────────────────────────────────────────────

    fn add_dep_args(id: &str, dep_id: &str, blocks: bool) -> cli::AddDepArgs {
        cli::AddDepArgs {
            id: id.to_string(),
            dep_id: dep_id.to_string(),
            blocks,
        }
    }

    fn undep_args(id: &str, dep_id: &str) -> cli::UndepArgs {
        cli::UndepArgs {
            id: id.to_string(),
            dep_id: dep_id.to_string(),
        }
    }

    #[test]
    #[serial(env)]
    fn dep_add_normal_mode_adds_dep_to_id() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("da-a".to_string());
        cmd_create(a, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("da-b".to_string());
        cmd_create(b, false).unwrap();

        cmd_dep_add(add_dep_args("da-a", "da-b", false), true).unwrap();

        let st = store::Store::open().unwrap();
        let ticket_a = st.read_ticket("da-a").unwrap();
        assert!(ticket_a.deps.contains(&"da-b".to_string()));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn dep_add_blocks_mode_adds_id_to_dep_id_deps() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("db-a".to_string());
        cmd_create(a, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("db-b".to_string());
        cmd_create(b, false).unwrap();

        // A blocks B → B's deps should contain A
        cmd_dep_add(add_dep_args("db-a", "db-b", true), true).unwrap();

        let st = store::Store::open().unwrap();
        let ticket_b = st.read_ticket("db-b").unwrap();
        assert!(ticket_b.deps.contains(&"db-a".to_string()));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn dep_add_idempotent_no_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("dd-a".to_string());
        cmd_create(a, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("dd-b".to_string());
        cmd_create(b, false).unwrap();

        cmd_dep_add(add_dep_args("dd-a", "dd-b", false), true).unwrap();
        cmd_dep_add(add_dep_args("dd-a", "dd-b", false), true).unwrap();

        let st = store::Store::open().unwrap();
        let ticket_a = st.read_ticket("dd-a").unwrap();
        assert_eq!(
            ticket_a.deps.iter().filter(|d| *d == "dd-b").count(),
            1,
            "expected exactly one dep entry"
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn dep_add_cycle_detection_returns_cycle_error() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        // Create A -> B -> C chain
        let mut a = create_args(Some("Ticket A"));
        a.id = Some("cy-a".to_string());
        cmd_create(a, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("cy-b".to_string());
        b.dep = vec!["cy-a".to_string()];
        cmd_create(b, true).unwrap();

        let mut c = create_args(Some("Ticket C"));
        c.id = Some("cy-c".to_string());
        c.dep = vec!["cy-b".to_string()];
        cmd_create(c, true).unwrap();

        // Adding A -> C (A depends on C) would create A -> C -> B -> A cycle
        let result = cmd_dep_add(add_dep_args("cy-a", "cy-c", false), true);
        assert!(result.is_err(), "expected cycle error");
        let err = result.unwrap_err();
        assert_eq!(err.code(), "cycle");
        assert_eq!(err.exit_code(), 2);

        // Verify error JSON contains "cycle" array
        let json = error::error_json(&err);
        assert!(json["cycle"].is_array(), "expected 'cycle' key in error json");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn undep_removes_dep_from_ticket() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("ud-a".to_string());
        cmd_create(a, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("ud-b".to_string());
        b.dep = vec!["ud-a".to_string()];
        cmd_create(b, true).unwrap();

        cmd_undep(undep_args("ud-b", "ud-a"), true).unwrap();

        let st = store::Store::open().unwrap();
        let ticket_b = st.read_ticket("ud-b").unwrap();
        assert!(!ticket_b.deps.contains(&"ud-a".to_string()));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn undep_dep_not_in_list_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("ue-a".to_string());
        cmd_create(a, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("ue-b".to_string());
        cmd_create(b, false).unwrap();

        let result = cmd_undep(undep_args("ue-a", "ue-b"), true);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "invalid_field");
        assert!(err.to_string().contains("dep not found"));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_with_blocks_updates_target_deps() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut target_args = create_args(Some("Target"));
        target_args.id = Some("target-01".to_string());
        cmd_create(target_args, false).unwrap();

        let mut args = create_args(Some("Blocker"));
        args.id = Some("blocker-01".to_string());
        args.blocks = vec!["target-01".to_string()];
        cmd_create(args, true).unwrap();

        let st = store::Store::open().unwrap();
        let target = st.read_ticket("target-01").unwrap();
        assert!(target.deps.contains(&"blocker-01".to_string()));

        std::env::remove_var("VIMA_DIR");
    }

    // ── dep tree command tests ───────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn dep_tree_linear_chain_a_b_c() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("tr-a".to_string());
        cmd_create(a, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("tr-b".to_string());
        cmd_create(b, false).unwrap();

        let mut c = create_args(Some("Ticket C"));
        c.id = Some("tr-c".to_string());
        cmd_create(c, false).unwrap();

        cmd_dep_add(add_dep_args("tr-a", "tr-b", false), true).unwrap();
        cmd_dep_add(add_dep_args("tr-b", "tr-c", false), true).unwrap();

        let st = store::Store::open().unwrap();
        let tickets = st.read_all().unwrap();
        let tree = deps::build_dep_tree(&tickets, "tr-a", false).unwrap();

        assert_eq!(tree.id, "tr-a");
        assert_eq!(tree.deps.len(), 1);
        assert_eq!(tree.deps[0].id, "tr-b");
        assert_eq!(tree.deps[0].deps.len(), 1);
        assert_eq!(tree.deps[0].deps[0].id, "tr-c");
        assert!(tree.deps[0].deps[0].deps.is_empty());

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn dep_tree_diamond_dedup_d_appears_once() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        for id in &["td-a", "td-b", "td-c", "td-d"] {
            let mut args = create_args(Some(&format!("Ticket {id}")));
            args.id = Some(id.to_string());
            cmd_create(args, false).unwrap();
        }

        // A→B, A→C, B→D, C→D
        cmd_dep_add(add_dep_args("td-a", "td-b", false), true).unwrap();
        cmd_dep_add(add_dep_args("td-a", "td-c", false), true).unwrap();
        cmd_dep_add(add_dep_args("td-b", "td-d", false), true).unwrap();
        cmd_dep_add(add_dep_args("td-c", "td-d", false), true).unwrap();

        let st = store::Store::open().unwrap();
        let tickets = st.read_all().unwrap();
        let tree = deps::build_dep_tree(&tickets, "td-a", false).unwrap();

        // Count how many times td-d appears in the tree
        fn count_id(node: &deps::TreeNode, id: &str) -> usize {
            let self_count = if node.id == id { 1 } else { 0 };
            self_count + node.deps.iter().map(|c| count_id(c, id)).sum::<usize>()
        }

        assert_eq!(count_id(&tree, "td-d"), 1, "td-d should appear exactly once");
        // td-d must appear at depth 2 (under td-b or td-c, not directly under td-a)
        assert!(
            tree.deps.iter().any(|c| c.deps.iter().any(|gc| gc.id == "td-d")),
            "td-d should be at depth 2"
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn dep_tree_diamond_full_d_appears_twice() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        for id in &["tf-a", "tf-b", "tf-c", "tf-d"] {
            let mut args = create_args(Some(&format!("Ticket {id}")));
            args.id = Some(id.to_string());
            cmd_create(args, false).unwrap();
        }

        cmd_dep_add(add_dep_args("tf-a", "tf-b", false), true).unwrap();
        cmd_dep_add(add_dep_args("tf-a", "tf-c", false), true).unwrap();
        cmd_dep_add(add_dep_args("tf-b", "tf-d", false), true).unwrap();
        cmd_dep_add(add_dep_args("tf-c", "tf-d", false), true).unwrap();

        let st = store::Store::open().unwrap();
        let tickets = st.read_all().unwrap();
        let tree = deps::build_dep_tree(&tickets, "tf-a", true).unwrap();

        fn count_id(node: &deps::TreeNode, id: &str) -> usize {
            let self_count = if node.id == id { 1 } else { 0 };
            self_count + node.deps.iter().map(|c| count_id(c, id)).sum::<usize>()
        }

        assert_eq!(count_id(&tree, "tf-d"), 2, "tf-d should appear twice in full mode");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn dep_tree_dangling_dep_shows_missing_marker() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        // Create ticket A and manually add a dep to a non-existent ticket
        let mut a = create_args(Some("Ticket A"));
        a.id = Some("tm-a".to_string());
        cmd_create(a, false).unwrap();

        // Manually inject a dangling dep by writing the ticket with a bad dep
        let vima_dir = std::env::var("VIMA_DIR").unwrap();
        let ticket_path = std::path::Path::new(&vima_dir).join("tickets/tm-a.md");
        let content = std::fs::read_to_string(&ticket_path).unwrap();
        // Add "ghost-id" to deps via a patched write
        let patched = content.replace("deps: []", "deps:\n  - ghost-id");
        std::fs::write(&ticket_path, patched).unwrap();

        let st = store::Store::open().unwrap();
        let tickets = st.read_all().unwrap();
        let tree = deps::build_dep_tree(&tickets, "tm-a", false).unwrap();

        assert_eq!(tree.deps.len(), 1);
        assert_eq!(tree.deps[0].id, "ghost-id");
        assert!(
            tree.deps[0].title.contains("[missing]"),
            "expected [missing] in title, got: {}",
            tree.deps[0].title
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn dep_tree_cycle_in_data_shows_cycle_marker() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        // Create A and B, then manually inject a cycle A→B→A (bypassing cycle checks)
        for id in &["tc-a", "tc-b"] {
            let mut args = create_args(Some(&format!("Ticket {id}")));
            args.id = Some(id.to_string());
            cmd_create(args, false).unwrap();
        }

        let vima_dir = std::env::var("VIMA_DIR").unwrap();

        // Inject A→B
        let a_path = std::path::Path::new(&vima_dir).join("tickets/tc-a.md");
        let content = std::fs::read_to_string(&a_path).unwrap();
        let patched = content.replace("deps: []", "deps:\n  - tc-b");
        std::fs::write(&a_path, patched).unwrap();

        // Inject B→A (creating the cycle)
        let b_path = std::path::Path::new(&vima_dir).join("tickets/tc-b.md");
        let content = std::fs::read_to_string(&b_path).unwrap();
        let patched = content.replace("deps: []", "deps:\n  - tc-a");
        std::fs::write(&b_path, patched).unwrap();

        let st = store::Store::open().unwrap();
        let tickets = st.read_all().unwrap();

        // Must not hang or panic
        let tree = deps::build_dep_tree(&tickets, "tc-a", true).unwrap();

        fn has_cycle_marker(node: &deps::TreeNode) -> bool {
            node.title.contains("[cycle]") || node.deps.iter().any(has_cycle_marker)
        }
        assert!(has_cycle_marker(&tree), "expected [cycle] marker in tree");

        std::env::remove_var("VIMA_DIR");
    }
}
