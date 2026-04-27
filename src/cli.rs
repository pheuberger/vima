use clap::{Args, Parser, Subcommand};

use crate::ticket::{Status, TicketType};

#[derive(Parser, Debug)]
#[command(
    name = "vima",
    about = "AI-agent-first ticketing CLI",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Human-only: pretty-print output (agents: use default JSON + --pluck instead)
    #[arg(long, global = true)]
    pub pretty: bool,

    /// Use exact ID matching (no partial match)
    #[arg(long, global = true, env = "VIMA_EXACT")]
    pub exact: bool,

    /// Preview changes without persisting (dry run)
    #[arg(long, global = true)]
    pub dry_run: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Create a new ticket
    Create(CreateArgs),
    /// Show a ticket
    Show(ShowArgs),
    /// List tickets
    List(FilterArgs),
    /// List tickets that are ready (no open deps)
    Ready(FilterArgs),
    /// List tickets that are blocked
    Blocked(FilterArgs),
    /// List closed tickets
    Closed(ClosedArgs),
    /// Update a ticket
    Update(UpdateArgs),
    /// Start work on a ticket (set status to in_progress)
    Start(StartArgs),
    /// Close a ticket
    Close(CloseArgs),
    /// Reopen a closed ticket
    Reopen(IdArgs),
    /// Check if a ticket is ready
    IsReady(IdArgs),
    /// Add a note to a ticket
    AddNote(AddNoteArgs),
    /// Manage dependencies
    Dep(DepArgs),
    /// Remove a dependency
    Undep(UndepArgs),
    /// Link two tickets symmetrically
    Link(LinkArgs),
    /// Unlink two tickets
    Unlink(LinkArgs),
    /// Initialize a vima store in the current directory
    Init(InitArgs),
    /// Show help
    Help(HelpArgs),
    /// External plugin subcommand
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Args, Debug)]
pub struct CreateArgs {
    /// Ticket title (positional)
    pub title: Option<String>,

    /// Ticket title (named flag, alias for positional)
    #[arg(long = "title", hide = true)]
    pub title_flag: Option<String>,

    /// Ticket type
    #[arg(short = 't', long = "type")]
    pub ticket_type: Option<TicketType>,

    /// Priority (0=critical, 1=high, 2=medium, 3=low, 4=backlog)
    #[arg(short = 'p', long)]
    pub priority: Option<u8>,

    /// Assignee
    #[arg(short = 'a', long)]
    pub assignee: Option<String>,

    /// Estimate in minutes
    #[arg(short = 'e', long)]
    pub estimate: Option<u32>,

    /// Comma-separated tags
    #[arg(long)]
    pub tags: Option<String>,

    /// Description
    #[arg(long, alias = "body")]
    pub description: Option<String>,

    /// Design notes
    #[arg(long)]
    pub design: Option<String>,

    /// Acceptance criteria
    #[arg(long)]
    pub acceptance: Option<String>,

    /// Dependencies (ticket IDs)
    #[arg(long = "dep")]
    pub dep: Vec<String>,

    /// Tickets this one blocks
    #[arg(long)]
    pub blocks: Vec<String>,

    /// Parent ticket ID
    #[arg(long)]
    pub parent: Option<String>,

    /// Explicit ticket ID
    #[arg(long)]
    pub id: Option<String>,

    /// Batch create from JSON
    #[arg(long)]
    pub batch: bool,

    /// Create from JSON object (all fields in one argument)
    #[arg(long)]
    pub json: Option<String>,
}

