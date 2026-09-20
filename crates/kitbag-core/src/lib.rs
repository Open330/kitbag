//! kitbag - everything this machine holds that is yours.
//!
//! See DESIGN.md. This crate holds the model: who state belongs to (`scope`),
//! how a file declares that for itself (`marker`), and - as the engine grows -
//! what the desired state is and how far the machine is from it.

pub mod collect;
pub mod config;
pub mod envelope;
pub mod lint;
pub mod marker;
pub mod recipe;
pub mod scope;
pub mod state;

pub use collect::{Collected, Item};
pub use config::Config;
pub use envelope::Envelope;
pub use lint::Finding;
pub use marker::Markers;
pub use scope::{Scope, Wanted};
pub use state::State;
