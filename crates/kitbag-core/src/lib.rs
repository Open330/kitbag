//! kitbag - everything this machine holds that is yours.
//!
//! See DESIGN.md. This crate holds the model: who state belongs to (`scope`),
//! how a file declares that for itself (`marker`), and - as the engine grows -
//! what the desired state is and how far the machine is from it.

pub mod collect;
pub mod config;
pub mod difference;
pub mod envelope;
pub mod exec;
pub mod ledger;
pub mod lint;
pub mod marker;
pub mod recipe;
pub mod scope;
pub mod state;

pub use collect::{Collected, Item};
pub use config::Config;
pub use envelope::{payload_hash, Envelope};

/// What this machine is, in the word a `platform` field is written in.
///
/// It decides what a restore is willing to write: a macOS keychain and a
/// bundle addressed to `~/Library/Application Support` are not state a Linux
/// server or a Windows box has anywhere to put, and writing them there is
/// worse than skipping them, because it looks like it worked.
pub fn this_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}
pub use lint::Finding;
pub use marker::Markers;
pub use scope::{Scope, Wanted};
pub use state::State;
