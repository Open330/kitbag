//! The command surface. See DESIGN.md §4.
//!
//! Every command is a stub until the engine behind it exists; they are listed
//! here first so the shape of the tool is reviewable before it is built.

use anyhow::Result;
use clap::{Parser, Subcommand};
use kitbag_core::Wanted;

#[derive(Parser)]
#[command(
    name = "kitbag",
    about = "Everything this machine holds that is yours",
    version
)]
struct Cli {
    /// Scopes this run takes: a comma-separated list, or `all`.
    #[arg(long, global = true)]
    scope: Option<String>,

    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// What this machine has, grouped by scope, marked against the store
    Status,
    /// What `apply` would change
    Plan,
    /// Make it so
    Apply,
    /// Find personal state that is not tracked yet
    Discover,
    /// Start tracking a path
    Track { path: String },
    /// Send tracked state to the store
    Push,
    /// Write tracked state back here
    Restore,
    /// Permissions, reachability, unscoped files, orphans
    Doctor,
    /// Refuse a commit that would leak the inventory
    Lint,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let wanted = match cli.scope.as_deref() {
        Some(s) => Wanted::parse(s)?,
        None => Wanted::default(),
    };

    let name = match cli.command {
        Command::Status => "status",
        Command::Plan => "plan",
        Command::Apply => "apply",
        Command::Discover => "discover",
        Command::Track { .. } => "track",
        Command::Push => "push",
        Command::Restore => "restore",
        Command::Doctor => "doctor",
        Command::Lint => "lint",
    };

    println!("kitbag {name}: not implemented yet (scopes: {wanted:?})");
    Ok(())
}