#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// Ticket ID
    pub id: String,

    /// New title
    #[arg(long)]
    pub title: Option<String>,

    /// New description
    #[arg(long, alias = "body")]
    pub description: Option<String>,

    /// New design notes
    #[arg(long)]
    pub design: Option<String>,

    /// New acceptance criteria
    #[arg(long)]
    pub acceptance: Option<String>,

    /// New priority
    #[arg(short = 'p', long)]
    pub priority: Option<u8>,

    /// New tags (comma-separated)
    #[arg(long)]
    pub tags: Option<String>,

    /// New assignee
    #[arg(short = 'a', long)]
    pub assignee: Option<String>,

    /// New estimate in minutes
    #[arg(short = 'e', long)]
    pub estimate: Option<u32>,

    /// New status
    #[arg(long)]
    pub status: Option<Status>,

    /// New type
    #[arg(short = 't', long = "type")]
    pub ticket_type: Option<TicketType>,

    /// Update from JSON object (all fields in one argument)
    #[arg(long)]
    pub json: Option<String>,
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Ticket IDs (one or more). Single ID returns one JSON object; multiple return a JSON array.
    #[arg(required = true, num_args = 1..)]
    pub ids: Vec<String>,

    /// Pluck a specific field from JSON output
    #[arg(long)]
    pub pluck: Option<String>,
}

#[derive(Args, Debug)]
pub struct FilterArgs {
    /// Filter by status
    #[arg(long)]
    pub status: Option<Status>,

    /// Filter by tag
    #[arg(short = 'T', long = "tag")]
    pub tag: Vec<String>,

    /// Filter by type
    #[arg(short = 't', long = "type")]
    pub ticket_type: Option<TicketType>,

    /// Filter by priority (supports ranges like "0-2")
    #[arg(short = 'p', long)]
    pub priority: Option<String>,

    /// Filter by assignee
    #[arg(short = 'a', long)]
    pub assignee: Option<String>,

    /// Limit number of results
    #[arg(long)]
    pub limit: Option<usize>,

    /// Pluck a specific field from JSON output
    #[arg(long)]
    pub pluck: Option<String>,

    /// Print count only
    #[arg(long)]
    pub count: bool,

    /// Output full ticket JSON (include description, design, acceptance, notes, body)
    #[arg(long)]
    pub full: bool,
}

#[derive(Args, Debug)]
pub struct IdArgs {
    /// Ticket ID
    pub id: String,
}

#[derive(Args, Debug)]
pub struct StartArgs {
    /// Ticket ID
    pub id: String,

    /// Claim the ticket for this assignee (fails if already claimed by another)
    #[arg(short = 'a', long)]
    pub assignee: Option<String>,
}

#[derive(Args, Debug)]
pub struct AddNoteArgs {
    /// Ticket ID
    pub id: String,

    /// Note text (reads from stdin if omitted)
    pub text: Option<String>,
}

#[derive(Args, Debug)]
pub struct LinkArgs {
    /// First ticket ID
    pub id_a: String,

    /// Second ticket ID
    pub id_b: String,
}

#[derive(Args, Debug)]
pub struct UndepArgs {
    /// Ticket ID
    pub id: String,

    /// Dependency ticket ID to remove
    pub dep_id: String,
}

#[derive(Args, Debug)]
pub struct ClosedArgs {
    #[command(flatten)]
    pub filter: FilterArgs,
}

#[derive(Args, Debug)]
pub struct CloseArgs {
    /// Ticket IDs to close
    pub ids: Vec<String>,

    /// Reason for closing
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Args, Debug)]
pub struct InitArgs {}

#[derive(Args, Debug)]
pub struct HelpArgs {
    /// Subcommand to show help for
    pub command: Option<String>,

    /// Output help as JSON (for agent consumption)
    #[arg(long)]
    pub json: bool,

    /// Output brief index: command names and one-line descriptions only
    #[arg(long)]
    pub brief: bool,
}

#[derive(Args, Debug)]
pub struct DepArgs {
    #[command(subcommand)]
    pub command: DepCommands,
}

#[derive(Subcommand, Debug)]
pub enum DepCommands {
    /// Add a dependency
    Add(AddDepArgs),
    /// Show dependency tree
    Tree(TreeArgs),
    /// Detect dependency cycles
    Cycle,
}

#[derive(Args, Debug)]
pub struct AddDepArgs {
    /// Ticket ID
    pub id: String,

    /// Dependency ticket ID
    pub dep_id: String,

    /// Record this as: id blocks dep_id (reverse direction)
    #[arg(long)]
    pub blocks: bool,
}

#[derive(Args, Debug)]
pub struct TreeArgs {
    /// Ticket ID
    pub id: String,

