//! The command surface. See DESIGN.md §4.
//!
//! The engine behind these is not built yet; the shape is here first so it can
//! be argued with before it is implemented. Every command that would change
//! something says what it would do and stops unless told otherwise.

mod ui;

use anyhow::Result;
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use kitbag_core::{Scope, Wanted};

use ui::{Colour, Group, Mark, Row};

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
        /// Only this kind of resource: pkg, file, defaults, secret
        #[arg(long, value_name = "KIND")]
        only: Option<String>,
    },

    /// Find personal state that nothing is tracking yet
    Discover,

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

    /// Send tracked state to the store
    Push {
        /// Say what would be sent and stop
        #[arg(long)]
        dry_run: bool,
    },

    /// Write tracked state back onto this machine
    Restore {
        /// Say what would be written and stop
        #[arg(long)]
        dry_run: bool,
    },

    /// Permissions, reachability, unscoped files, orphans in the store
    Doctor,

    /// Refuse the things that must not be committed
    Lint {
        /// Files to check; defaults to everything git tracks
        paths: Vec<String>,
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

    let wanted = match cli.scope.as_deref() {
        Some(s) => Wanted::parse(s)?,
        None => Wanted::default(),
    };

    if let Command::Completions { shell } = cli.command {
        let mut cmd = Cli::command();
        let name = cmd.get_name().to_string();
        clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
        return Ok(());
    }

    // Until the engine exists, every command shows the shape of its answer
    // rather than pretending to have one.
    let sample = vec![
        Group {
            scope: Scope::Personal,
            owner: None,
            rows: vec![Row {
                mark: Some(Mark::Unchanged),
                name: "env:example".into(),
                detail: "EXAMPLE_TOKEN EXAMPLE_URL".into(),
            }],
        },
        Group {
            scope: Scope::Work,
            owner: Some("acme".into()),
            rows: vec![Row {
                mark: Some(Mark::New),
                name: "file:example".into(),
                detail: "binary, 24 bytes → ~/.config/example".into(),
            }],
        },
    ];

    let width = terminal_width();
    print!("{}", ui::render(&sample, colour, width));
    println!();
    println!("  kitbag is not implemented yet — this is the shape of the answer.");
    println!("  scopes this run would take: {wanted:?}");
    Ok(())
}

fn terminal_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(100)
}
