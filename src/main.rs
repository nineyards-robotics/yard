use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use yard::adaptors::KeyAction;
use yard::engine::{self, FileReport};
use yard::{ConfigError, EngineError, YardConfig};

#[derive(Parser)]
#[command(name = "yard", version, about = "ROS 2 workspace orchestrator")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Bootstrap a new yard workspace in the current directory.
    Init,
    /// Reconcile managed files against yard.toml.
    Apply,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => {
            eprintln!("yard init: not yet implemented");
            ExitCode::from(1)
        }
        Command::Apply => match run_apply() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::from(1)
            }
        },
    }
}

#[derive(Debug, thiserror::Error)]
enum ApplyError {
    #[error("could not determine current working directory: {0}")]
    Cwd(#[source] std::io::Error),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Engine(#[from] EngineError),
}

fn run_apply() -> Result<(), ApplyError> {
    let cwd = std::env::current_dir().map_err(ApplyError::Cwd)?;
    let yard_toml = cwd.join("yard.toml");
    let config = YardConfig::from_path(&yard_toml)?;
    let report = engine::run(&config, &cwd)?;
    print_report(&cwd, &report.files);
    Ok(())
}

fn print_report(workspace: &PathBuf, files: &[FileReport]) {
    if files.is_empty() {
        println!("yard apply: nothing to reconcile.");
        return;
    }
    for file in files {
        let display = file.path.strip_prefix(workspace).unwrap_or(&file.path);
        println!("{}:", display.display());
        for action in &file.actions {
            print_action(action);
        }
    }
}

fn print_action(action: &KeyAction) {
    match action {
        KeyAction::InSync { key } => println!("  in sync   {key}"),
        KeyAction::Updated { key, .. } => println!("  updated   {key}"),
        KeyAction::Reemitted { key, .. } => println!("  emitted   {key}"),
        KeyAction::Frozen { key } => println!("  frozen    {key}"),
    }
}
