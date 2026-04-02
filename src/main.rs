mod cli;
mod error;
mod filter;
mod id;
mod store;
mod ticket;

use clap::Parser;
use cli::{Cli, Commands};
use error::{Error, Result};

fn cmd_init() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let vima_dir = cwd.join(".vima");
    let tickets_dir = vima_dir.join("tickets");

    std::fs::create_dir_all(&tickets_dir)?;

    let config_path = vima_dir.join("config.yml");
    if !config_path.exists() {
        let prefix = id::get_prefix(&cwd)?;
        std::fs::write(&config_path, format!("prefix: {}\n", prefix))?;
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
        Commands::Init(_) => cmd_init(),
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

    #[test]
    #[serial(env)]
    fn init_creates_vima_directory_structure() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init().unwrap();

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

        cmd_init().unwrap();

        let config = std::fs::read_to_string(project_dir.join(".vima/config.yml")).unwrap();
        assert!(config.contains("prefix: mp"), "config was: {config}");
    }

    #[test]
    #[serial(env)]
    fn init_idempotent_does_not_overwrite_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init().unwrap();

        // Overwrite config with custom prefix
        std::fs::write(tmp.path().join(".vima/config.yml"), "prefix: custom\n").unwrap();

        // Run init again — must not overwrite config
        cmd_init().unwrap();

        let config = std::fs::read_to_string(tmp.path().join(".vima/config.yml")).unwrap();
        assert!(config.contains("prefix: custom"), "config was: {config}");
    }

    #[test]
    #[serial(env)]
    fn init_idempotent_no_error_on_second_run() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::env::remove_var("VIMA_DIR");

        cmd_init().unwrap();
        cmd_init().unwrap();
    }
}
