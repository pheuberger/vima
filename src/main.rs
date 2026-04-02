mod cli;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Create(_) => {
            eprintln!("not implemented: create");
            std::process::exit(1);
        }
        Commands::Show(_) => {
            eprintln!("not implemented: show");
            std::process::exit(1);
        }
        Commands::List(_) => {
            eprintln!("not implemented: list");
            std::process::exit(1);
        }
        Commands::Ready(_) => {
            eprintln!("not implemented: ready");
            std::process::exit(1);
        }
        Commands::Blocked(_) => {
            eprintln!("not implemented: blocked");
            std::process::exit(1);
        }
        Commands::Closed(_) => {
            eprintln!("not implemented: closed");
            std::process::exit(1);
        }
        Commands::Update(_) => {
            eprintln!("not implemented: update");
            std::process::exit(1);
        }
        Commands::Start(_) => {
            eprintln!("not implemented: start");
            std::process::exit(1);
        }
        Commands::Close(_) => {
            eprintln!("not implemented: close");
            std::process::exit(1);
        }
        Commands::Reopen(_) => {
            eprintln!("not implemented: reopen");
            std::process::exit(1);
        }
        Commands::IsReady(_) => {
            eprintln!("not implemented: is-ready");
            std::process::exit(1);
        }
        Commands::AddNote(_) => {
            eprintln!("not implemented: add-note");
            std::process::exit(1);
        }
        Commands::Dep(_) => {
            eprintln!("not implemented: dep");
            std::process::exit(1);
        }
        Commands::Undep(_) => {
            eprintln!("not implemented: undep");
            std::process::exit(1);
        }
        Commands::Link(_) => {
            eprintln!("not implemented: link");
            std::process::exit(1);
        }
        Commands::Unlink(_) => {
            eprintln!("not implemented: unlink");
            std::process::exit(1);
        }
        Commands::Init(_) => {
            eprintln!("not implemented: init");
            std::process::exit(1);
        }
        Commands::Help(_) => {
            eprintln!("not implemented: help");
            std::process::exit(1);
        }
        Commands::External(args) => {
            eprintln!("not implemented: {}", args[0]);
            std::process::exit(1);
        }
    }
}
