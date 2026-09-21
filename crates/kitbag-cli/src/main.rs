//! The command surface. See DESIGN.md §4.
//!
//! The engine behind these is not built yet; the shape is here first so it can
//! be argued with before it is implemented. Every command that would change
//! something says what it would do and stops unless told otherwise.

mod check;
mod run;
mod trust;
mod ui;

use anyhow::Result;
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use kitbag_core::Wanted;

use ui::Colour;

fn styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Green.on_default() | Effects::BOLD)
        .usage(AnsiColor::Green.on_default() | Effects::BOLD)
        .literal(AnsiColor::Cyan.on_default() | Effects::BOLD)
        .placeholder(AnsiColor::Cyan.on_default())
}

#[derive(Parser)]
#[command(
    name = "kitbag",
    version,
    about = "Everything this machine holds that is yours",
    long_about = "Everything this machine holds that is yours: what it is, whose it is, \
                  and how it gets onto the next machine.\n\n\
                  Packages, configuration, system settings, credentials and app data are \
                  one kind of thing here — each with an owner. A machine declares which \
                  scopes it takes, and that decides what is sent, what is written, and \
                  what a report shows.",
    styles = styles(),
    max_term_width = 100
)]
struct Cli {
    /// Scopes this run takes: a comma-separated list, or `all`
    #[arg(long, global = true, value_name = "LIST", env = "KITBAG_SCOPE")]
    scope: Option<String>,

    /// Secret store to talk to
    #[arg(long, global = true, value_name = "NAME", env = "KITBAG_BACKEND")]
    backend: Option<String>,

    /// Items this machine keeps for itself: a comma-separated list of names
    #[arg(long, global = true, value_name = "LIST", env = "KITBAG_SKIP")]
    skip: Option<String>,

    /// Machine-readable output
    #[arg(long, global = true)]
    json: bool,

    /// When to colour output
    #[arg(long, global = true, value_name = "WHEN", value_enum, default_value_t = ColourChoice::Auto)]
    color: ColourChoice,

    /// Say less
    #[arg(long, short, global = true, conflicts_with = "verbose")]
    quiet: bool,