    /// Show full transitive tree
    #[arg(long)]
    pub full: bool,

    /// Output as flat array of {id, parent_id, depth, status, title} objects
    #[arg(long)]
    pub flat: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    // Helper to parse CLI args
    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    // 1. Valid create with priority 0-4
    #[test]
    fn create_valid_priority_values() {
        for p in 0..=4u8 {
            let cli = parse(&["vima", "create", "Title", "-p", &p.to_string()]).unwrap();
            if let Commands::Create(args) = cli.command {
                assert_eq!(args.priority, Some(p));
            } else {
                panic!("Expected Create command");
            }
        }
    }

    // 2. Priority value 255 is accepted at clap level (u8 range), validation is in main.rs
    #[test]
    fn create_priority_overflow_rejected() {
        // Values > 255 overflow u8 and are rejected by clap
        let result = parse(&["vima", "create", "Title", "-p", "256"]);
        assert!(result.is_err());
    }

    // 3. Negative priority rejected (not a valid u8)
    #[test]
    fn create_negative_priority_rejected() {
        let result = parse(&["vima", "create", "Title", "-p", "-1"]);
        assert!(result.is_err());
    }

    // 4. Invalid ticket type string rejected by clap ValueEnum
    #[test]
    fn create_invalid_ticket_type_rejected() {
        let result = parse(&["vima", "create", "Title", "-t", "story"]);
        assert!(result.is_err());
    }

    // 5. Valid ticket types accepted
    #[test]
    fn create_valid_ticket_types() {
        for t in &["bug", "feature", "task", "epic", "chore"] {
            let cli = parse(&["vima", "create", "Title", "-t", t]).unwrap();
            if let Commands::Create(args) = cli.command {
                assert!(args.ticket_type.is_some());
            } else {
                panic!("Expected Create command");
            }
        }
    }

    // 6. Valid estimate values accepted
    #[test]
    fn create_valid_estimate() {
        let cli = parse(&["vima", "create", "Title", "-e", "120"]).unwrap();
        if let Commands::Create(args) = cli.command {
            assert_eq!(args.estimate, Some(120));
        } else {
            panic!("Expected Create command");
        }
    }

    // 7. Invalid estimate (negative) rejected
    #[test]
    fn create_negative_estimate_rejected() {
        let result = parse(&["vima", "create", "Title", "-e", "-5"]);
        assert!(result.is_err());
    }

    // 8. --exact flag parsed correctly
    #[test]
    fn exact_flag_parsed() {
        let cli = parse(&["vima", "--exact", "show", "vi-1234"]).unwrap();
        assert!(cli.exact);
    }

    // 9. --pretty flag parsed correctly
    #[test]
    fn pretty_flag_parsed() {
        let cli = parse(&["vima", "--pretty", "list"]).unwrap();
        assert!(cli.pretty);
    }

    // 10. FilterArgs: --tag, --type, --priority, --assignee, --count, --pluck
    #[test]
    fn filter_args_parsed_correctly() {
        let cli = parse(&[
            "vima", "list", "--tag", "backend", "--tag", "urgent", "-t", "bug", "-p", "0-2", "-a",
            "alice", "--count", "--limit", "10",
        ])
        .unwrap();
        if let Commands::List(f) = cli.command {
            assert_eq!(f.tag, vec!["backend", "urgent"]);
            assert_eq!(f.ticket_type, Some(TicketType::Bug));
            assert_eq!(f.priority, Some("0-2".to_string()));
            assert_eq!(f.assignee, Some("alice".to_string()));
            assert!(f.count);
            assert_eq!(f.limit, Some(10));
        } else {
            panic!("Expected List command");
        }
    }

    // 11. Multiple --dep flags on create
    #[test]
    fn create_multiple_deps() {
        let cli = parse(&[
            "vima", "create", "Title", "--dep", "vi-0001", "--dep", "vi-0002", "--dep", "vi-0003",
        ])
        .unwrap();
        if let Commands::Create(args) = cli.command {
            assert_eq!(args.dep, vec!["vi-0001", "vi-0002", "vi-0003"]);
        } else {
            panic!("Expected Create command");
        }
    }

