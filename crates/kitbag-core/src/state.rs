//! How far this machine is from the store.
//!
//! The comparison is a hash, never a fetch: a store lists what it holds with
//! the payload hash beside each name, so a machine with nothing to send costs
//! one call and hands out no secrets to find that out.

use std::collections::HashMap;

use crate::collect::Item;
use crate::envelope::Envelope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The store has never seen it.
    New,
    /// The store holds something else under that name.
    Changed,
    /// Nothing to send.
    Unchanged,
    /// Only building the payload would tell, and building it is not free —
    /// an account bundle whose tokens rotate on their own, for instance.
    Unknown,
}

impl State {
    pub fn glyph(self) -> char {
        match self {
            State::New => '+',
            State::Changed => '~',
            State::Unchanged => '=',
            State::Unknown => '?',
        }
    }
}

/// What the store reports: item name to payload hash. A `None` hash means the
/// store holds the item but cannot say cheaply what is in it.
pub type Remote = HashMap<String, Option<String>>;

pub fn compare(item: &Item, remote: &Remote) -> State {
    let here = Envelope::new(item.scope.clone(), item.payload.clone()).sha256();
    match remote.get(&item.name) {
        None => State::New,
        Some(None) => State::Unknown,
        Some(Some(there)) if *there == here => State::Unchanged,
        Some(Some(_)) => State::Changed,
    }
}

/// Names the store holds that this machine no longer sends — a file that turned
/// machine-local, was deleted, or lost its marker. They are still readable
/// secrets, so a report says so rather than leaving them to rot.
pub fn orphans<'a>(items: &[Item], remote: &'a Remote) -> Vec<&'a String> {
    let mine: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
    let mut out: Vec<&String> = remote
        .keys()
        .filter(|name| !mine.contains(&name.as_str()))
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::Scope;
    use std::path::PathBuf;

    fn item(name: &str, body: &str) -> Item {
        Item {
            name: name.into(),
            scope: Scope::Personal,
            owner: None,
            path: PathBuf::from("/x"),
            payload: body.as_bytes().to_vec(),
            source: crate::collect::Source::File(PathBuf::from("/x")),
        }
    }

    fn hash(body: &str) -> String {
        Envelope::new(Scope::Personal, body.as_bytes().to_vec()).sha256()
    }

    #[test]
    fn a_name_the_store_has_never_seen_is_new() {
        assert_eq!(compare(&item("env:a", "x"), &Remote::new()), State::New);
    }

    #[test]
    fn the_same_bytes_are_unchanged() {
        let remote = Remote::from([("env:a".to_string(), Some(hash("x")))]);
        assert_eq!(compare(&item("env:a", "x"), &remote), State::Unchanged);
    }

    #[test]
    fn different_bytes_are_changed() {
        let remote = Remote::from([("env:a".to_string(), Some(hash("x")))]);
        assert_eq!(compare(&item("env:a", "y"), &remote), State::Changed);
    }

    #[test]
    fn a_store_that_cannot_say_leaves_it_unknown() {
        let remote = Remote::from([("env:a".to_string(), None)]);
        assert_eq!(compare(&item("env:a", "x"), &remote), State::Unknown);
    }

    #[test]
    fn what_the_store_keeps_and_this_machine_does_not_send_is_named() {
        let remote = Remote::from([
            ("env:a".to_string(), Some(hash("x"))),
            ("env:gone".to_string(), Some(hash("y"))),
        ]);
        assert_eq!(orphans(&[item("env:a", "x")], &remote), vec!["env:gone"]);
    }

    #[test]
    fn the_scope_is_part_of_what_is_compared_only_through_the_payload() {
        // Two items with the same bytes hash the same whatever their scope:
        // a scope change moves an item, it does not rewrite its contents.
        let mut work = item("env:a", "x");
        work.scope = Scope::Work;
        let remote = Remote::from([("env:a".to_string(), Some(hash("x")))]);
        assert_eq!(compare(&work, &remote), State::Unchanged);
    }
}
