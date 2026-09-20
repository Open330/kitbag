//! The path a payload takes when it is larger than a note may be.
//!
//! This is the case that stopped a real push dead: the backend declared
//! `attachments: true` and a note limit, and then put everything in the note
//! regardless. These tests drive the backend against a `bw` that records what
//! it was asked, so the shapes of those calls are checked by something other
//! than someone's live vault.

use kitbag_core::{Envelope, Scope};
use kitbag_vault::bw::Bw;
use kitbag_vault::Backend;

/// Point the backend at the stub, in a state directory of this test's own.
fn with_fake_bw<T>(f: impl FnOnce() -> T) -> T {
    // Serialised: the client and its state are process-wide.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = tempfile::tempdir().expect("a state directory");
    let stub = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-bw.sh");

    std::env::set_var("KITBAG_BW", stub);
    std::env::set_var("KITBAG_FAKE_STATE", dir.path());
    let out = f();
    std::env::remove_var("KITBAG_BW");
    std::env::remove_var("KITBAG_FAKE_STATE");
    out
}

fn big_envelope(len: usize) -> Envelope {
    Envelope::new(Scope::Personal, vec![b'k'; len]).with_path(Some("~/big".to_string()))
}

#[test]
fn a_payload_too_big_for_a_note_survives_the_round_trip() {
    with_fake_bw(|| {
        let store = Bw::new().expect("the stub reports an unlocked vault");
        let sent = big_envelope(40_000);

        store.put("app:big", &sent).expect("put");
        let back = store.get("app:big").expect("get");

        assert_eq!(back.payload, sent.payload, "byte for byte");
        assert_eq!(back.sha256(), sent.sha256());
        assert_eq!(back.path.as_deref(), Some("~/big"), "and where it goes");
    });
}

#[test]
fn a_small_payload_still_goes_in_the_note() {
    with_fake_bw(|| {
        let store = Bw::new().expect("unlocked");
        let sent = Envelope::new(Scope::Personal, b"small enough".to_vec());
        store.put("env:small", &sent).expect("put");
        assert_eq!(store.get("env:small").expect("get").payload, sent.payload);
    });
}

#[test]
fn the_hash_is_listed_without_the_payload_being_fetched() {
    with_fake_bw(|| {
        let store = Bw::new().expect("unlocked");
        let sent = big_envelope(40_000);
        store.put("app:big", &sent).expect("put");

        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "app:big");
        assert_eq!(
            listed[0].payload_hash.as_deref(),
            Some(sent.sha256().as_str()),
            "an unchanged attachment must compare without being downloaded"
        );
    });
}

#[test]
fn replacing_a_payload_leaves_one_attachment_not_two() {
    with_fake_bw(|| {
        let store = Bw::new().expect("unlocked");
        store.put("app:big", &big_envelope(40_000)).expect("first");

        let second = Envelope::new(Scope::Personal, vec![b'j'; 40_000]);
        store.put("app:big", &second).expect("second");

        let back = store.get("app:big").expect("get");
        assert_eq!(
            back.payload, second.payload,
            "the newer one is what returns"
        );
        assert_eq!(
            store.list().expect("list")[0].payload_hash.as_deref(),
            Some(second.sha256().as_str())
        );
    });
}
