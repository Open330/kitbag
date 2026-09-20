//! Who a piece of state belongs to, and which machine may hold it.
//!
//! This is the one idea the tool is built around. Everything else — packages,
//! files, secrets — is filtered through it.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Whose state this is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Yours.
    Personal,
    /// An employer's or a client's.
    Work,
    /// An account someone else owns that you were given access to.
    Shared,
    /// One artifact holding several lives at once, because it cannot be split:
    /// an account bundle, an OTP vault, a GPG key with more than one identity.
    /// It has to say which lives it spans, so the scope that means "I could not
    /// separate this" never hides what is inside it.
    Mixed { spans: Vec<String> },
    /// This machine only. Never leaves it: a cached session, a legacy path kept
    /// for a script that still reads it.
    Local,
}

impl Scope {
    pub fn name(&self) -> &'static str {
        match self {
            Scope::Personal => "personal",
            Scope::Work => "work",
            Scope::Shared => "shared",
            Scope::Mixed { .. } => "mixed",
            Scope::Local => "local",
        }
    }

    /// Machine-local state is never sent anywhere.
    pub fn travels(&self) -> bool {
        !matches!(self, Scope::Local)
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("unknown scope `{0}` (expected personal, work, shared, mixed or local)")]
pub struct UnknownScope(pub String);

impl FromStr for Scope {
    type Err = UnknownScope;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "personal" => Ok(Scope::Personal),
            "work" => Ok(Scope::Work),
            "shared" => Ok(Scope::Shared),
            "mixed" => Ok(Scope::Mixed { spans: Vec::new() }),
            "local" => Ok(Scope::Local),
            other => Err(UnknownScope(other.to_string())),
        }
    }
}

/// What a machine is willing to hold.
///
/// Absent configuration this is `personal` alone: a machine that was never told
/// otherwise must not end up with an employer's credentials on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted {
    All,
    Only(Vec<Scope>),
}

impl Default for Wanted {
    fn default() -> Self {
        Wanted::Only(vec![Scope::Personal])
    }
}

impl Wanted {
    /// Parses `all`, or a comma-separated list.
    pub fn parse(s: &str) -> Result<Self, UnknownScope> {
        if s.trim().eq_ignore_ascii_case("all") {
            return Ok(Wanted::All);
        }
        s.split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(Scope::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map(Wanted::Only)
    }

    /// Does a machine that wants these scopes take this item?
    ///
    /// `mixed` is taken by any machine that takes anything: the artifact cannot
    /// be split from out here, so refusing it would mean refusing the personal
    /// half along with the rest. `local` never travels, so it is never taken
    /// from a store — it is only ever already here.
    pub fn accepts(&self, scope: &Scope) -> bool {
        match scope {
            Scope::Local => false,
            Scope::Mixed { .. } => true,
            other => match self {
                Wanted::All => true,
                Wanted::Only(list) => list.iter().any(|s| s.name() == other.name()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_told_nothing_takes_only_personal() {
        let w = Wanted::default();
        assert!(w.accepts(&Scope::Personal));
        assert!(!w.accepts(&Scope::Work));
        assert!(!w.accepts(&Scope::Shared));
    }

    #[test]
    fn mixed_is_taken_by_anyone_who_takes_anything() {
        let spans = Scope::Mixed {
            spans: vec!["personal".into(), "work".into()],
        };
        assert!(Wanted::default().accepts(&spans));
        assert!(Wanted::parse("work").unwrap().accepts(&spans));
        assert!(Wanted::All.accepts(&spans));
    }

    #[test]
    fn local_never_travels() {
        assert!(!Scope::Local.travels());
        assert!(!Wanted::All.accepts(&Scope::Local));
    }

    #[test]
    fn lists_and_all() {
        let w = Wanted::parse("personal, work").unwrap();
        assert!(w.accepts(&Scope::Personal));
        assert!(w.accepts(&Scope::Work));
        assert!(!w.accepts(&Scope::Shared));
        assert!(Wanted::parse("all").unwrap().accepts(&Scope::Shared));
    }

    #[test]
    fn an_unknown_scope_is_an_error_not_a_default() {
        assert_eq!(
            Scope::from_str("production"),
            Err(UnknownScope("production".into()))
        );
        assert!(Wanted::parse("personal,typo").is_err());
    }
}
