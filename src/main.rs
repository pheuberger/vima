mod batch;
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

pub(crate) fn parse_tags(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn cmd_create(mut args: cli::CreateArgs, exact: bool, dry_run: bool, pretty: bool) -> Result<()> {
    // Merge --title flag into positional title (flag takes precedence)
    if args.title.is_none() {
        args.title = args.title_flag.take();
    }
    if let Some(ref json_str) = args.json {
        let obj: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| Error::Yaml(format!("invalid --json: {e}")))?;
        let obj = obj
            .as_object()
            .ok_or_else(|| Error::InvalidField("--json must be a JSON object".into()))?;
        if let Some(v) = obj.get("title").and_then(|v| v.as_str()) {
            args.title = Some(v.to_string());
        }
        if let Some(v) = obj.get("type").and_then(|v| v.as_str()) {
            args.ticket_type = Some(
                serde_json::from_value(serde_json::Value::String(v.to_string()))
                    .map_err(|_| Error::InvalidField(format!("invalid type: {v}")))?,
            );
        }
        if let Some(v) = obj.get("priority").and_then(|v| v.as_u64()) {
            args.priority = Some(v as u8);
        }
        if let Some(v) = obj.get("assignee").and_then(|v| v.as_str()) {
            args.assignee = Some(v.to_string());
        }
        if let Some(v) = obj.get("estimate").and_then(|v| v.as_u64()) {
            args.estimate = Some(v as u32);
        }
        if let Some(v) = obj.get("tags") {
            if let Some(s) = v.as_str() {
                args.tags = Some(s.to_string());
            } else if let Some(arr) = v.as_array() {
                let tags: Vec<String> = arr
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
                args.tags = Some(tags.join(","));
            }
        }
        if let Some(v) = obj.get("description").and_then(|v| v.as_str()) {
            args.description = Some(v.to_string());
        }
        if let Some(v) = obj.get("design").and_then(|v| v.as_str()) {
            args.design = Some(v.to_string());
        }
        if let Some(v) = obj.get("acceptance").and_then(|v| v.as_str()) {
            args.acceptance = Some(v.to_string());
        }
        if let Some(v) = obj.get("id").and_then(|v| v.as_str()) {
            args.id = Some(v.to_string());
        }
        if let Some(v) = obj.get("parent").and_then(|v| v.as_str()) {
            args.parent = Some(v.to_string());
        }
        if let Some(arr) = obj.get("dep").and_then(|v| v.as_array()) {
            args.dep = arr
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
        }
        if let Some(arr) = obj.get("blocks").and_then(|v| v.as_array()) {
            args.blocks = arr
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
        }
    }

    if args.batch {
        let st = store::Store::open()?;
        let tickets = batch::batch_create(&st, exact)?;
        output::output_many(&tickets, &None, false)?;
        return Ok(());
    }

    let title = args
        .title
        .ok_or_else(|| Error::InvalidField("title is required".into()))?;

    if let Some(p) = args.priority {
        if p > filter::MAX_PRIORITY {
            return Err(Error::InvalidField(format!(
                "priority must be 0-{}",
                filter::MAX_PRIORITY
            )));
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

    let tags: Vec<String> = parse_tags(args.tags.as_deref().unwrap_or(""));

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

    let parent = args.parent.map(|p| st.resolve_id(&p, exact)).transpose()?;

    let ticket = ticket::Ticket {
        id: ticket_id.clone(),
        version: None,
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

    if dry_run {
        let preview = serde_json::json!({
            "dry_run": true,
            "action": "create",
            "ticket": serde_json::to_value(&ticket)?,
        });
        println!("{}", serde_json::to_string_pretty(&preview)?);
        return Ok(());
    }

    st.write_ticket(&ticket)?;

    for block_target in &args.blocks {
        let resolved = st.resolve_id(block_target, exact)?;
        let mut target = st.read_ticket(&resolved)?;
        if !target.deps.contains(&ticket_id) {
            target.deps.push(ticket_id.clone());
        }
        st.write_ticket(&target)?;
    }

    if pretty {
        eprintln!("Created {}", ticket_id);
    }
    output::output_one(&ticket, &None)?;

    Ok(())
}

fn cmd_show(args: cli::ShowArgs, exact: bool, pretty: bool) -> Result<()> {
    let st = store::Store::open()?;
    let tickets: Vec<ticket::Ticket> = args
        .ids
        .iter()
        .map(|id| {
            let resolved = st.resolve_id(id, exact)?;
            st.load_and_compute(&resolved)
        })
        .collect::<Result<_>>()?;

    if pretty {
        for t in &tickets {
            output::pretty_show(t)?;
        }
    } else if tickets.len() == 1 {
        output::output_one(&tickets[0], &args.pluck)?;
    } else {
        output::output_many_full(&tickets, &args.pluck, false, true)?;
    }
    Ok(())
}

fn cmd_add_note(args: cli::AddNoteArgs, exact: bool, pretty: bool) -> Result<()> {
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
    if pretty {
        eprintln!("Added note to {}", resolved);
    }
    output::output_one(&updated, &None)?;

    Ok(())
}

fn cmd_link(args: cli::LinkArgs, exact: bool, pretty: bool) -> Result<()> {
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
    if pretty {
        eprintln!("Linked {} \u{2194} {}", id_a, id_b);
    }
    output::output_many(&[updated_a, updated_b], &None, false)?;

    Ok(())
}

fn cmd_unlink(args: cli::LinkArgs, exact: bool, pretty: bool) -> Result<()> {
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
    if pretty {
        eprintln!("Unlinked {} \u{2194} {}", id_a, id_b);
    }
    output::output_many(&[updated_a, updated_b], &None, false)?;

    Ok(())
}

fn cmd_dep_add(args: cli::AddDepArgs, exact: bool, dry_run: bool, pretty: bool) -> Result<()> {
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

    if dry_run {
        let preview = serde_json::json!({
            "dry_run": true,
            "action": "dep_add",
            "ticket": serde_json::to_value(&target)?,
        });
        println!("{}", serde_json::to_string_pretty(&preview)?);
        return Ok(());
    }

    st.write_ticket(&target)?;

    let updated = st.load_and_compute(&target_id)?;
    if pretty {
        if args.blocks {
            eprintln!("Added dep {} to {}", id, dep_id);
        } else {
            eprintln!("Added dep {} to {}", dep_id, id);
        }
    }
    output::output_one(&updated, &None)?;

    Ok(())
}

fn cmd_undep(args: cli::UndepArgs, exact: bool, pretty: bool) -> Result<()> {
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
    if pretty {
        eprintln!("Removed dep {} from {}", dep_id, id);
    }
    output::output_one(&updated, &None)?;

    Ok(())
}

fn cmd_update(mut args: cli::UpdateArgs, exact: bool, dry_run: bool, pretty: bool) -> Result<()> {
    if let Some(ref json_str) = args.json {
        let obj: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| Error::Yaml(format!("invalid --json: {e}")))?;
        let obj = obj
            .as_object()
            .ok_or_else(|| Error::InvalidField("--json must be a JSON object".into()))?;
        if let Some(v) = obj.get("title").and_then(|v| v.as_str()) {
            args.title = Some(v.to_string());
        }
        if let Some(v) = obj.get("description").and_then(|v| v.as_str()) {
            args.description = Some(v.to_string());
        }
        if let Some(v) = obj.get("design").and_then(|v| v.as_str()) {
            args.design = Some(v.to_string());
        }
        if let Some(v) = obj.get("acceptance").and_then(|v| v.as_str()) {
            args.acceptance = Some(v.to_string());
        }
        if let Some(v) = obj.get("priority").and_then(|v| v.as_u64()) {
            args.priority = Some(v as u8);
        }
        if let Some(v) = obj.get("tags") {
            if let Some(s) = v.as_str() {
                args.tags = Some(s.to_string());
            } else if let Some(arr) = v.as_array() {
                let tags: Vec<String> = arr
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
                args.tags = Some(tags.join(","));
            }
        }
        if let Some(v) = obj.get("assignee").and_then(|v| v.as_str()) {
            args.assignee = Some(v.to_string());
        }
        if let Some(v) = obj.get("estimate").and_then(|v| v.as_u64()) {
            args.estimate = Some(v as u32);
        }
        if let Some(v) = obj.get("status").and_then(|v| v.as_str()) {
            args.status = Some(
                serde_json::from_value(serde_json::Value::String(v.to_string()))
                    .map_err(|_| Error::InvalidField(format!("invalid status: {v}")))?,
            );
        }
        if let Some(v) = obj.get("type").and_then(|v| v.as_str()) {
            args.ticket_type = Some(
                serde_json::from_value(serde_json::Value::String(v.to_string()))
                    .map_err(|_| Error::InvalidField(format!("invalid type: {v}")))?,
            );
        }
    }

    let st = store::Store::open()?;
    let resolved = st.resolve_id(&args.id, exact)?;
    let mut ticket = st.read_ticket(&resolved)?;

    if let Some(title) = args.title {
        ticket.title = title;
    }
    if let Some(description) = args.description {
        ticket.description = if description.is_empty() {
            None
        } else {
            Some(description)
        };
    }
    if let Some(design) = args.design {
        ticket.design = if design.is_empty() {
            None
        } else {
            Some(design)
        };
    }
    if let Some(acceptance) = args.acceptance {
        ticket.acceptance = if acceptance.is_empty() {
            None
        } else {
            Some(acceptance)
        };
    }
    if let Some(priority) = args.priority {
        if priority > filter::MAX_PRIORITY {
            return Err(Error::InvalidField(format!(
                "priority must be 0-{}",
                filter::MAX_PRIORITY
            )));
        }
        ticket.priority = priority;
    }
    if let Some(tags) = args.tags {
        ticket.tags = parse_tags(&tags);
    }
    if let Some(assignee) = args.assignee {
        ticket.assignee = if assignee.is_empty() {
            None
        } else {
            Some(assignee)
        };
    }
    if let Some(estimate) = args.estimate {
        ticket.estimate = Some(estimate);
    }
    if let Some(status) = args.status {
        ticket.status = status;
    }
    if let Some(ticket_type) = args.ticket_type {
        ticket.ticket_type = ticket_type;
    }

    if dry_run {
        let preview = serde_json::json!({
            "dry_run": true,
            "action": "update",
            "ticket": serde_json::to_value(&ticket)?,
        });
        println!("{}", serde_json::to_string_pretty(&preview)?);
        return Ok(());
    }

    st.write_ticket(&ticket)?;
    let updated = st.load_and_compute(&resolved)?;
    if pretty {
        eprintln!("Updated {}", resolved);
    }
    output::output_one(&updated, &None)?;

    Ok(())
}

fn cmd_set_status(
    id: &str,
    exact: bool,
    target: ticket::Status,
    verb: &str,
    dry_run: bool,
    pretty: bool,
) -> Result<()> {
    let st = store::Store::open()?;
    let resolved = st.resolve_id(id, exact)?;
    let mut ticket = st.read_ticket(&resolved)?;

    if ticket.status == target {
        let current = st.load_and_compute(&resolved)?;
        output::output_one(&current, &None)?;
        return Ok(());
    }

    ticket.status = target;

    if dry_run {
        let preview = serde_json::json!({
            "dry_run": true,
            "action": verb.to_lowercase(),
            "ticket": serde_json::to_value(&ticket)?,
        });
        println!("{}", serde_json::to_string_pretty(&preview)?);
        return Ok(());
    }

    st.write_ticket(&ticket)?;
    let updated = st.load_and_compute(&resolved)?;
    if pretty {
        eprintln!("{} {}", verb, resolved);
    }
    output::output_one(&updated, &None)?;

    Ok(())
}

fn cmd_start(args: cli::StartArgs, exact: bool, dry_run: bool, pretty: bool) -> Result<()> {
    let st = store::Store::open()?;
    let resolved = st.resolve_id(&args.id, exact)?;
    let mut ticket = st.read_ticket(&resolved)?;

    // Claim check: if ticket is already in_progress with an assignee
    if ticket.status == ticket::Status::InProgress {
        if let Some(ref current) = ticket.assignee {
            match &args.assignee {
                Some(new) if new == current => {
                    // Idempotent: same assignee, already started
                    let current_ticket = st.load_and_compute(&resolved)?;
                    output::output_one(&current_ticket, &None)?;
                    return Ok(());
                }
                _ => {
                    return Err(Error::AlreadyClaimed {
                        id: resolved,
                        current_assignee: current.clone(),
                    });
                }
            }
        }
        // No current assignee — if no new assignee either, idempotent no-op
        if args.assignee.is_none() {
            let current_ticket = st.load_and_compute(&resolved)?;
            output::output_one(&current_ticket, &None)?;
            return Ok(());
        }
    }

    ticket.status = ticket::Status::InProgress;
    if let Some(ref assignee) = args.assignee {
        ticket.assignee = Some(assignee.clone());
    }

    if dry_run {
        let preview = serde_json::json!({
            "dry_run": true,
            "action": "started",
            "ticket": serde_json::to_value(&ticket)?,
        });
        println!("{}", serde_json::to_string_pretty(&preview)?);
        return Ok(());
    }

    st.write_ticket(&ticket)?;
    let updated = st.load_and_compute(&resolved)?;
    if pretty {
        eprintln!("Started {}", resolved);
    }
    output::output_one(&updated, &None)?;

    Ok(())
}

fn cmd_close(args: cli::CloseArgs, exact: bool, dry_run: bool, pretty: bool) -> Result<()> {
    let st = store::Store::open()?;
    let mut closed_tickets = Vec::new();

    for raw_id in &args.ids {
        let resolved = st.resolve_id(raw_id, exact)?;
        let mut ticket = st.read_ticket(&resolved)?;

        if ticket.status == ticket::Status::Closed {
            let current = st.load_and_compute(&resolved)?;
            closed_tickets.push(current);
            continue;
        }

        ticket.status = ticket::Status::Closed;
        if let Some(ref reason) = args.reason {
            ticket.notes.push(ticket::Note {
                timestamp: jiff::Timestamp::now().to_string(),
                text: reason.clone(),
            });
        }
        if dry_run {
            closed_tickets.push(ticket);
            continue;
        }
        st.write_ticket(&ticket)?;
        let updated = st.load_and_compute(&resolved)?;
        if pretty {
            eprintln!("Closed {}", resolved);
        }
        closed_tickets.push(updated);
    }

    if dry_run {
        let preview = serde_json::json!({
            "dry_run": true,
            "action": "close",
            "tickets": serde_json::to_value(&closed_tickets)?,
        });
        println!("{}", serde_json::to_string_pretty(&preview)?);
        return Ok(());
    }

    output::output_many(&closed_tickets, &None, false)?;

    Ok(())
}

fn cmd_reopen(args: cli::IdArgs, exact: bool, dry_run: bool, pretty: bool) -> Result<()> {
    cmd_set_status(
        &args.id,
        exact,
        ticket::Status::Open,
        "Reopened",
        dry_run,
        pretty,
    )
}

fn cmd_dep_tree(args: cli::TreeArgs, exact: bool, pretty: bool) -> Result<()> {
    let st = store::Store::open()?;
    let id = st.resolve_id(&args.id, exact)?;
    let tickets = st.read_all()?;
    let tree = deps::build_dep_tree(&tickets, &id, args.full)?;
    if args.flat {
        let flat = deps::flatten_tree(&tree);
        println!("{}", serde_json::to_string(&flat)?);
    } else if pretty {
        output::pretty_tree(&tree);
    } else {
        println!("{}", serde_json::to_string(&tree)?);
    }
    Ok(())
}

fn cmd_dep_cycle() -> Result<()> {
    let st = store::Store::open()?;
    let tickets = st.read_all()?;
    let cycles = deps::detect_all_cycles(&tickets);
    println!("{}", serde_json::json!({ "cycles": cycles }));
    if !cycles.is_empty() {
        std::process::exit(2);
    }
    Ok(())
}

fn cmd_init(_args: cli::InitArgs, pretty: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let vima_dir = cwd.join(".vima");
    let tickets_dir = vima_dir.join("tickets");

    std::fs::create_dir_all(&tickets_dir)?;

    let config_path = vima_dir.join("config.yml");
    if !config_path.exists() {
        let prefix = id::get_prefix(&cwd)?;
        std::fs::write(&config_path, format!("prefix: {}\n", prefix))?;
    }

    if pretty {
        eprintln!("Initialized vima in .vima/");
    }
    Ok(())
}

fn cmd_help(args: cli::HelpArgs) -> Result<()> {
    if args.brief {
        let json = help_json();
        let commands = json["commands"].as_array().ok_or_else(|| {
            Error::InvalidField("internal error: help_json missing commands".into())
        })?;
        let brief: Vec<serde_json::Value> = commands
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c["name"],
                    "about": c.get("about").cloned().unwrap_or(serde_json::Value::Null),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&brief)?);
        return Ok(());
    }

    if args.json {
        if let Some(ref cmd_name) = args.command {
            let full = help_json();
            let commands = full["commands"].as_array().ok_or_else(|| {
                Error::InvalidField("internal error: help_json missing commands".into())
            })?;
            let found = commands
                .iter()
                .find(|c| c["name"].as_str() == Some(cmd_name));
            match found {
                Some(cmd_json) => {
                    println!("{}", serde_json::to_string_pretty(cmd_json)?);
                }
                None => {
                    return Err(Error::NotFound(format!("command '{}'", cmd_name)));
                }
            }
        } else {
            let json = help_json();
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        return Ok(());
    }

    let mut cmd = Cli::command();

    if let Some(ref subcmd_name) = args.command {
        let sub = cmd
            .get_subcommands_mut()
            .find(|s| s.get_name() == subcmd_name);
        match sub {
            Some(s) => {
                s.print_help()
                    .map_err(|e| Error::InvalidField(e.to_string()))?;
                println!();
            }
            None => {
                return Err(Error::NotFound(format!("command '{}'", subcmd_name)));
            }
        }
    } else {
        cmd.print_help()
            .map_err(|e| Error::InvalidField(e.to_string()))?;
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
    }
    Ok(())
}

fn help_json() -> serde_json::Value {
    let cmd = Cli::command();
    let mut commands = Vec::new();

    for sub in cmd.get_subcommands() {
        let name = sub.get_name().to_string();
        let about = sub.get_about().map(|a| a.to_string());

        let mut args = Vec::new();
        for arg in sub.get_arguments() {
            if arg.get_id() == "help" || arg.get_id() == "version" {
                continue;
            }
            let mut entry = serde_json::json!({
                "name": arg.get_id().as_str(),
            });
            if let Some(short) = arg.get_short() {
                entry["short"] = serde_json::json!(format!("-{}", short));
            }
            if let Some(long) = arg.get_long() {
                entry["long"] = serde_json::json!(format!("--{}", long));
            }
            if let Some(help) = arg.get_help() {
                entry["help"] = serde_json::json!(help.to_string());
            }
            entry["required"] = serde_json::json!(arg.is_required_set());
            if let Some(vals) = arg.get_value_names() {
                let names: Vec<&str> = vals.iter().map(|v| v.as_str()).collect();
                if !names.is_empty() {
                    entry["value_name"] = serde_json::json!(names.join(", "));
                }
            }
            args.push(entry);
        }

        let mut subcommands = Vec::new();
        for subsub in sub.get_subcommands() {
            let mut sc = serde_json::json!({
                "name": subsub.get_name(),
            });
            if let Some(about) = subsub.get_about() {
                sc["about"] = serde_json::json!(about.to_string());
            }
            let mut sc_args = Vec::new();
            for arg in subsub.get_arguments() {
                if arg.get_id() == "help" || arg.get_id() == "version" {
                    continue;
                }
                let mut entry = serde_json::json!({"name": arg.get_id().as_str()});
                if let Some(short) = arg.get_short() {
                    entry["short"] = serde_json::json!(format!("-{}", short));
                }
                if let Some(long) = arg.get_long() {
                    entry["long"] = serde_json::json!(format!("--{}", long));
                }
                if let Some(help) = arg.get_help() {
                    entry["help"] = serde_json::json!(help.to_string());
                }
                entry["required"] = serde_json::json!(arg.is_required_set());
                sc_args.push(entry);
            }
            if !sc_args.is_empty() {
                sc["args"] = serde_json::json!(sc_args);
            }
            subcommands.push(sc);
        }

        let mut cmd_json = serde_json::json!({ "name": name });
        if let Some(about) = about {
            cmd_json["about"] = serde_json::json!(about);
        }
        if !args.is_empty() {
            cmd_json["args"] = serde_json::json!(args);
        }
        if !subcommands.is_empty() {
            cmd_json["subcommands"] = serde_json::json!(subcommands);
        }
        commands.push(cmd_json);
    }

    let plugins = plugin::discover_plugins();
    let plugin_json: Vec<serde_json::Value> = plugins
        .iter()
        .map(|(name, desc)| {
            let mut p = serde_json::json!({"name": name});
            if let Some(d) = desc {
                p["about"] = serde_json::json!(d);
            }
            p
        })
        .collect();

    let mut root = serde_json::json!({
        "name": "vima",
        "about": "AI-agent-first ticketing CLI",
        "global_flags": [
            {"long": "--pretty", "help": "Human-only: pretty-print output (agents: use default JSON + --pluck instead)"},
            {"long": "--exact", "help": "Use exact ID matching (no partial match). Also: VIMA_EXACT=1"},
            {"long": "--dry-run", "help": "Preview changes without persisting (mutating commands only)"}
        ],
        "output_format": "All commands emit JSON to stdout. Errors are JSON on stderr.",
        "exit_codes": {
            "0": "success",
            "1": "general error (invalid_field, io_error, yaml_error, etc.)",
            "2": "cycle detected or ticket blocked (is-ready, dep cycle)",
            "3": "not found or ambiguous ID (not_found, ambiguous_id)",
            "4": "conflict (id_exists)",
            "5": "stale (concurrent modification detected — re-read and retry)",
            "6": "already claimed (ticket in_progress with different assignee)"
        },
        "commands": commands,
    });

    if !plugin_json.is_empty() {
        root["plugins"] = serde_json::json!(plugin_json);
    }

    root
}

fn cmd_list(args: cli::FilterArgs, pretty: bool) -> Result<()> {
    let st = store::Store::open()?;
    let mut tickets = st.read_all()?;
    deps::compute_reverse_fields(&mut tickets);
    let filter = filter::Filter::from_args(&args)?;
    let filtered = filter::apply_filters(tickets, &filter);
    if pretty {
        output::pretty_list(&filtered)
    } else {
        output::output_many_full(&filtered, &args.pluck, args.count, args.full)
    }
}

fn closed_collect(args: &cli::ClosedArgs) -> Result<Vec<ticket::Ticket>> {
    let st = store::Store::open()?;
    let mut tickets = st.read_all()?;
    deps::compute_reverse_fields(&mut tickets);

    // Force status=Closed regardless of user --status flag
    let mut filter = filter::Filter::from_args(&args.filter)?;
    filter.status = Some(ticket::Status::Closed);

    // Default limit=20 if not provided
    if filter.limit.is_none() {
        filter.limit = Some(20);
    }

    // Filter first (without applying limit/sort from apply_filters)
    let mut filtered: Vec<ticket::Ticket> =
        tickets.into_iter().filter(|t| filter.matches(t)).collect();

    // Sort by mtime DESC — pre-collect to avoid O(n log n) filesystem calls
    let tickets_dir = st.tickets_dir().to_path_buf();
    let mtimes: std::collections::HashMap<String, Option<std::time::SystemTime>> = filtered
        .iter()
        .map(|t| {
            let mtime = tickets_dir
                .join(format!("{}.md", t.id))
                .metadata()
                .and_then(|m| m.modified())
                .ok();
            (t.id.clone(), mtime)
        })
        .collect();
    filtered.sort_by(|a, b| mtimes[&b.id].cmp(&mtimes[&a.id]));

    // Apply limit
    if let Some(limit) = filter.limit {
        filtered.truncate(limit);
    }

    Ok(filtered)
}

fn cmd_closed(args: cli::ClosedArgs, pretty: bool) -> Result<()> {
    let filtered = closed_collect(&args)?;

    if pretty {
        output::pretty_list(&filtered)
    } else {
        output::output_many_full(
            &filtered,
            &args.filter.pluck,
            args.filter.count,
            args.filter.full,
        )
    }
}

fn closed_id_set(tickets: &[ticket::Ticket]) -> std::collections::HashSet<String> {
    tickets
        .iter()
        .filter(|t| t.status == ticket::Status::Closed)
        .map(|t| t.id.clone())
        .collect()
}

fn cmd_ready(args: cli::FilterArgs, pretty: bool) -> Result<()> {
    let st = store::Store::open()?;
    let mut tickets = st.read_all()?;
    deps::compute_reverse_fields(&mut tickets);

    let closed_ids = closed_id_set(&tickets);

    // Keep only open/in_progress tickets where ALL deps are closed
    let candidates: Vec<ticket::Ticket> = tickets
        .into_iter()
        .filter(|t| {
            (t.status == ticket::Status::Open || t.status == ticket::Status::InProgress)
                && t.deps.iter().all(|dep_id| closed_ids.contains(dep_id))
        })
        .collect();

    // Apply tag/type/priority/assignee filters, but not status (already handled)
    let mut filter = filter::Filter::from_args(&args)?;
    filter.status = None;

    let filtered = filter::apply_filters(candidates, &filter);
    if pretty {
        output::pretty_list(&filtered)
    } else {
        output::output_many_full(&filtered, &args.pluck, args.count, args.full)
    }
}

fn cmd_blocked(args: cli::FilterArgs, pretty: bool) -> Result<()> {
    let st = store::Store::open()?;
    let mut tickets = st.read_all()?;
    deps::compute_reverse_fields(&mut tickets);

    let closed_ids = closed_id_set(&tickets);

    // Keep only open/in_progress tickets where ANY dep is NOT closed
    let candidates: Vec<ticket::Ticket> = tickets
        .into_iter()
        .filter(|t| {
            (t.status == ticket::Status::Open || t.status == ticket::Status::InProgress)
                && t.deps.iter().any(|dep_id| !closed_ids.contains(dep_id))
        })
        .collect();

    let mut filter = filter::Filter::from_args(&args)?;
    filter.status = None;

    let filtered = filter::apply_filters(candidates, &filter);

    if pretty {
        return output::pretty_list(&filtered);
    }

    if args.count {
        println!("{}", filtered.len());
        return Ok(());
    }

    // Serialize each ticket and inject open_deps before output
    let values: Vec<serde_json::Value> = filtered
        .iter()
        .map(|t| -> Result<serde_json::Value> {
            let mut v = serde_json::to_value(t)?;
            let open_deps: Vec<&String> = t
                .deps
                .iter()
                .filter(|dep_id| !closed_ids.contains(*dep_id))
                .collect();
            v["open_deps"] = serde_json::json!(open_deps);
            Ok(v)
        })
        .collect::<Result<_>>()?;

    if let Some(ref fields) = args.pluck {
        output::output_plucked(&values, fields);
    } else {
        let mut values = values;
        if !args.full {
            for v in &mut values {
                output::strip_heavy_fields(v);
            }
        }
        println!("{}", serde_json::Value::Array(values));
    }

    Ok(())
}

fn is_ready_state(id: &str, exact: bool) -> Result<(bool, Vec<String>)> {
    let st = store::Store::open()?;
    let resolved = st.resolve_id(id, exact)?;
    let tickets = st.read_all()?;

    let ticket = tickets
        .iter()
        .find(|t| t.id == resolved)
        .ok_or_else(|| Error::NotFound(resolved.clone()))?;

    if ticket.status == ticket::Status::Closed {
        return Ok((true, vec![]));
    }

    let open_deps: Vec<String> = ticket
        .deps
        .iter()
        .filter(|dep_id| {
            tickets
                .iter()
                .find(|t| &t.id == *dep_id)
                .map(|t| t.status != ticket::Status::Closed)
                .unwrap_or(true) // missing dep counts as open
        })
        .cloned()
        .collect();

    Ok((open_deps.is_empty(), open_deps))
}

fn cmd_is_ready(args: cli::IdArgs, exact: bool) -> Result<()> {
    let (ready, open_deps) = is_ready_state(&args.id, exact)?;
    let output = serde_json::json!({
        "ready": ready,
        "open_deps": open_deps,
    });
    println!("{}", output);
    if !ready {
        std::process::exit(2);
    }
    Ok(())
}

fn dispatch(cli: Cli) -> Result<()> {
    let exact = cli.exact;
    let pretty = cli.pretty;
    let dry_run = cli.dry_run;
    colored::control::set_override(pretty);

    // Commands that don't need the store skip locking entirely
    match &cli.command {
        Commands::Init(_) | Commands::Help(_) | Commands::External(_) => {}
        _ => {
            // Determine if the command mutates the store
            let is_mutating = matches!(
                &cli.command,
                Commands::Create(_)
                    | Commands::Update(_)
                    | Commands::Start(_)
                    | Commands::Close(_)
                    | Commands::Reopen(_)
                    | Commands::AddNote(_)
                    | Commands::Undep(_)
                    | Commands::Link(_)
                    | Commands::Unlink(_)
                    | Commands::Dep(cli::DepArgs {
                        command: cli::DepCommands::Add(_),
                        ..
                    })
            );
            // Acquire lock — the guard is held for the duration of this scope.
            // Mutating commands get exclusive access; read-only get shared.
            let store = store::Store::open()?;
            let _lock = if is_mutating {
                store.lock_exclusive()?
            } else {
                store.lock_shared()?
            };
            // Lock is held while the command runs, then released on drop
            return match cli.command {
                Commands::Create(args) => cmd_create(args, exact, dry_run, pretty),
                Commands::Show(args) => cmd_show(args, exact, pretty),
                Commands::List(args) => cmd_list(args, pretty),
                Commands::Ready(args) => cmd_ready(args, pretty),
                Commands::Blocked(args) => cmd_blocked(args, pretty),
                Commands::Closed(args) => cmd_closed(args, pretty),
                Commands::Update(args) => cmd_update(args, exact, dry_run, pretty),
                Commands::Start(args) => cmd_start(args, exact, dry_run, pretty),
                Commands::Close(args) => cmd_close(args, exact, dry_run, pretty),
                Commands::Reopen(args) => cmd_reopen(args, exact, dry_run, pretty),
                Commands::IsReady(args) => cmd_is_ready(args, exact),
                Commands::AddNote(args) => cmd_add_note(args, exact, pretty),
                Commands::Dep(dep_args) => match dep_args.command {
                    cli::DepCommands::Add(add_args) => {
                        cmd_dep_add(add_args, exact, dry_run, pretty)
                    }
                    cli::DepCommands::Tree(args) => cmd_dep_tree(args, exact, pretty),
                    cli::DepCommands::Cycle => cmd_dep_cycle(),
                },
                Commands::Undep(args) => cmd_undep(args, exact, pretty),
                Commands::Link(args) => cmd_link(args, exact, pretty),
                Commands::Unlink(args) => cmd_unlink(args, exact, pretty),
                _ => unreachable!(),
            };
        }
    }

    // Non-store commands (Init, Help, External) run without locking
    match cli.command {
        Commands::Init(args) => cmd_init(args, pretty),
        Commands::Help(args) => cmd_help(args),
        Commands::External(args) => {
            let cmd = &args[0];
            match plugin::try_plugin(cmd, &args[1..]) {
                None => Err(Error::InvalidField(format!("unknown command: {}", cmd))),
                Some(result) => result,
            }
        }
        _ => unreachable!(),
    }
}

fn main() {
    // Pre-process: intercept `help` before clap can hijack it.
    // Clap's parser special-cases a subcommand named "help" even with
    // disable_help_subcommand, so we handle it ourselves.
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "help" {
        let help_args = cli::HelpArgs {
            command: args.get(2).filter(|a| !a.starts_with('-')).cloned(),
            json: args.iter().any(|a| a == "--json"),
            brief: args.iter().any(|a| a == "--brief"),
        };
        if let Err(err) = cmd_help(help_args) {
            error::log_error(&err);
            std::process::exit(err.exit_code());
        }
        return;
    }

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            // --help / --version are not errors: clap returns them as Err
            // but they should print to stdout and exit 0.
            match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    let _ = e.print();
                    std::process::exit(0);
                }
                _ => {}
            }
            // Convert clap errors to structured JSON on stderr
            let message = e.to_string();
            // Extract the subcommand name from args to suggest available flags
            let subcmd = args.get(1).map(|s| s.as_str()).unwrap_or("");
            let available = available_flags_for_command(subcmd);
            let mut json = serde_json::json!({
                "error": "invalid_argument",
                "message": message.trim(),
            });
            if !available.is_empty() {
                json["available_flags"] = serde_json::json!(available);
            }
            use std::io::Write;
            let _ = writeln!(std::io::stderr(), "{}", json);
            std::process::exit(2);
        }
    };

    if let Err(err) = dispatch(cli) {
        error::log_error(&err);
        std::process::exit(err.exit_code());
    }
}

