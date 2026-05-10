use clap::{Parser, Subcommand};

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

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => println!("yard init: not yet implemented"),
        Command::Apply => println!("yard apply: not yet implemented"),
    }
}