    // 12. Multiple --blocks flags on create
    #[test]
    fn create_multiple_blocks() {
        let cli = parse(&[
            "vima", "create", "Title", "--blocks", "vi-0001", "--blocks", "vi-0002",
        ])
        .unwrap();
        if let Commands::Create(args) = cli.command {
            assert_eq!(args.blocks, vec!["vi-0001", "vi-0002"]);
        } else {
            panic!("Expected Create command");
        }
    }

    // 13. Subcommand names exist (help text generation)
    #[test]
    fn all_subcommands_recognized() {
        let subcommands = [
            "create", "show", "list", "ready", "blocked", "closed", "update", "start", "close",
            "reopen", "is-ready", "add-note", "dep", "undep", "link", "unlink", "init", "help",
        ];
        for sub in &subcommands {
            // Each subcommand without required args should give a usage error, not "unknown subcommand"
            let result = parse(&["vima", sub]);
            // Some subcommands need no args (list, ready, blocked, init, help)
            // Others will fail with missing required arg — but NOT with "unknown subcommand"
            if let Err(e) = &result {
                let msg = e.to_string();
                assert!(
                    !msg.contains("unrecognized subcommand"),
                    "Subcommand '{}' not recognized: {}",
                    sub,
                    msg
                );
            }
        }
    }

    // 14. Invalid status string rejected
    #[test]
    fn invalid_status_rejected() {
        let result = parse(&["vima", "list", "--status", "deleted"]);
        assert!(result.is_err());
    }

    // 15. Valid status values accepted
    #[test]
    fn valid_status_values() {
        for s in &["open", "in_progress", "closed"] {
            let cli = parse(&["vima", "list", "--status", s]).unwrap();
            if let Commands::List(f) = cli.command {
                assert!(f.status.is_some());
            } else {
                panic!("Expected List command");
            }
        }
    }

    // 16. Dep subcommand: add with --blocks flag
    #[test]
    fn dep_add_blocks_flag() {
        let cli = parse(&["vima", "dep", "add", "vi-0001", "vi-0002", "--blocks"]).unwrap();
        if let Commands::Dep(dep_args) = cli.command {
            if let DepCommands::Add(add) = dep_args.command {
                assert_eq!(add.id, "vi-0001");
                assert_eq!(add.dep_id, "vi-0002");
                assert!(add.blocks);
            } else {
                panic!("Expected Add subcommand");
            }
        } else {
            panic!("Expected Dep command");
        }
    }

    // 17. Close with multiple IDs and reason
    #[test]
    fn close_multiple_ids_with_reason() {
        let cli = parse(&[
            "vima",
            "close",
            "vi-0001",
            "vi-0002",
            "--reason",
            "duplicate",
        ])
        .unwrap();
        if let Commands::Close(args) = cli.command {
            assert_eq!(args.ids, vec!["vi-0001", "vi-0002"]);
            assert_eq!(args.reason, Some("duplicate".to_string()));
        } else {
            panic!("Expected Close command");
        }
    }

    // 18. Show with --pluck
    #[test]
    fn show_pluck_field() {
        let cli = parse(&["vima", "show", "vi-1234", "--pluck", "title"]).unwrap();
        if let Commands::Show(args) = cli.command {
            assert_eq!(args.ids, vec!["vi-1234"]);
            assert_eq!(args.pluck, Some("title".to_string()));
        } else {
            panic!("Expected Show command");
        }
    }

    // 19. No subcommand gives error
    #[test]
    fn no_subcommand_errors() {
        let result = parse(&["vima"]);
        assert!(result.is_err());
    }

    // 20. Tree with --full flag
    #[test]
    fn dep_tree_full_flag() {
        let cli = parse(&["vima", "dep", "tree", "vi-0001", "--full"]).unwrap();
        if let Commands::Dep(dep_args) = cli.command {
            if let DepCommands::Tree(tree) = dep_args.command {
                assert_eq!(tree.id, "vi-0001");
                assert!(tree.full);
            } else {
                panic!("Expected Tree subcommand");
            }
        } else {
            panic!("Expected Dep command");
        }
    }
}