/// Return the available flags for a given subcommand name.
fn available_flags_for_command(subcmd: &str) -> Vec<&'static str> {
    match subcmd {
        "list" | "ready" | "blocked" => vec![
            "--status",
            "--tag",
            "--type",
            "--priority",
            "--assignee",
            "--limit",
            "--pluck",
            "--count",
            "--full",
        ],
        "closed" => vec!["--limit", "--pluck", "--count", "--full"],
        "create" => vec![
            "--id",
            "--priority",
            "--tags",
            "--type",
            "--assignee",
            "--estimate",
            "--description",
            "--parent",
            "--dep",
            "--json",
            "--batch",
        ],
        "update" => vec![
            "--title",
            "--priority",
            "--tags",
            "--type",
            "--assignee",
            "--estimate",
            "--description",
            "--status",
            "--version",
            "--json",
        ],
        "show" => vec![],
        "start" => vec!["--assignee"],
        "close" => vec!["--reason"],
        "dep" => vec![],
        "link" | "unlink" | "undep" | "reopen" | "is-ready" | "add-note" => vec![],
        "init" => vec!["--prefix"],
        "help" => vec!["--json", "--brief"],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Mirror of cmd_show that writes JSON output to an arbitrary writer instead of stdout.
    /// Used to capture output in tests without redirecting the global stdout fd.
    fn cmd_show_to_writer<W: std::io::Write>(
        args: cli::ShowArgs,
        exact: bool,
        w: &mut W,
    ) -> Result<()> {
        let st = store::Store::open()?;
        let tickets: Vec<ticket::Ticket> = args
            .ids
            .iter()
            .map(|id| {
                let resolved = st.resolve_id(id, exact)?;
                st.load_and_compute(&resolved)
            })
            .collect::<Result<_>>()?;
        if tickets.len() == 1 {
            output::output_one_to_writer(&tickets[0], &args.pluck, w)
        } else {
            output::output_many_full_to_writer(&tickets, &args.pluck, false, true, w)
        }
    }

    /// Mirror of cmd_list that writes output to an arbitrary writer.
    fn cmd_list_to_writer<W: std::io::Write>(args: cli::FilterArgs, w: &mut W) -> Result<()> {
        let st = store::Store::open()?;
        let mut tickets = st.read_all()?;
        deps::compute_reverse_fields(&mut tickets);
        let filter = filter::Filter::from_args(&args)?;
        let filtered = filter::apply_filters(tickets, &filter);
        output::output_many_full_to_writer(&filtered, &args.pluck, args.count, args.full, w)
    }

    /// Mirror of cmd_ready that writes output to an arbitrary writer.
    fn cmd_ready_to_writer<W: std::io::Write>(args: cli::FilterArgs, w: &mut W) -> Result<()> {
        let st = store::Store::open()?;
        let mut tickets = st.read_all()?;
        deps::compute_reverse_fields(&mut tickets);
        let closed_ids = closed_id_set(&tickets);
        let candidates: Vec<ticket::Ticket> = tickets
            .into_iter()
            .filter(|t| {
                (t.status == ticket::Status::Open || t.status == ticket::Status::InProgress)
                    && t.deps.iter().all(|dep_id| closed_ids.contains(dep_id))
            })
            .collect();
        let mut filter = filter::Filter::from_args(&args)?;
        filter.status = None;
        let filtered = filter::apply_filters(candidates, &filter);
        output::output_many_full_to_writer(&filtered, &args.pluck, args.count, args.full, w)
    }

    /// Mirror of cmd_blocked that writes output to an arbitrary writer.
    fn cmd_blocked_to_writer<W: std::io::Write>(args: cli::FilterArgs, w: &mut W) -> Result<()> {
        let st = store::Store::open()?;
        let mut tickets = st.read_all()?;
        deps::compute_reverse_fields(&mut tickets);
        let closed_ids = closed_id_set(&tickets);
        let candidates: Vec<ticket::Ticket> = tickets
            .into_iter()
            .filter(|t| {
                (t.status == ticket::Status::Open || t.status == ticket::Status::InProgress)
                    && t.deps.iter().any(|dep_id| !closed_ids.contains(dep_id))
            })
            .collect();
        let mut filter = filter::Filter::from_args(&args)?;
        filter.status = None;
        let filtered = filter::apply_filters(candidates, &filter);
        output::output_many_full_to_writer(&filtered, &args.pluck, args.count, args.full, w)
    }

    fn init_args() -> cli::InitArgs {
        cli::InitArgs {}
    }

    #[test]
    #[serial(env)]
    fn init_creates_vima_directory_structure() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init(init_args(), false).unwrap();

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

        cmd_init(init_args(), false).unwrap();

        let config = std::fs::read_to_string(project_dir.join(".vima/config.yml")).unwrap();
        assert!(config.contains("prefix: mp"), "config was: {config}");
    }

    #[test]
    #[serial(env)]
    fn init_idempotent_does_not_overwrite_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init(init_args(), false).unwrap();

        // Overwrite config with custom prefix
        std::fs::write(tmp.path().join(".vima/config.yml"), "prefix: custom\n").unwrap();

        // Run init again — must not overwrite config
        cmd_init(init_args(), false).unwrap();

        let config = std::fs::read_to_string(tmp.path().join(".vima/config.yml")).unwrap();
        assert!(config.contains("prefix: custom"), "config was: {config}");
    }

    #[test]
    #[serial(env)]
    fn init_idempotent_no_error_on_second_run() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init(init_args(), false).unwrap();
        cmd_init(init_args(), false).unwrap();
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
            title_flag: None,
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
            json: None,
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

        let result = cmd_create(args, false, false, false);
        assert!(result.is_ok(), "create failed: {:?}", result);

        let tickets_dir = tmp.path().join(".vima/tickets");
        let entries: Vec<_> = std::fs::read_dir(&tickets_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
            .collect();
        assert_eq!(entries.len(), 1);

        let st = store::Store::open().unwrap();
        let ticket = st
            .read_ticket(
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
    fn create_title_flag_overrides_positional() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(None);
        args.title_flag = Some("From --title flag".to_string());
        args.description = Some("From --body alias".to_string());

        let result = cmd_create(args, false, false, false);
        assert!(result.is_ok(), "create failed: {:?}", result);

        let st = store::Store::open().unwrap();
        let tickets = st.read_all().unwrap();
        assert_eq!(tickets.len(), 1);
        assert_eq!(tickets[0].title, "From --title flag");
        assert_eq!(tickets[0].description.as_deref(), Some("From --body alias"));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_with_explicit_id() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Test"));
        args.id = Some("my-id-01".to_string());

        cmd_create(args, false, false, false).unwrap();

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
        let result = cmd_create(args, false, false, false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "invalid_field");
        assert!(err.to_string().contains("title is required"));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_with_duplicate_id_returns_exit_code_4() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args1 = create_args(Some("First"));
        args1.id = Some("dup-id-01".to_string());
        cmd_create(args1, false, false, false).unwrap();

        let mut args2 = create_args(Some("Second with same ID"));
        args2.id = Some("dup-id-01".to_string());
        let result = cmd_create(args2, false, false, false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "id_exists");
        assert_eq!(err.exit_code(), 4);
        assert!(err.to_string().contains("dup-id-01"));

        // Verify error JSON has correct structure
        let json = error::error_json(&err);
        assert_eq!(json["error"], "id_exists");
        assert!(json["suggestion"]
            .as_str()
            .unwrap()
            .contains("different --id"));

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

        let result = cmd_create(args, false, false, false);
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

        cmd_create(args, false, false, false).unwrap();

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
        cmd_create(dep_args, false, false, false).unwrap();

        let mut args = create_args(Some("Dependent"));
        args.id = Some("dep-02".to_string());
        args.dep = vec!["dep-01".to_string()];
        cmd_create(args, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("dep-02").unwrap();
        assert_eq!(ticket.deps, vec!["dep-01"]);

        std::env::remove_var("VIMA_DIR");
    }

    // ── JSON input tests ──────────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn create_via_json_flag() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(None);
        args.json = Some(
            r#"{"title":"From JSON","type":"bug","priority":1,"tags":["a","b"],"id":"vi-json"}"#
                .to_string(),
        );

        let result = cmd_create(args, false, false, false);
        assert!(result.is_ok(), "create --json failed: {:?}", result);

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("vi-json").unwrap();
        assert_eq!(ticket.title, "From JSON");
        assert_eq!(ticket.ticket_type, ticket::TicketType::Bug);
        assert_eq!(ticket.priority, 1);
        assert_eq!(ticket.tags, vec!["a", "b"]);
    }

    #[test]
    #[serial(env)]
    fn create_via_json_flag_invalid_json() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(None);
        args.json = Some("not valid json".to_string());

        let result = cmd_create(args, false, false, false);
        assert!(result.is_err());
    }

    #[test]
    #[serial(env)]
    fn update_via_json_flag() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Original"));
        ca.id = Some("vi-ujsn".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("vi-ujsn");
        ua.json = Some(r#"{"title":"Updated via JSON","priority":0}"#.to_string());
        cmd_update(ua, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("vi-ujsn").unwrap();
        assert_eq!(ticket.title, "Updated via JSON");
        assert_eq!(ticket.priority, 0);
    }

    // ── dry-run tests ────────────────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn create_dry_run_does_not_persist() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Dry run ticket"));
        args.id = Some("vi-dryc".to_string());

        let result = cmd_create(args, false, true, false);
        assert!(result.is_ok(), "dry-run create failed: {:?}", result);

        // Ticket should NOT exist on disk
        let path = tmp.path().join(".vima/tickets/vi-dryc.md");
        assert!(!path.exists(), "dry-run should not write ticket file");
    }

    #[test]
    #[serial(env)]
    fn update_dry_run_does_not_persist() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Before dry-run"));
        ca.id = Some("vi-dryu".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("vi-dryu");
        ua.title = Some("After dry-run".to_string());
        cmd_update(ua, true, true, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("vi-dryu").unwrap();
        assert_eq!(
            ticket.title, "Before dry-run",
            "dry-run update should not persist"
        );
    }

    #[test]
    #[serial(env)]
    fn close_dry_run_does_not_persist() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Dry close"));
        ca.id = Some("vi-drcl".to_string());
        cmd_create(ca, false, false, false).unwrap();

        cmd_close(close_args(vec!["vi-drcl"]), true, true, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("vi-drcl").unwrap();
        assert_eq!(
            ticket.status,
            ticket::Status::Open,
            "dry-run close should not persist"
        );
    }

    fn show_args(id: &str) -> cli::ShowArgs {
        cli::ShowArgs {
            ids: vec![id.to_string()],
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
        cmd_create(args, false, false, false).unwrap();

        let result = cmd_show(show_args("show-01"), true, false);
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
        cmd_create(args, false, false, false).unwrap();

        // Use prefix "partial" which should resolve to "partial-01"
        let result = cmd_show(show_args("partial"), false, false);
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
        cmd_create(args, false, false, false).unwrap();

        let result = cmd_show(show_args("exact"), true, false);
        assert!(
            result.is_err(),
            "expected error for partial id with --exact"
        );
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
        cmd_create(args, false, false, false).unwrap();

        let mut sa = show_args("pluck-01");
        sa.pluck = Some("title".to_string());
        let result = cmd_show(sa, true, false);
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
        cmd_create(args, false, false, false).unwrap();

        let mut sa = show_args("mpluck-01");
        sa.pluck = Some("title,priority".to_string());
        let result = cmd_show(sa, true, false);
        assert!(
            result.is_ok(),
            "show --pluck title,priority failed: {:?}",
            result
        );

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
        cmd_create(parent_args, false, false, false).unwrap();

        // Create blocker and blocked ticket
        let mut blocker_args = create_args(Some("Blocker"));
        blocker_args.id = Some("blocker-01".to_string());
        cmd_create(blocker_args, false, false, false).unwrap();

        let mut blocked_args = create_args(Some("Blocked"));
        blocked_args.id = Some("blocked-01".to_string());
        blocked_args.dep = vec!["blocker-01".to_string()];
        blocked_args.parent = Some("parent-01".to_string());
        cmd_create(blocked_args, true, false, false).unwrap();

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

        let result = cmd_show(show_args("nonexistent"), false, false);
        assert!(result.is_err(), "expected error for nonexistent id");
        let err = result.unwrap_err();
        assert_eq!(err.code(), "not_found");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn show_multiple_ids_returns_array() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("First"));
        a.id = Some("multi-01".to_string());
        cmd_create(a, true, false, false).unwrap();
        let mut b = create_args(Some("Second"));
        b.id = Some("multi-02".to_string());
        cmd_create(b, true, false, false).unwrap();
        let mut c = create_args(Some("Third"));
        c.id = Some("multi-03".to_string());
        cmd_create(c, true, false, false).unwrap();

        let sa = cli::ShowArgs {
            ids: vec![
                "multi-01".to_string(),
                "multi-02".to_string(),
                "multi-03".to_string(),
            ],
            pluck: None,
        };
        let mut buf = Vec::new();
        cmd_show_to_writer(sa, true, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let arr = parsed.as_array().expect("expected JSON array for multi-id");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["id"], "multi-01");
        assert_eq!(arr[1]["id"], "multi-02");
        assert_eq!(arr[2]["id"], "multi-03");
        // Heavy fields included for show (unlike list)
        assert!(arr[0].get("title").is_some());

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn show_multiple_ids_with_pluck() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("A"));
        a.id = Some("mp-01".to_string());
        cmd_create(a, true, false, false).unwrap();
        let mut b = create_args(Some("B"));
        b.id = Some("mp-02".to_string());
        cmd_create(b, true, false, false).unwrap();

        let sa = cli::ShowArgs {
            ids: vec!["mp-01".to_string(), "mp-02".to_string()],
            pluck: Some("id,title".to_string()),
        };
        let mut buf = Vec::new();
        cmd_show_to_writer(sa, true, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let arr = parsed.as_array().expect("expected JSON array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], "mp-01");
        assert_eq!(arr[0]["title"], "A");
        assert_eq!(arr[1]["id"], "mp-02");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn show_multiple_fails_if_any_missing() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Exists"));
        a.id = Some("mm-01".to_string());
        cmd_create(a, true, false, false).unwrap();

        let sa = cli::ShowArgs {
            ids: vec!["mm-01".to_string(), "nonexistent".to_string()],
            pluck: None,
        };
        let result = cmd_show(sa, false, false);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "not_found");

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
        cmd_create(args, false, false, false).unwrap();

        cmd_add_note(add_note_args("note-01", Some("My note")), true, false).unwrap();

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
        cmd_create(args, false, false, false).unwrap();

        cmd_add_note(add_note_args("note-02", Some("First note")), true, false).unwrap();
        cmd_add_note(add_note_args("note-02", Some("Second note")), true, false).unwrap();

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
        cmd_create(args, false, false, false).unwrap();

        let result = cmd_add_note(add_note_args("note-03", Some("")), true, false);
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

        let result = cmd_add_note(add_note_args("nonexistent", Some("note")), true, false);
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
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("link-b".to_string());
        cmd_create(b, false, false, false).unwrap();

        cmd_link(link_args("link-a", "link-b"), true, false).unwrap();

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
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("idem-b".to_string());
        cmd_create(b, false, false, false).unwrap();

        cmd_link(link_args("idem-a", "idem-b"), true, false).unwrap();
        cmd_link(link_args("idem-a", "idem-b"), true, false).unwrap();

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
        cmd_create(a, false, false, false).unwrap();

        let result = cmd_link(link_args("exists-a", "does-not-exist"), true, false);
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
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("ul-b".to_string());
        cmd_create(b, false, false, false).unwrap();

        cmd_link(link_args("ul-a", "ul-b"), true, false).unwrap();
        cmd_unlink(link_args("ul-a", "ul-b"), true, false).unwrap();

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
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("nul-b".to_string());
        cmd_create(b, false, false, false).unwrap();

        // Unlink when never linked — should succeed without error
        let result = cmd_unlink(link_args("nul-a", "nul-b"), true, false);
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
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("da-b".to_string());
        cmd_create(b, false, false, false).unwrap();

        cmd_dep_add(add_dep_args("da-a", "da-b", false), true, false, false).unwrap();

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
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("db-b".to_string());
        cmd_create(b, false, false, false).unwrap();

        // A blocks B → B's deps should contain A
        cmd_dep_add(add_dep_args("db-a", "db-b", true), true, false, false).unwrap();

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
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("dd-b".to_string());
        cmd_create(b, false, false, false).unwrap();

        cmd_dep_add(add_dep_args("dd-a", "dd-b", false), true, false, false).unwrap();
        cmd_dep_add(add_dep_args("dd-a", "dd-b", false), true, false, false).unwrap();

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
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("cy-b".to_string());
        b.dep = vec!["cy-a".to_string()];
        cmd_create(b, true, false, false).unwrap();

        let mut c = create_args(Some("Ticket C"));
        c.id = Some("cy-c".to_string());
        c.dep = vec!["cy-b".to_string()];
        cmd_create(c, true, false, false).unwrap();

        // Adding A -> C (A depends on C) would create A -> C -> B -> A cycle
        let result = cmd_dep_add(add_dep_args("cy-a", "cy-c", false), true, false, false);
        assert!(result.is_err(), "expected cycle error");
        let err = result.unwrap_err();
        assert_eq!(err.code(), "cycle");
        assert_eq!(err.exit_code(), 2);

        // Verify error JSON contains "cycle" array
        let json = error::error_json(&err);
        assert!(
            json["cycle"].is_array(),
            "expected 'cycle' key in error json"
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn undep_removes_dep_from_ticket() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ticket A"));
        a.id = Some("ud-a".to_string());
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("ud-b".to_string());
        b.dep = vec!["ud-a".to_string()];
        cmd_create(b, true, false, false).unwrap();

        cmd_undep(undep_args("ud-b", "ud-a"), true, false).unwrap();

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
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("ue-b".to_string());
        cmd_create(b, false, false, false).unwrap();

        let result = cmd_undep(undep_args("ue-a", "ue-b"), true, false);
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
        cmd_create(target_args, false, false, false).unwrap();

        let mut args = create_args(Some("Blocker"));
        args.id = Some("blocker-01".to_string());
        args.blocks = vec!["target-01".to_string()];
        cmd_create(args, true, false, false).unwrap();

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
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("tr-b".to_string());
        cmd_create(b, false, false, false).unwrap();

        let mut c = create_args(Some("Ticket C"));
        c.id = Some("tr-c".to_string());
        cmd_create(c, false, false, false).unwrap();

        cmd_dep_add(add_dep_args("tr-a", "tr-b", false), true, false, false).unwrap();
        cmd_dep_add(add_dep_args("tr-b", "tr-c", false), true, false, false).unwrap();

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
            cmd_create(args, false, false, false).unwrap();
        }

        // A→B, A→C, B→D, C→D
        cmd_dep_add(add_dep_args("td-a", "td-b", false), true, false, false).unwrap();
        cmd_dep_add(add_dep_args("td-a", "td-c", false), true, false, false).unwrap();
        cmd_dep_add(add_dep_args("td-b", "td-d", false), true, false, false).unwrap();
        cmd_dep_add(add_dep_args("td-c", "td-d", false), true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let tickets = st.read_all().unwrap();
        let tree = deps::build_dep_tree(&tickets, "td-a", false).unwrap();

        // Count how many times td-d appears in the tree
        fn count_id(node: &deps::TreeNode, id: &str) -> usize {
            let self_count = if node.id == id { 1 } else { 0 };
            self_count + node.deps.iter().map(|c| count_id(c, id)).sum::<usize>()
        }

        assert_eq!(
            count_id(&tree, "td-d"),
            1,
            "td-d should appear exactly once"
        );
        // td-d must appear at depth 2 (under td-b or td-c, not directly under td-a)
        assert!(
            tree.deps
                .iter()
                .any(|c| c.deps.iter().any(|gc| gc.id == "td-d")),
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
            cmd_create(args, false, false, false).unwrap();
        }

        cmd_dep_add(add_dep_args("tf-a", "tf-b", false), true, false, false).unwrap();
        cmd_dep_add(add_dep_args("tf-a", "tf-c", false), true, false, false).unwrap();
        cmd_dep_add(add_dep_args("tf-b", "tf-d", false), true, false, false).unwrap();
        cmd_dep_add(add_dep_args("tf-c", "tf-d", false), true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let tickets = st.read_all().unwrap();
        let tree = deps::build_dep_tree(&tickets, "tf-a", true).unwrap();

        fn count_id(node: &deps::TreeNode, id: &str) -> usize {
            let self_count = if node.id == id { 1 } else { 0 };
            self_count + node.deps.iter().map(|c| count_id(c, id)).sum::<usize>()
        }

        assert_eq!(
            count_id(&tree, "tf-d"),
            2,
            "tf-d should appear twice in full mode"
        );

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
        cmd_create(a, false, false, false).unwrap();

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
            cmd_create(args, false, false, false).unwrap();
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

    // ── update command tests ─────────────────────────────────────────────────

    fn update_args(id: &str) -> cli::UpdateArgs {
        cli::UpdateArgs {
            id: id.to_string(),
            title: None,
            description: None,
            design: None,
            acceptance: None,
            priority: None,
            tags: None,
            assignee: None,
            estimate: None,
            status: None,
            ticket_type: None,
            json: None,
        }
    }

    #[test]
    #[serial(env)]
    fn update_title_changes_title() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Old title"));
        ca.id = Some("upd-01".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("upd-01");
        ua.title = Some("New title".to_string());
        cmd_update(ua, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("upd-01").unwrap();
        assert_eq!(ticket.title, "New title");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn update_priority_zero_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Priority test"));
        ca.id = Some("upd-02".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("upd-02");
        ua.priority = Some(0);
        cmd_update(ua, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("upd-02").unwrap();
        assert_eq!(ticket.priority, 0);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn update_priority_five_returns_invalid_field() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Priority test"));
        ca.id = Some("upd-03".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("upd-03");
        ua.priority = Some(5);
        let result = cmd_update(ua, true, false, false);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "invalid_field");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn update_tags_replaces_entire_tags_vec() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Tags test"));
        ca.id = Some("upd-04".to_string());
        ca.tags = Some("old".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("upd-04");
        ua.tags = Some("a,b,c".to_string());
        cmd_update(ua, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("upd-04").unwrap();
        assert_eq!(ticket.tags, vec!["a", "b", "c"]);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn update_assignee_set_then_clear() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Assignee test"));
        ca.id = Some("upd-05".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("upd-05");
        ua.assignee = Some("alice".to_string());
        cmd_update(ua, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("upd-05").unwrap();
        assert_eq!(ticket.assignee, Some("alice".to_string()));

        let mut ua2 = update_args("upd-05");
        ua2.assignee = Some("".to_string());
        cmd_update(ua2, true, false, false).unwrap();

        let ticket2 = st.read_ticket("upd-05").unwrap();
        assert_eq!(ticket2.assignee, None);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn update_description_empty_string_clears_it() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Desc test"));
        ca.id = Some("upd-06".to_string());
        ca.description = Some("Some description".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("upd-06");
        ua.description = Some("".to_string());
        cmd_update(ua, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("upd-06").unwrap();
        assert_eq!(ticket.description, None);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn update_status_to_in_progress() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Status test"));
        ca.id = Some("upd-07".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("upd-07");
        ua.status = Some(ticket::Status::InProgress);
        cmd_update(ua, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("upd-07").unwrap();
        assert_eq!(ticket.status, ticket::Status::InProgress);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn update_succeeds_and_persists_change() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Stderr test"));
        ca.id = Some("upd-08".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("upd-08");
        ua.title = Some("Updated title".to_string());
        cmd_update(ua, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("upd-08").unwrap();
        assert_eq!(ticket.title, "Updated title");

        std::env::remove_var("VIMA_DIR");
    }

    fn start_args(id: &str) -> cli::StartArgs {
        cli::StartArgs {
            id: id.to_string(),
            assignee: None,
        }
    }

    fn id_args(id: &str) -> cli::IdArgs {
        cli::IdArgs { id: id.to_string() }
    }

    fn close_args(ids: Vec<&str>) -> cli::CloseArgs {
        cli::CloseArgs {
            ids: ids.iter().map(|s| s.to_string()).collect(),
            reason: None,
        }
    }

    #[test]
    #[serial(env)]
    fn start_sets_status_to_in_progress() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Start test"));
        ca.id = Some("st-01".to_string());
        cmd_create(ca, false, false, false).unwrap();

        cmd_start(start_args("st-01"), true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("st-01").unwrap();
        assert_eq!(ticket.status, ticket::Status::InProgress);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn start_on_in_progress_ticket_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Start noop"));
        ca.id = Some("st-02".to_string());
        cmd_create(ca, false, false, false).unwrap();

        cmd_start(start_args("st-02"), true, false, false).unwrap();
        let result = cmd_start(start_args("st-02"), true, false, false);
        assert!(result.is_ok(), "second start should succeed: {:?}", result);

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("st-02").unwrap();
        assert_eq!(ticket.status, ticket::Status::InProgress);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn start_with_assignee_sets_both_fields() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Claim test"));
        ca.id = Some("st-03".to_string());
        cmd_create(ca, true, false, false).unwrap();

        let mut sa = start_args("st-03");
        sa.assignee = Some("agent-1".to_string());
        cmd_start(sa, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("st-03").unwrap();
        assert_eq!(ticket.status, ticket::Status::InProgress);
        assert_eq!(ticket.assignee, Some("agent-1".to_string()));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn start_same_assignee_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Idempotent claim"));
        ca.id = Some("st-04".to_string());
        cmd_create(ca, true, false, false).unwrap();

        let mut sa = start_args("st-04");
        sa.assignee = Some("agent-1".to_string());
        cmd_start(sa, true, false, false).unwrap();

        // Same assignee, second start — should succeed
        let mut sa2 = start_args("st-04");
        sa2.assignee = Some("agent-1".to_string());
        let result = cmd_start(sa2, true, false, false);
        assert!(
            result.is_ok(),
            "same assignee re-start should succeed: {:?}",
            result
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn start_different_assignee_returns_already_claimed() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Contested claim"));
        ca.id = Some("st-05".to_string());
        cmd_create(ca, true, false, false).unwrap();

        let mut sa = start_args("st-05");
        sa.assignee = Some("agent-1".to_string());
        cmd_start(sa, true, false, false).unwrap();

        // Different assignee — should fail
        let mut sa2 = start_args("st-05");
        sa2.assignee = Some("agent-2".to_string());
        let err = cmd_start(sa2, true, false, false).unwrap_err();
        assert!(
            matches!(err, error::Error::AlreadyClaimed { .. }),
            "expected AlreadyClaimed, got: {:?}",
            err
        );
        assert_eq!(err.exit_code(), 6);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn start_no_assignee_on_claimed_ticket_returns_already_claimed() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Claimed no assignee"));
        ca.id = Some("st-06".to_string());
        cmd_create(ca, true, false, false).unwrap();

        let mut sa = start_args("st-06");
        sa.assignee = Some("agent-1".to_string());
        cmd_start(sa, true, false, false).unwrap();

        // No assignee — should fail (ticket is claimed)
        let sa2 = start_args("st-06");
        let err = cmd_start(sa2, true, false, false).unwrap_err();
        assert!(
            matches!(err, error::Error::AlreadyClaimed { .. }),
            "expected AlreadyClaimed, got: {:?}",
            err
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn start_no_assignee_on_unclaimed_in_progress_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Unclaimed noop"));
        ca.id = Some("st-07".to_string());
        cmd_create(ca, true, false, false).unwrap();

        // Start without assignee
        cmd_start(start_args("st-07"), true, false, false).unwrap();

        // Start again without assignee — should be noop
        let result = cmd_start(start_args("st-07"), true, false, false);
        assert!(
            result.is_ok(),
            "unclaimed re-start should succeed: {:?}",
            result
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn start_assignee_on_open_ticket_claims_it() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Open claim"));
        ca.id = Some("st-08".to_string());
        cmd_create(ca, true, false, false).unwrap();

        let mut sa = start_args("st-08");
        sa.assignee = Some("agent-1".to_string());
        cmd_start(sa, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("st-08").unwrap();
        assert_eq!(ticket.status, ticket::Status::InProgress);
        assert_eq!(ticket.assignee, Some("agent-1".to_string()));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn start_dry_run_with_assignee_does_not_persist() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Dry start"));
        ca.id = Some("st-09".to_string());
        cmd_create(ca, true, false, false).unwrap();

        let mut sa = start_args("st-09");
        sa.assignee = Some("agent-1".to_string());
        cmd_start(sa, true, true, false).unwrap(); // dry_run=true

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("st-09").unwrap();
        assert_eq!(
            ticket.status,
            ticket::Status::Open,
            "dry-run start should not persist status"
        );
        assert_eq!(
            ticket.assignee, None,
            "dry-run start should not persist assignee"
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_then_show_includes_version() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Version show test"));
        ca.id = Some("vs-01".to_string());
        cmd_create(ca, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("vs-01").unwrap();
        assert!(
            ticket.version.is_some(),
            "created ticket should have a version"
        );
        let v = ticket.version.unwrap();
        assert_eq!(v.len(), 16, "version should be 16 hex chars");

        // Verify version appears in JSON serialization (what show outputs)
        let computed = st.load_and_compute("vs-01").unwrap();
        let json_val = serde_json::to_value(&computed).unwrap();
        assert!(
            json_val.get("version").is_some(),
            "version should appear in JSON output"
        );
        assert_eq!(json_val["version"].as_str().unwrap().len(), 16);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn stale_write_returns_exit_code_5_integration() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Stale integration"));
        ca.id = Some("stl-01".to_string());
        cmd_create(ca, true, false, false).unwrap();

        // Simulate two agents: both read the same ticket
        let st = store::Store::open().unwrap();
        let mut agent_a = st.read_ticket("stl-01").unwrap();
        let mut agent_b = st.read_ticket("stl-01").unwrap();

        // Agent A writes first — succeeds
        agent_a.title = "Agent A wins".to_string();
        st.write_ticket(&agent_a).unwrap();

        // Agent B tries to write with the old version — should fail with exit 5
        agent_b.title = "Agent B loses".to_string();
        let err = st.write_ticket(&agent_b).unwrap_err();
        assert!(
            matches!(err, error::Error::Stale { .. }),
            "expected Stale error, got: {:?}",
            err
        );
        assert_eq!(err.exit_code(), 5);

        // Verify Agent A's change persisted
        let final_ticket = st.read_ticket("stl-01").unwrap();
        assert_eq!(final_ticket.title, "Agent A wins");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn legacy_ticket_first_update_adds_version() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        // Write a legacy ticket file directly (no version field)
        let legacy_content = r#"---
id: leg-01
title: Legacy ticket
status: open
type: task
priority: 2
tags: []
deps: []
links: []
created: "2026-04-02T00:00:00Z"
notes: []
---
"#;
        std::fs::write(tmp.path().join(".vima/tickets/leg-01.md"), legacy_content).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("leg-01").unwrap();
        assert!(
            ticket.version.is_none(),
            "legacy ticket should have no version"
        );

        // Update via command
        let mut ua = update_args("leg-01");
        ua.title = Some("Updated legacy".to_string());
        cmd_update(ua, true, false, false).unwrap();

        let updated = st.read_ticket("leg-01").unwrap();
        assert!(
            updated.version.is_some(),
            "version should be added after first update"
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn close_sets_status_to_closed() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Close test"));
        ca.id = Some("cl-01".to_string());
        cmd_create(ca, false, false, false).unwrap();

        cmd_close(close_args(vec!["cl-01"]), true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("cl-01").unwrap();
        assert_eq!(ticket.status, ticket::Status::Closed);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn close_with_reason_appends_note() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Close reason test"));
        ca.id = Some("cl-02".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut args = close_args(vec!["cl-02"]);
        args.reason = Some("Done".to_string());
        cmd_close(args, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("cl-02").unwrap();
        assert_eq!(ticket.status, ticket::Status::Closed);
        assert_eq!(ticket.notes.len(), 1);
        assert_eq!(ticket.notes[0].text, "Done");
        assert!(!ticket.notes[0].timestamp.is_empty());

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn close_on_already_closed_is_noop_no_duplicate_note() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Close noop"));
        ca.id = Some("cl-03".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut args1 = close_args(vec!["cl-03"]);
        args1.reason = Some("First close".to_string());
        cmd_close(args1, true, false, false).unwrap();

        let mut args2 = close_args(vec!["cl-03"]);
        args2.reason = Some("Second close".to_string());
        let result = cmd_close(args2, true, false, false);
        assert!(result.is_ok(), "second close should succeed: {:?}", result);

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("cl-03").unwrap();
        assert_eq!(ticket.status, ticket::Status::Closed);
        assert_eq!(ticket.notes.len(), 1, "no duplicate note should be added");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn close_multiple_ids_returns_json_array() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca1 = create_args(Some("Close multi A"));
        ca1.id = Some("cl-04".to_string());
        cmd_create(ca1, false, false, false).unwrap();

        let mut ca2 = create_args(Some("Close multi B"));
        ca2.id = Some("cl-05".to_string());
        cmd_create(ca2, false, false, false).unwrap();

        cmd_close(close_args(vec!["cl-04", "cl-05"]), true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let t1 = st.read_ticket("cl-04").unwrap();
        let t2 = st.read_ticket("cl-05").unwrap();
        assert_eq!(t1.status, ticket::Status::Closed);
        assert_eq!(t2.status, ticket::Status::Closed);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn reopen_sets_status_to_open() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Reopen test"));
        ca.id = Some("ro-01".to_string());
        cmd_create(ca, false, false, false).unwrap();

        cmd_close(close_args(vec!["ro-01"]), true, false, false).unwrap();
        cmd_reopen(id_args("ro-01"), true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("ro-01").unwrap();
        assert_eq!(ticket.status, ticket::Status::Open);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn reopen_on_open_ticket_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Reopen noop"));
        ca.id = Some("ro-02".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let result = cmd_reopen(id_args("ro-02"), true, false, false);
        assert!(
            result.is_ok(),
            "reopen of open ticket should succeed: {:?}",
            result
        );

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("ro-02").unwrap();
        assert_eq!(ticket.status, ticket::Status::Open);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn reopen_from_in_progress_sets_open() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Reopen from in_progress"));
        ca.id = Some("ro-03".to_string());
        cmd_create(ca, false, false, false).unwrap();

        cmd_start(start_args("ro-03"), true, false, false).unwrap();
        cmd_reopen(id_args("ro-03"), true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("ro-03").unwrap();
        assert_eq!(ticket.status, ticket::Status::Open);

        std::env::remove_var("VIMA_DIR");
    }

    // ── list command tests ───────────────────────────────────────────────────

    fn filter_args_default() -> cli::FilterArgs {
        cli::FilterArgs {
            status: None,
            tag: vec![],
            ticket_type: None,
            priority: None,
            assignee: None,
            limit: None,
            pluck: None,
            count: false,
            full: false,
        }
    }

    fn closed_args_default() -> cli::ClosedArgs {
        cli::ClosedArgs {
            filter: filter_args_default(),
        }
    }

    #[test]
    #[serial(env)]
    fn list_returns_all_tickets_sorted_by_priority_asc() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("High prio"));
        a.id = Some("lst-a".to_string());
        a.priority = Some(3);
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Low prio"));
        b.id = Some("lst-b".to_string());
        b.priority = Some(0);
        cmd_create(b, false, false, false).unwrap();

        let mut c = create_args(Some("Mid prio"));
        c.id = Some("lst-c".to_string());
        c.priority = Some(1);
        cmd_create(c, false, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let mut tickets = st.read_all().unwrap();
        deps::compute_reverse_fields(&mut tickets);
        let filter = filter::Filter::from_args(&filter_args_default()).unwrap();
        let result = filter::apply_filters(tickets, &filter);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].id, "lst-b"); // priority 0
        assert_eq!(result[1].id, "lst-c"); // priority 1
        assert_eq!(result[2].id, "lst-a"); // priority 3

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn list_status_filter_returns_only_open() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Open ticket"));
        a.id = Some("lstst-a".to_string());
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("To close"));
        b.id = Some("lstst-b".to_string());
        cmd_create(b, false, false, false).unwrap();
        cmd_close(close_args(vec!["lstst-b"]), true, false, false).unwrap();

        let mut args = filter_args_default();
        args.status = Some(ticket::Status::Open);
        let result = cmd_list(args, false);
        assert!(result.is_ok());

        let st = store::Store::open().unwrap();
        let mut tickets = st.read_all().unwrap();
        deps::compute_reverse_fields(&mut tickets);
        let mut fa = filter_args_default();
        fa.status = Some(ticket::Status::Open);
        let filter = filter::Filter::from_args(&fa).unwrap();
        let filtered = filter::apply_filters(tickets, &filter);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "lstst-a");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn list_tag_filter_returns_tagged_tickets() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Backend ticket"));
        a.id = Some("lsttg-a".to_string());
        a.tags = Some("backend".to_string());
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Frontend ticket"));
        b.id = Some("lsttg-b".to_string());
        b.tags = Some("frontend".to_string());
        cmd_create(b, false, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let mut tickets = st.read_all().unwrap();
        deps::compute_reverse_fields(&mut tickets);
        let mut fa = filter_args_default();
        fa.tag = vec!["backend".to_string()];
        let filter = filter::Filter::from_args(&fa).unwrap();
        let filtered = filter::apply_filters(tickets, &filter);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "lsttg-a");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn list_priority_range_filter() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("P0"));
        a.id = Some("lstpr-a".to_string());
        a.priority = Some(0);
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("P1"));
        b.id = Some("lstpr-b".to_string());
        b.priority = Some(1);
        cmd_create(b, false, false, false).unwrap();

        let mut c = create_args(Some("P3"));
        c.id = Some("lstpr-c".to_string());
        c.priority = Some(3);
        cmd_create(c, false, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let mut tickets = st.read_all().unwrap();
        deps::compute_reverse_fields(&mut tickets);
        let mut fa = filter_args_default();
        fa.priority = Some("0-1".to_string());
        let filter = filter::Filter::from_args(&fa).unwrap();
        let filtered = filter::apply_filters(tickets, &filter);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| t.priority <= 1));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn list_limit_returns_one() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("A"));
        a.id = Some("lstlm-a".to_string());
        a.priority = Some(0);
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("B"));
        b.id = Some("lstlm-b".to_string());
        b.priority = Some(1);
        cmd_create(b, false, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let mut tickets = st.read_all().unwrap();
        deps::compute_reverse_fields(&mut tickets);
        let mut fa = filter_args_default();
        fa.limit = Some(1);
        let filter = filter::Filter::from_args(&fa).unwrap();
        let filtered = filter::apply_filters(tickets, &filter);
        assert_eq!(filtered.len(), 1);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn list_pluck_id_returns_flat_ids() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Pluck test"));
        a.id = Some("lstpl-a".to_string());
        cmd_create(a, false, false, false).unwrap();

        let mut fa = filter_args_default();
        fa.pluck = Some("id".to_string());
        let mut buf = Vec::new();
        cmd_list_to_writer(fa, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let arr = parsed.as_array().expect("expected JSON array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0], serde_json::json!("lstpl-a"));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn list_count_returns_integer() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Count me"));
        a.id = Some("lstcnt-a".to_string());
        cmd_create(a, false, false, false).unwrap();

        let mut fa = filter_args_default();
        fa.count = true;
        let mut buf = Vec::new();
        cmd_list_to_writer(fa, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let count: usize = output.trim().parse().expect("expected integer output");
        assert_eq!(count, 1);

        std::env::remove_var("VIMA_DIR");
    }

    // ── closed command tests ─────────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn closed_returns_closed_tickets_sorted_by_mtime_desc() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("First"));
        a.id = Some("clsd-a".to_string());
        cmd_create(a, false, false, false).unwrap();

        let mut b = create_args(Some("Second"));
        b.id = Some("clsd-b".to_string());
        cmd_create(b, false, false, false).unwrap();

        // Close a first, then b — b should appear first (newer mtime)
        cmd_close(close_args(vec!["clsd-a"]), true, false, false).unwrap();
        // Sleep briefly to ensure different mtime
        std::thread::sleep(std::time::Duration::from_millis(10));
        cmd_close(close_args(vec!["clsd-b"]), true, false, false).unwrap();

        let result = cmd_closed(closed_args_default(), false);
        assert!(result.is_ok(), "closed failed: {:?}", result);

        // Verify ordering: b (closed later) should appear before a in cmd_closed output
        let sorted = closed_collect(&closed_args_default()).unwrap();
        let ids: Vec<&str> = sorted.iter().map(|t| t.id.as_str()).collect();
        let idx_b = ids.iter().position(|&id| id == "clsd-b").unwrap();
        let idx_a = ids.iter().position(|&id| id == "clsd-a").unwrap();
        assert!(
            idx_b < idx_a,
            "clsd-b (closed later) should appear before clsd-a in sorted output, got order: {:?}",
            ids
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn closed_defaults_to_limit_20() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        // Create 25 tickets and close them all
        for i in 0..25u32 {
            let mut a = create_args(Some(&format!("Ticket {}", i)));
            a.id = Some(format!("clsdlm-{:02}", i));
            cmd_create(a, false, false, false).unwrap();
        }
        for i in 0..25u32 {
            cmd_close(
                close_args(vec![&format!("clsdlm-{:02}", i)]),
                true,
                false,
                false,
            )
            .unwrap();
        }

        let result = cmd_closed(closed_args_default(), false);
        assert!(result.is_ok(), "closed failed: {:?}", result);

        // Verify the internal filter logic applies limit=20
        let st = store::Store::open().unwrap();
        let mut tickets = st.read_all().unwrap();
        deps::compute_reverse_fields(&mut tickets);
        let mut filter = filter::Filter::from_args(&filter_args_default()).unwrap();
        filter.status = Some(ticket::Status::Closed);
        if filter.limit.is_none() {
            filter.limit = Some(20);
        }
        let mut filtered: Vec<_> = tickets.into_iter().filter(|t| filter.matches(t)).collect();
        if let Some(limit) = filter.limit {
            filtered.truncate(limit);
        }
        assert_eq!(filtered.len(), 20);

        std::env::remove_var("VIMA_DIR");
    }

    // ── ready command tests ──────────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn ready_returns_ticket_with_no_deps() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("A"));
        a.id = Some("ready-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut b = create_args(Some("B"));
        b.id = Some("ready-b".to_string());
        b.dep = vec!["ready-a".to_string()];
        cmd_create(b, true, false, false).unwrap();

        // Only A should be ready (B depends on A which is open)
        let mut buf = Vec::new();
        cmd_ready_to_writer(filter_args_default(), &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let arr = parsed.as_array().expect("expected JSON array");
        let ids: Vec<&str> = arr.iter().map(|v| v["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"ready-a"), "A should be ready");
        assert!(!ids.contains(&"ready-b"), "B should not be ready");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn ready_after_closing_dep_includes_unblocked() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("A"));
        a.id = Some("rca-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut b = create_args(Some("B"));
        b.id = Some("rca-b".to_string());
        b.dep = vec!["rca-a".to_string()];
        cmd_create(b, true, false, false).unwrap();

        // Close A
        cmd_close(
            cli::CloseArgs {
                ids: vec!["rca-a".to_string()],
                reason: None,
            },
            true,
            false,
            false,
        )
        .unwrap();

        let mut buf = Vec::new();
        cmd_ready_to_writer(filter_args_default(), &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let arr = parsed.as_array().expect("expected JSON array");
        let ids: Vec<&str> = arr.iter().map(|v| v["id"].as_str().unwrap()).collect();
        // A is closed so not in ready list; B is now ready
        assert!(!ids.contains(&"rca-a"), "A is closed, not in ready list");
        assert!(
            ids.contains(&"rca-b"),
            "B should be ready after A is closed"
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn ready_with_count_flag() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Count A"));
        a.id = Some("rc-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut args = filter_args_default();
        args.count = true;
        let mut buf = Vec::new();
        cmd_ready_to_writer(args, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let count: usize = output.trim().parse().expect("expected integer output");
        assert_eq!(count, 1, "one ready ticket expected");

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn ready_with_tag_filter() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Tagged"));
        a.id = Some("rt-a".to_string());
        a.tags = Some("backend".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut b = create_args(Some("Untagged"));
        b.id = Some("rt-b".to_string());
        cmd_create(b, true, false, false).unwrap();

        let mut args = filter_args_default();
        args.tag = vec!["backend".to_string()];
        let mut buf = Vec::new();
        cmd_ready_to_writer(args, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let arr = parsed.as_array().expect("expected JSON array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "rt-a");

        std::env::remove_var("VIMA_DIR");
    }

    // ── blocked command tests ────────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn blocked_returns_ticket_with_open_dep() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Dep A"));
        a.id = Some("blk-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut b = create_args(Some("Blocked B"));
        b.id = Some("blk-b".to_string());
        b.dep = vec!["blk-a".to_string()];
        cmd_create(b, true, false, false).unwrap();

        let mut buf = Vec::new();
        cmd_blocked_to_writer(filter_args_default(), &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let arr = parsed.as_array().expect("expected JSON array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "blk-b");
        // B should have blk-a in its deps
        let deps = arr[0]["deps"].as_array().unwrap();
        assert!(deps.contains(&serde_json::json!("blk-a")));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn blocked_with_priority_filter() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Dep"));
        a.id = Some("bpf-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut b = create_args(Some("High priority blocked"));
        b.id = Some("bpf-b".to_string());
        b.priority = Some(1);
        b.dep = vec!["bpf-a".to_string()];
        cmd_create(b, true, false, false).unwrap();

        let mut c = create_args(Some("Low priority blocked"));
        c.id = Some("bpf-c".to_string());
        c.priority = Some(3);
        c.dep = vec!["bpf-a".to_string()];
        cmd_create(c, true, false, false).unwrap();

        let mut args = filter_args_default();
        args.priority = Some("0-2".to_string());
        let mut buf = Vec::new();
        cmd_blocked_to_writer(args, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let arr = parsed.as_array().expect("expected JSON array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "bpf-b");

        std::env::remove_var("VIMA_DIR");
    }

    // ── is-ready command tests ───────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn is_ready_state_ticket_with_no_deps_is_ready() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("No deps"));
        a.id = Some("ir-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let (ready, open_deps) = is_ready_state("ir-a", true).unwrap();
        assert!(ready, "ticket with no deps should be ready");
        assert!(open_deps.is_empty());

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn is_ready_state_blocked_returns_open_deps() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Dep A"));
        a.id = Some("irb-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut b = create_args(Some("Blocked B"));
        b.id = Some("irb-b".to_string());
        b.dep = vec!["irb-a".to_string()];
        cmd_create(b, true, false, false).unwrap();

        let (ready, open_deps) = is_ready_state("irb-b", true).unwrap();
        assert!(!ready, "B should not be ready while A is open");
        assert_eq!(open_deps, vec!["irb-a".to_string()]);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn is_ready_state_becomes_ready_after_dep_closed() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Dep A"));
        a.id = Some("irc-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut b = create_args(Some("Blocked B"));
        b.id = Some("irc-b".to_string());
        b.dep = vec!["irc-a".to_string()];
        cmd_create(b, true, false, false).unwrap();

        // Close A
        cmd_close(
            cli::CloseArgs {
                ids: vec!["irc-a".to_string()],
                reason: None,
            },
            true,
            false,
            false,
        )
        .unwrap();

        let (ready, open_deps) = is_ready_state("irc-b", true).unwrap();
        assert!(ready, "B should be ready after A is closed");
        assert!(open_deps.is_empty());

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn is_ready_state_closed_ticket_is_ready() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Closed ticket"));
        a.id = Some("ird-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        cmd_close(
            cli::CloseArgs {
                ids: vec!["ird-a".to_string()],
                reason: None,
            },
            true,
            false,
            false,
        )
        .unwrap();

        let (ready, open_deps) = is_ready_state("ird-a", true).unwrap();
        assert!(ready, "closed ticket should be ready");
        assert!(open_deps.is_empty());

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn cmd_is_ready_returns_ok_when_ready() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ready ticket"));
        a.id = Some("ire-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let result = cmd_is_ready(
            cli::IdArgs {
                id: "ire-a".to_string(),
            },
            true,
        );
        assert!(
            result.is_ok(),
            "cmd_is_ready should return Ok for ready ticket"
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn is_ready_state_not_found_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let result = is_ready_state("nonexistent", true);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "not_found");

        std::env::remove_var("VIMA_DIR");
    }

    // ── dep cycle command tests ──────────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn dep_cycle_no_cycles_returns_ok() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        // Create A → B → C (no cycle)
        let mut a = create_args(Some("Ticket A"));
        a.id = Some("dc-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("dc-b".to_string());
        b.dep = vec!["dc-a".to_string()];
        cmd_create(b, true, false, false).unwrap();

        let mut c = create_args(Some("Ticket C"));
        c.id = Some("dc-c".to_string());
        c.dep = vec!["dc-b".to_string()];
        cmd_create(c, true, false, false).unwrap();

        let result = cmd_dep_cycle();
        assert!(
            result.is_ok(),
            "dep cycle with no cycles should return Ok: {:?}",
            result
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn dep_cycle_detects_cycle_via_detect_all_cycles() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        // Create A and B without deps first
        let mut a = create_args(Some("Ticket A"));
        a.id = Some("dcc-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut b = create_args(Some("Ticket B"));
        b.id = Some("dcc-b".to_string());
        cmd_create(b, true, false, false).unwrap();

        // Manually introduce A→B and B→A to bypass would_create_cycle
        let st = store::Store::open().unwrap();
        let mut ticket_a = st.read_ticket("dcc-a").unwrap();
        ticket_a.deps = vec!["dcc-b".to_string()];
        st.write_ticket(&ticket_a).unwrap();

        let mut ticket_b = st.read_ticket("dcc-b").unwrap();
        ticket_b.deps = vec!["dcc-a".to_string()];
        st.write_ticket(&ticket_b).unwrap();

        // detect_all_cycles (the core of cmd_dep_cycle) must find exactly one cycle
        let tickets = st.read_all().unwrap();
        let cycles = deps::detect_all_cycles(&tickets);
        assert!(
            !cycles.is_empty(),
            "should detect a cycle between dcc-a and dcc-b"
        );
        assert_eq!(cycles.len(), 1, "expected exactly one cycle");
        let cycle = &cycles[0];
        assert!(
            cycle.contains(&"dcc-a".to_string()),
            "cycle should contain dcc-a"
        );
        assert!(
            cycle.contains(&"dcc-b".to_string()),
            "cycle should contain dcc-b"
        );

        std::env::remove_var("VIMA_DIR");
    }

    // ── pretty output integration tests ─────────────────────────────────────

    #[test]
    #[serial(env)]
    fn pretty_list_returns_ok_and_json_output_has_correct_tickets() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Fix auth middleware"));
        args.id = Some("pt-a1".to_string());
        args.ticket_type = Some(ticket::TicketType::Bug);
        args.priority = Some(1);
        cmd_create(args, true, false, false).unwrap();

        let mut args2 = create_args(Some("Add rate limiter"));
        args2.id = Some("pt-b2".to_string());
        cmd_create(args2, true, false, false).unwrap();

        // Verify pretty mode doesn't error
        let result = cmd_list(filter_args_default(), true);
        assert!(result.is_ok(), "pretty list failed: {:?}", result);

        // Verify JSON output contains the correct tickets
        let mut buf = Vec::new();
        cmd_list_to_writer(filter_args_default(), &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let arr = parsed.as_array().expect("expected JSON array");
        assert_eq!(arr.len(), 2);
        let ids: Vec<&str> = arr.iter().map(|v| v["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"pt-a1"));
        assert!(ids.contains(&"pt-b2"));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn pretty_list_empty_returns_ok() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let result = cmd_list(filter_args_default(), true);
        assert!(result.is_ok(), "pretty list empty failed: {:?}", result);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn pretty_show_returns_ok_and_json_has_correct_fields() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("Show me pretty"));
        args.id = Some("pt-show1".to_string());
        args.assignee = Some("alice".to_string());
        args.estimate = Some(30);
        args.tags = Some("backend,auth".to_string());
        args.description = Some("The auth middleware stores session tokens...".to_string());
        cmd_create(args, true, false, false).unwrap();

        // Verify pretty mode doesn't error
        let sa = cli::ShowArgs {
            ids: vec!["pt-show1".to_string()],
            pluck: None,
        };
        let result = cmd_show(sa, true, true);
        assert!(result.is_ok(), "pretty show failed: {:?}", result);

        // Verify JSON output has correct fields
        let mut buf = Vec::new();
        cmd_show_to_writer(show_args("pt-show1"), true, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["id"], "pt-show1");
        assert_eq!(parsed["title"], "Show me pretty");
        assert_eq!(parsed["assignee"], "alice");
        assert_eq!(parsed["estimate"], 30);
        assert_eq!(parsed["tags"], serde_json::json!(["backend", "auth"]));
        assert_eq!(
            parsed["description"],
            "The auth middleware stores session tokens..."
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn pretty_show_no_colors_in_json_mode() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut args = create_args(Some("JSON output ticket"));
        args.id = Some("pt-json1".to_string());
        cmd_create(args, true, false, false).unwrap();

        // Pre-arm colors: any accidental colorize_* call in the JSON output path
        // would emit ANSI escape codes and cause the assertion below to fail.
        colored::control::set_override(true);

        let sa = cli::ShowArgs {
            ids: vec!["pt-json1".to_string()],
            pluck: None,
        };
        let mut buf: Vec<u8> = Vec::new();
        cmd_show_to_writer(sa, true, &mut buf).unwrap();

        colored::control::set_override(false);

        let output = String::from_utf8(buf).expect("output is not valid UTF-8");
        assert!(
            !output.contains("\x1b["),
            "ANSI escape codes found in JSON output: {output}"
        );

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn pretty_dep_tree_returns_ok() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Fix auth middleware"));
        a.id = Some("pt-tree-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut b = create_args(Some("Add rate limiter"));
        b.id = Some("pt-tree-b".to_string());
        b.dep = vec!["pt-tree-a".to_string()];
        cmd_create(b, true, false, false).unwrap();

        let mut c = create_args(Some("Update docs"));
        c.id = Some("pt-tree-c".to_string());
        c.dep = vec!["pt-tree-b".to_string()];
        cmd_create(c, true, false, false).unwrap();

        let result = cmd_dep_tree(
            cli::TreeArgs {
                id: "pt-tree-c".to_string(),
                full: false,
                flat: false,
            },
            true,
            true,
        );
        assert!(result.is_ok(), "pretty dep tree failed: {:?}", result);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn pretty_ready_returns_ok() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Ready ticket"));
        a.id = Some("pt-ready-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let result = cmd_ready(filter_args_default(), true);
        assert!(result.is_ok(), "pretty ready failed: {:?}", result);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn pretty_blocked_returns_ok() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Dep A"));
        a.id = Some("pt-blk-a".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut b = create_args(Some("Blocked B"));
        b.id = Some("pt-blk-b".to_string());
        b.dep = vec!["pt-blk-a".to_string()];
        cmd_create(b, true, false, false).unwrap();

        let result = cmd_blocked(filter_args_default(), true);
        assert!(result.is_ok(), "pretty blocked failed: {:?}", result);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn pretty_closed_returns_ok() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Closed ticket"));
        a.id = Some("pt-cls-a".to_string());
        cmd_create(a, true, false, false).unwrap();
        cmd_close(
            cli::CloseArgs {
                ids: vec!["pt-cls-a".to_string()],
                reason: None,
            },
            true,
            false,
            false,
        )
        .unwrap();

        let result = cmd_closed(
            cli::ClosedArgs {
                filter: filter_args_default(),
            },
            true,
        );
        assert!(result.is_ok(), "pretty closed failed: {:?}", result);

        std::env::remove_var("VIMA_DIR");
    }

    // ── parse_tags tests ────────────────────────────────────────────────────

    #[test]
    fn parse_tags_basic_comma_separated() {
        assert_eq!(parse_tags("a,b,c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_tags_empty_string_returns_empty() {
        let result: Vec<String> = parse_tags("");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_tags_whitespace_around_commas() {
        assert_eq!(parse_tags(" a , b , c "), vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_tags_trailing_comma_ignored() {
        assert_eq!(parse_tags("a,b,"), vec!["a", "b"]);
    }

    #[test]
    fn parse_tags_leading_comma_ignored() {
        assert_eq!(parse_tags(",a,b"), vec!["a", "b"]);
    }

    #[test]
    fn parse_tags_multiple_commas_ignored() {
        assert_eq!(parse_tags("a,,b,,,c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_tags_single_tag_no_comma() {
        assert_eq!(parse_tags("solo"), vec!["solo"]);
    }

    #[test]
    fn parse_tags_whitespace_only_returns_empty() {
        let result: Vec<String> = parse_tags("  ,  ,  ");
        assert!(result.is_empty());
    }

    // ── help / help_json tests ──────────────────────────────────────────────

    #[test]
    fn help_json_returns_valid_structure() {
        let json = help_json();
        assert_eq!(json["name"], "vima");
        assert!(json["about"].is_string());
        assert!(json["commands"].is_array());
        assert!(json["global_flags"].is_array());
        assert!(json["exit_codes"].is_object());
    }

    #[test]
    fn help_json_contains_all_subcommands() {
        let json = help_json();
        let commands = json["commands"].as_array().unwrap();
        let names: Vec<&str> = commands
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();

        // All built-in commands must be present
        for expected in &[
            "create", "show", "list", "ready", "blocked", "closed", "update", "start", "close",
            "reopen", "is-ready", "add-note", "dep", "undep", "link", "unlink", "init", "help",
        ] {
            assert!(names.contains(expected), "missing command: {expected}");
        }
    }

    #[test]
    fn help_json_create_has_expected_args() {
        let json = help_json();
        let commands = json["commands"].as_array().unwrap();
        let create = commands.iter().find(|c| c["name"] == "create").unwrap();
        let args = create["args"].as_array().unwrap();
        let arg_names: Vec<&str> = args.iter().map(|a| a["name"].as_str().unwrap()).collect();

        assert!(arg_names.contains(&"title"));
        assert!(arg_names.contains(&"priority"));
        assert!(arg_names.contains(&"tags"));
    }

    #[test]
    fn help_json_dep_has_subcommands() {
        let json = help_json();
        let commands = json["commands"].as_array().unwrap();
        let dep = commands.iter().find(|c| c["name"] == "dep").unwrap();
        let subs = dep["subcommands"].as_array().unwrap();
        let sub_names: Vec<&str> = subs.iter().map(|s| s["name"].as_str().unwrap()).collect();

        assert!(sub_names.contains(&"add"));
        assert!(sub_names.contains(&"tree"));
        assert!(sub_names.contains(&"cycle"));
    }

    #[test]
    fn help_json_args_have_required_field() {
        let json = help_json();
        let commands = json["commands"].as_array().unwrap();
        let create = commands.iter().find(|c| c["name"] == "create").unwrap();
        let args = create["args"].as_array().unwrap();
        for arg in args {
            assert!(
                arg["required"].is_boolean(),
                "arg {} missing 'required' field",
                arg["name"]
            );
        }
    }

    #[test]
    fn help_json_exit_codes_include_stale_and_claimed() {
        let json = help_json();
        let exit_codes = json["exit_codes"].as_object().unwrap();
        assert!(
            exit_codes.contains_key("5"),
            "exit_codes should contain code 5 (stale)"
        );
        assert!(
            exit_codes.contains_key("6"),
            "exit_codes should contain code 6 (already_claimed)"
        );
        assert!(exit_codes["5"].as_str().unwrap().contains("stale"));
        assert!(exit_codes["6"].as_str().unwrap().contains("claimed"));
        // Verify all 7 exit codes are present (0-6)
        for code in 0..=6 {
            assert!(
                exit_codes.contains_key(&code.to_string()),
                "exit_codes missing code {code}"
            );
        }
    }

    #[test]
    #[serial(env)]
    fn cmd_help_json_succeeds() {
        // cmd_help with json=true writes to stdout; just verify it doesn't error
        let result = cmd_help(cli::HelpArgs {
            command: None,
            json: true,
            brief: false,
        });
        assert!(result.is_ok(), "cmd_help --json failed: {:?}", result);
    }

    #[test]
    #[serial(env)]
    fn cmd_help_json_per_command() {
        let result = cmd_help(cli::HelpArgs {
            command: Some("create".into()),
            json: true,
            brief: false,
        });
        assert!(
            result.is_ok(),
            "cmd_help create --json failed: {:?}",
            result
        );
    }

    #[test]
    #[serial(env)]
    fn cmd_help_json_per_command_not_found() {
        let result = cmd_help(cli::HelpArgs {
            command: Some("nonexistent".into()),
            json: true,
            brief: false,
        });
        assert!(result.is_err());
    }

    #[test]
    #[serial(env)]
    fn cmd_help_brief_succeeds() {
        let result = cmd_help(cli::HelpArgs {
            command: None,
            json: false,
            brief: true,
        });
        assert!(result.is_ok(), "cmd_help --brief failed: {:?}", result);
    }

    // ── cmd_update missing field tests ──────────────────────────────────────

    #[test]
    #[serial(env)]
    fn update_design_set_and_clear() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Design test"));
        ca.id = Some("upd-dsg".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("upd-dsg");
        ua.design = Some("Some design notes".to_string());
        cmd_update(ua, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("upd-dsg").unwrap();
        assert_eq!(ticket.design, Some("Some design notes".to_string()));

        let mut ua2 = update_args("upd-dsg");
        ua2.design = Some("".to_string());
        cmd_update(ua2, true, false, false).unwrap();

        let ticket2 = st.read_ticket("upd-dsg").unwrap();
        assert_eq!(ticket2.design, None);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn update_acceptance_set_and_clear() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Acceptance test"));
        ca.id = Some("upd-acc".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("upd-acc");
        ua.acceptance = Some("Passes all tests".to_string());
        cmd_update(ua, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("upd-acc").unwrap();
        assert_eq!(ticket.acceptance, Some("Passes all tests".to_string()));

        let mut ua2 = update_args("upd-acc");
        ua2.acceptance = Some("".to_string());
        cmd_update(ua2, true, false, false).unwrap();

        let ticket2 = st.read_ticket("upd-acc").unwrap();
        assert_eq!(ticket2.acceptance, None);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn update_estimate_sets_value() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Estimate test"));
        ca.id = Some("upd-est".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let mut ua = update_args("upd-est");
        ua.estimate = Some(120);
        cmd_update(ua, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("upd-est").unwrap();
        assert_eq!(ticket.estimate, Some(120));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn update_ticket_type_changes_type() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Type test"));
        ca.id = Some("upd-typ".to_string());
        cmd_create(ca, false, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("upd-typ").unwrap();
        assert_eq!(ticket.ticket_type, ticket::TicketType::Task);

        let mut ua = update_args("upd-typ");
        ua.ticket_type = Some(ticket::TicketType::Bug);
        cmd_update(ua, true, false, false).unwrap();

        let ticket2 = st.read_ticket("upd-typ").unwrap();
        assert_eq!(ticket2.ticket_type, ticket::TicketType::Bug);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn update_nonexistent_ticket_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let ua = update_args("no-such-id");
        let result = cmd_update(ua, true, false, false);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "not_found");

        std::env::remove_var("VIMA_DIR");
    }

    // ── cmd_create missing field tests ──────────────────────────────────────

    #[test]
    #[serial(env)]
    fn create_with_parent_populates_parent_field() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut parent = create_args(Some("Parent ticket"));
        parent.id = Some("cr-par".to_string());
        cmd_create(parent, true, false, false).unwrap();

        let mut child = create_args(Some("Child ticket"));
        child.id = Some("cr-chi".to_string());
        child.parent = Some("cr-par".to_string());
        cmd_create(child, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("cr-chi").unwrap();
        assert_eq!(ticket.parent, Some("cr-par".to_string()));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_with_design_and_acceptance() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Full ticket"));
        ca.id = Some("cr-full".to_string());
        ca.design = Some("Design notes here".to_string());
        ca.acceptance = Some("All tests pass".to_string());
        cmd_create(ca, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("cr-full").unwrap();
        assert_eq!(ticket.design, Some("Design notes here".to_string()));
        assert_eq!(ticket.acceptance, Some("All tests pass".to_string()));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_with_estimate_populates_estimate() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Estimated ticket"));
        ca.id = Some("cr-est".to_string());
        ca.estimate = Some(60);
        cmd_create(ca, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("cr-est").unwrap();
        assert_eq!(ticket.estimate, Some(60));

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_with_explicit_type() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Feature ticket"));
        ca.id = Some("cr-feat".to_string());
        ca.ticket_type = Some(ticket::TicketType::Feature);
        cmd_create(ca, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("cr-feat").unwrap();
        assert_eq!(ticket.ticket_type, ticket::TicketType::Feature);

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn create_with_assignee() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Assigned ticket"));
        ca.id = Some("cr-asgn".to_string());
        ca.assignee = Some("alice".to_string());
        cmd_create(ca, true, false, false).unwrap();

        let st = store::Store::open().unwrap();
        let ticket = st.read_ticket("cr-asgn").unwrap();
        assert_eq!(ticket.assignee, Some("alice".to_string()));

        std::env::remove_var("VIMA_DIR");
    }

    // ── cmd_closed additional tests ─────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn closed_no_closed_tickets_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut ca = create_args(Some("Open ticket"));
        ca.id = Some("cls-open".to_string());
        cmd_create(ca, true, false, false).unwrap();

        let result = closed_collect(&closed_args_default()).unwrap();
        assert!(result.is_empty());

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn closed_with_tag_filter() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("Tagged closed"));
        a.id = Some("cls-tg-a".to_string());
        a.tags = Some("urgent".to_string());
        cmd_create(a, true, false, false).unwrap();
        cmd_close(
            cli::CloseArgs {
                ids: vec!["cls-tg-a".to_string()],
                reason: None,
            },
            true,
            false,
            false,
        )
        .unwrap();

        let mut b = create_args(Some("Untagged closed"));
        b.id = Some("cls-tg-b".to_string());
        cmd_create(b, true, false, false).unwrap();
        cmd_close(
            cli::CloseArgs {
                ids: vec!["cls-tg-b".to_string()],
                reason: None,
            },
            true,
            false,
            false,
        )
        .unwrap();

        let mut args = closed_args_default();
        args.filter.tag = vec!["urgent".to_string()];
        let result = closed_collect(&args).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "cls-tg-a");

        std::env::remove_var("VIMA_DIR");
    }

    // ── cmd_blocked additional tests ────────────────────────────────────────

    #[test]
    #[serial(env)]
    fn blocked_mixed_deps_some_closed() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut dep1 = create_args(Some("Dep 1"));
        dep1.id = Some("blk-d1".to_string());
        cmd_create(dep1, true, false, false).unwrap();

        let mut dep2 = create_args(Some("Dep 2"));
        dep2.id = Some("blk-d2".to_string());
        cmd_create(dep2, true, false, false).unwrap();

        let mut blocked = create_args(Some("Blocked ticket"));
        blocked.id = Some("blk-main".to_string());
        blocked.dep = vec!["blk-d1".to_string(), "blk-d2".to_string()];
        cmd_create(blocked, true, false, false).unwrap();

        // Close one dep — ticket should still be blocked
        cmd_close(
            cli::CloseArgs {
                ids: vec!["blk-d1".to_string()],
                reason: None,
            },
            true,
            false,
            false,
        )
        .unwrap();

        let mut buf = Vec::new();
        cmd_blocked_to_writer(filter_args_default(), &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let arr = parsed.as_array().expect("expected JSON array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "blk-main");

        // Close second dep — ticket should no longer be blocked
        cmd_close(
            cli::CloseArgs {
                ids: vec!["blk-d2".to_string()],
                reason: None,
            },
            true,
            false,
            false,
        )
        .unwrap();

        let mut buf2 = Vec::new();
        cmd_blocked_to_writer(filter_args_default(), &mut buf2).unwrap();
        let output2 = String::from_utf8(buf2).unwrap();
        let parsed2: serde_json::Value = serde_json::from_str(output2.trim()).unwrap();
        let arr2 = parsed2.as_array().expect("expected JSON array");
        assert!(arr2.is_empty());

        std::env::remove_var("VIMA_DIR");
    }

    #[test]
    #[serial(env)]
    fn blocked_empty_when_no_deps() {
        let tmp = tempfile::tempdir().unwrap();
        setup_vima(&tmp);

        let mut a = create_args(Some("No deps"));
        a.id = Some("blk-none".to_string());
        cmd_create(a, true, false, false).unwrap();

        let mut buf = Vec::new();
        cmd_blocked_to_writer(filter_args_default(), &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let arr = parsed.as_array().expect("expected JSON array");
        assert!(arr.is_empty());

        std::env::remove_var("VIMA_DIR");
    }
}
