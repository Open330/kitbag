//! How far this machine is from the store.
//!
//! The comparison is a hash, never a fetch: a store lists what it holds with
//! the payload hash beside each name, so a machine with nothing to send costs
//! one call and hands out no secrets to find that out.

use std::collections::HashMap;

use crate::collect::Item;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The store has never seen it.
    New,
    /// The store holds something else, and there is no record of what the two
    /// last agreed on — so which way it should go cannot be worked out here.
    Changed,
    /// This machine has moved since the last exchange and the store has not.
    /// Sending it loses nothing.
    Ahead,
    /// The store has moved and this machine has not. Taking it loses nothing.
    Behind,
    /// Both have moved since they last agreed. Whichever way this goes, one
    /// side's work is written over, so it is not a tool's to choose.
    Conflict,
    /// Nothing to send.
    Unchanged,
    /// No answer is available. Either building the payload is not free, or it
    /// was built and says nothing: an export whose bytes differ every run
    /// differs from the store every run, and "changed" would be a claim about
    /// the machine that nobody checked.
    Unknown,
}

impl State {
    pub fn glyph(self) -> char {
        match self {
            State::New => '+',
            State::Changed => '~',
            State::Ahead => '>',
            State::Behind => '<',
            State::Conflict => '!',
            State::Unchanged => '=',
            State::Unknown => '?',
        }
    }

    /// Can this be acted on without asking anybody?
    pub fn is_decided(self) -> bool {
        !matches!(self, State::Conflict | State::Changed)
    }
}

/// What the store reports: item name to the fingerprint of what it holds. A
/// `None` means the store has the item but cannot say cheaply what is in it.
pub type Remote = HashMap<String, Option<String>>;

pub fn compare(item: &Item, remote: &Remote, home: &std::path::Path) -> State {
    compare_against(item, remote, home, &crate::ledger::Ledger::default())
}

/// The same, with the record of what the two last agreed on.
///
/// That record is the third point a difference needs to have a direction —
/// the same three-way git does. Without it the only honest answer to "these
/// differ" is that they differ.
pub fn compare_against(
    item: &Item,
    remote: &Remote,
    home: &std::path::Path,
    ledger: &crate::ledger::Ledger,
) -> State {
    // The whole envelope, not the payload: a scope marker that changed, or a
    // platform tag that was added, leaves the bytes alone and still has to
    // reach the store.
    let here = crate::collect::envelope_for(item, home).fingerprint();
    match remote.get(&item.name) {
        None => State::New,
        Some(None) => State::Unknown,
        // The hashes will differ, and the difference carries no information:
        // saying "changed" would make the report unreadable by making three
        // items shout on every run.
        Some(_) if item.volatile => State::Unknown,
        Some(Some(there)) if *there == here => State::Unchanged,
        Some(Some(there)) => match ledger.base(&item.name) {
            None => State::Changed,
            Some(base) if base == here => State::Behind,
            Some(base) if base == there => State::Ahead,
            Some(_) => State::Conflict,
        },
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

    fn compare_here(item: &Item, remote: &Remote) -> State {
        compare(item, remote, &home())
    }
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
            volatile: false,
            platform: Vec::new(),
            machine: None,
        }
    }

    fn home() -> PathBuf {
        PathBuf::from("/")
    }

    /// What the store would report for an item this machine holds.
    fn hash(body: &str) -> String {
        crate::collect::envelope_for(&item("ignored", body), &home()).fingerprint()
    }

    #[test]
    fn a_name_the_store_has_never_seen_is_new() {
        assert_eq!(
            compare_here(&item("env:a", "x"), &Remote::new()),
            State::New
        );
    }

    #[test]
    fn the_same_bytes_are_unchanged() {
        let remote = Remote::from([("env:a".to_string(), Some(hash("x")))]);
        assert_eq!(compare_here(&item("env:a", "x"), &remote), State::Unchanged);
    }

    #[test]
    fn different_bytes_are_changed() {
        let remote = Remote::from([("env:a".to_string(), Some(hash("x")))]);
        assert_eq!(compare_here(&item("env:a", "y"), &remote), State::Changed);
    }

    #[test]
    fn a_store_that_cannot_say_leaves_it_unknown() {
        let remote = Remote::from([("env:a".to_string(), None)]);
        assert_eq!(compare_here(&item("env:a", "x"), &remote), State::Unknown);
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
    fn a_volatile_item_is_never_called_changed() {
        // Its export differs every run by construction, so the difference is
        // not news. Three of these turned every status report into noise.
        let mut volatile = item("app:tokens", "built at 09:00");
        volatile.volatile = true;
        let remote = Remote::from([("app:tokens".to_string(), Some(hash("built at 08:59")))]);
        assert_eq!(compare_here(&volatile, &remote), State::Unknown);
    }

    #[test]
    fn a_volatile_item_the_store_has_never_seen_is_still_new() {
        // "Cannot tell" is about comparing. With nothing to compare against
        // there is no doubt: it has never been sent.
        let mut volatile = item("app:tokens", "x");
        volatile.volatile = true;
        assert_eq!(compare_here(&volatile, &Remote::new()), State::New);
    }

    #[test]
    fn a_stable_item_is_still_compared_normally() {
        let remote = Remote::from([("env:a".to_string(), Some(hash("x")))]);
        assert_eq!(compare_here(&item("env:a", "x"), &remote), State::Unchanged);
        assert_eq!(compare_here(&item("env:a", "y"), &remote), State::Changed);
    }

    #[test]
    fn a_marker_that_changed_is_a_change() {
        // This test used to assert the opposite, on the reasoning that a scope
        // change moves an item rather than rewriting its contents. The store
        // holds the scope too, and a push that will not notice leaves it
        // holding the old one: another machine then restores the item into the
        // wrong life and nothing ever says so.
        let mut work = item("env:a", "x");
        work.scope = Scope::Work;
        let remote = Remote::from([("env:a".to_string(), Some(hash("x")))]);
        assert_eq!(compare_here(&work, &remote), State::Changed);
    }

    #[test]
    fn a_platform_tag_that_was_added_is_a_change() {
        // Found the hard way: four items were tagged `macos` and a push said
        // "38 already there", because it was comparing payloads and the
        // payloads had not moved.
        let mut tagged = item("app:x", "x");
        tagged.platform = vec!["macos".to_string()];
        let remote = Remote::from([("app:x".to_string(), Some(hash("x")))]);
        assert_eq!(compare_here(&tagged, &remote), State::Changed);
    }

    #[test]
    fn an_owner_that_changed_is_a_change() {
        let mut owned = item("env:a", "x");
        owned.owner = Some("acme".to_string());
        let remote = Remote::from([("env:a".to_string(), Some(hash("x")))]);
        assert_eq!(compare_here(&owned, &remote), State::Changed);
    }

    #[test]
    fn nothing_moving_is_still_nothing_to_send() {
        // The point of all of the above is not to make everything look
        // changed.
        let remote = Remote::from([("env:a".to_string(), Some(hash("x")))]);
        assert_eq!(compare_here(&item("env:a", "x"), &remote), State::Unchanged);
    }
}
