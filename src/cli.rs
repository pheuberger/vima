use clap::{Args, Parser, Subcommand};

use crate::ticket::{Status, TicketType};

#[derive(Parser, Debug)]
#[command(name = "vima", about = "AI-agent-first ticketing CLI", disable_help_subcommand = true)]
pub struct Cli {
    /// Output in human-readable pretty format
    #[arg(long, global = true)]
    pub pretty: bool,

    /// Use exact ID matching (no partial match)
    #[arg(long, global = true, env = "VIMA_EXACT")]
    pub exact: bool,

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
    Start(IdArgs),
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
    /// Ticket title
    pub title: Option<String>,

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
    #[arg(long)]
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
}

#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// Ticket ID
    pub id: String,

    /// New title
    #[arg(long)]
    pub title: Option<String>,

    /// New description
    #[arg(long)]
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
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Ticket ID
    pub id: String,

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
}

#[derive(Args, Debug)]
pub struct IdArgs {
    /// Ticket ID
    pub id: String,
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
pub struct InitArgs {
    /// Also create a .vima/instructions.md template
    #[arg(long)]
    pub with_instructions: bool,
}

#[derive(Args, Debug)]
pub struct HelpArgs {
    /// Subcommand to show help for
    pub command: Option<String>,
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
}
