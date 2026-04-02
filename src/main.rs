mod cli;
mod error;
mod filter;
mod id;
mod store;
mod ticket;

use clap::Parser;
use cli::{Cli, Commands};
use error::{Error, Result};

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
        Commands::Init(_) => Err(Error::InvalidField("not implemented: init".into())),
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