    /// Say more; repeat for more still
    #[arg(long, short, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum TrustCmd {
    /// Who is trusted where
    List { hosts: Vec<String> },
    /// This machine joins the list
    Register {
        /// Make a new key for this machine first
        #[arg(long)]
        new_key: bool,
    },
    /// Collect every host's key, then give every host the union
    Sync { hosts: Vec<String> },
    /// Drop a key here and everywhere
    Revoke {
        /// A fingerprint or a comment
        targets: Vec<String>,
        /// Hosts to remove it from as well
        #[arg(long, value_name = "HOST")]
        from: Vec<String>,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum ColourChoice {
    Auto,
    Always,
    Never,
}

#[derive(Subcommand)]
enum Command {
    /// What this machine has, grouped by scope, marked against the store
    #[command(visible_alias = "st")]
    Status,

    /// What `apply` would change, and nothing else
    Plan,

    /// Make the machine match the recipes
    Apply {
        /// Only this recipe
        #[arg(long, value_name = "RECIPE")]
        only: Option<String>,
        /// Do not ask first
        #[arg(long, short)]
        yes: bool,
    },

    /// Find personal state that nothing is tracking yet
    Discover {
        /// Add everything found to this machine's config
        #[arg(long)]
        write: bool,
        /// Never propose this path again
        #[arg(long, value_name = "PATH")]
        dismiss: Option<String>,
    },

    /// Start tracking a path
    Track {
        path: String,
        /// Whose it is; omit if the file carries its own `# scope:` marker
        #[arg(long)]
        scope: Option<String>,
        /// Free text, kept beside the item
        #[arg(long)]
        owner: Option<String>,
    },

    /// What differs between this machine and the store, without the values
    #[command(visible_alias = "changed")]
    Diff {
        /// Only these items, by name
        #[arg(value_name = "NAME")]
        only: Vec<String>,
    },

    /// Settle the differences neither side can settle alone, one at a time
    Resolve,

    /// Send tracked state to the store
    Push {
        /// Say what would be sent and stop
        #[arg(long)]
        dry_run: bool,

        /// Only these items, by name — for resolving one at a time
        #[arg(long, value_name = "NAME", num_args = 1..)]
        only: Vec<String>,
    },

    /// Write tracked state back onto this machine
    Restore {
        /// Say what would be sent and stop
        #[arg(long)]
        dry_run: bool,

        /// Only these items, by name — for resolving one at a time
        #[arg(long, value_name = "NAME", num_args = 1..)]
        only: Vec<String>,
    },

    /// Permissions, reachability, unscoped files, orphans in the store
    Doctor,

    /// Refuse the things that must not be committed
    Lint {
        /// Files to check; defaults to everything git tracks
        paths: Vec<String>,
    },

    /// The machines that may log in here
    Trust {
        #[command(subcommand)]
        what: TrustCmd,
    },

    /// Print a shell completion script
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let colour = match cli.color {
        ColourChoice::Always => Colour::Always,
        ColourChoice::Never => Colour::Never,
        ColourChoice::Auto => Colour::resolve(None),
    };

    // The config reads KITBAG_SKIP, so the flag and the variable are the same
    // answer arriving by different routes. Putting the flag into the
    // environment is how they become one route.
    if let Some(list) = cli.skip.as_deref() {
        std::env::set_var("KITBAG_SKIP", list);
    }

    // Only an explicit --scope overrides the machine's own configuration;
    // otherwise the machine file decides what this machine is willing to hold.
    let wanted = match cli.scope.as_deref() {
        Some(s) => Some(Wanted::parse(s)?),
        None => None,
    };

    if let Command::Completions { shell } = cli.command {
        let mut cmd = Cli::command();
        let name = cmd.get_name().to_string();
        clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
        return Ok(());
    }

    let width = terminal_width();

    match cli.command {
        Command::Status => run::status(cli.backend.as_deref(), wanted, colour, width, cli.json)?,
        Command::Plan => run::plan(&repo_root(), colour, width, cli.json)?,
        Command::Apply { ref only, yes } => {
            run::apply(&repo_root(), only.as_deref(), yes, colour, width)?
        }
        Command::Discover { write, ref dismiss } => {
            run::discover(write, dismiss.as_deref(), cli.json)?
        }
        Command::Track {
            ref path,
            ref scope,
            ref owner,
        } => run::track(path, scope.as_deref(), owner.as_deref())?,
        Command::Doctor => check::doctor(cli.backend.as_deref(), cli.json)?,
        Command::Trust { ref what } => match what {
            TrustCmd::List { hosts } => trust::list(hosts)?,
            TrustCmd::Register { new_key } => trust::register(*new_key)?,
            TrustCmd::Sync { hosts } => trust::sync(hosts)?,
            TrustCmd::Revoke { targets, from } => trust::revoke_cmd(targets, from)?,
        },
        Command::Lint { ref paths } => check::lint(paths, cli.json)?,
        Command::Diff { ref only } => run::diff(cli.backend.as_deref(), only, colour)?,
        Command::Resolve => run::resolve(cli.backend.as_deref(), wanted, colour)?,
        Command::Push { dry_run, ref only } => {
            run::push(cli.backend.as_deref(), wanted, dry_run, only, colour)?
        }
        Command::Restore { dry_run, ref only } => {
            run::restore(cli.backend.as_deref(), wanted, dry_run, only, colour)?
        }
        other => {
            println!("  kitbag {} is not implemented yet.", name_of(&other));
            println!("  Implemented so far: status, plan, completions.");
        }
    }
    Ok(())
}

/// Where the recipes live: the repository this was run from, or one named.
fn repo_root() -> std::path::PathBuf {
    std::env::var_os("KITBAG_REPO")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

fn name_of(c: &Command) -> &'static str {
    match c {
        Command::Status => "status",
        Command::Plan => "plan",
        Command::Apply { .. } => "apply",
        Command::Discover { .. } => "discover",
        Command::Track { .. } => "track",
        Command::Diff { .. } => "diff",
        Command::Resolve => "resolve",
        Command::Push { .. } => "push",
        Command::Restore { .. } => "restore",
        Command::Doctor => "doctor",
        Command::Lint { .. } => "lint",
        Command::Trust { .. } => "trust",
        Command::Completions { .. } => "completions",
    }
}

fn terminal_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(100)
}
