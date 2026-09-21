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
/// The closure is handed that directory, where the stub records every call.
fn with_fake_bw<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
    // Serialised: the client and its state are process-wide.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = tempfile::tempdir().expect("a state directory");
    let stub = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-bw.sh");

    std::env::set_var("KITBAG_BW", stub);
    std::env::set_var("KITBAG_FAKE_STATE", dir.path());
    let out = f(dir.path());
    std::env::remove_var("KITBAG_BW");
    std::env::remove_var("KITBAG_FAKE_STATE");
    out
}

/// How many times the stub was asked for a given call.
fn calls(state: &std::path::Path, what: &str) -> usize {
    std::fs::read_to_string(state.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.trim() == what)
        .count()
}

fn big_envelope(len: usize) -> Envelope {
    Envelope::new(Scope::Personal, vec![b'k'; len]).with_path(Some("~/big".to_string()))
}

#[test]
fn a_payload_too_big_for_a_note_survives_the_round_trip() {
    with_fake_bw(|_state| {
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
    with_fake_bw(|_state| {
        let store = Bw::new().expect("unlocked");
        let sent = Envelope::new(Scope::Personal, b"small enough".to_vec());
        store.put("env:small", &sent).expect("put");
        assert_eq!(store.get("env:small").expect("get").payload, sent.payload);
    });
}

#[test]
fn the_hash_is_listed_without_the_payload_being_fetched() {
    with_fake_bw(|_state| {
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
    with_fake_bw(|_state| {
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

#[test]
fn the_whole_vault_is_listed_once_however_many_items_are_read() {
    // `bw list items` decrypts every item the vault holds. Reading thirty-nine
    // items used to ask for that thirty-nine times, which is what made a
    // restore slow enough to notice.
    with_fake_bw(|state| {
        let store = Bw::new().expect("unlocked");
        for n in 0..12 {
            let env = Envelope::new(Scope::Personal, format!("item {n}").into_bytes());
            store.put(&format!("env:{n}"), &env).expect("put");
        }
        for n in 0..12 {
            store.get(&format!("env:{n}")).expect("get");
        }

        assert_eq!(
            calls(state, "list items"),
            1,
            "24 operations, one decryption of the vault"
        );
        assert_eq!(
            calls(state, "list folders"),
            1,
            "and the folder is looked up once, not once per item"
        );
        assert_eq!(
            calls(state, "encode "),
            0,
            "encoding is base64, and does not need a process to do it"
        );
    });
}

#[test]
fn what_was_written_is_readable_without_asking_the_vault_again() {
    // The cache has to stay true across a write, or a push would hand back
    // whatever the vault held before it started.
    with_fake_bw(|state| {
        let store = Bw::new().expect("unlocked");
        let first = Envelope::new(Scope::Personal, b"before".to_vec());
        store.put("env:a", &first).expect("put");

        let second = Envelope::new(Scope::Personal, b"after".to_vec());
        store.put("env:a", &second).expect("put again");

        assert_eq!(store.get("env:a").expect("get").payload, second.payload);
        assert_eq!(calls(state, "list items"), 1);
    });
}

#[test]
fn a_payload_is_sent_even_when_the_old_copy_will_not_go() {
    // The new copy goes up first so that a failure after it is survivable.
    // Treating one as fatal reported an item as not sent when it was, left
    // the agreement unrecorded, and sent it again next run to fail in the
    // same place — two machines were stuck on one item like that.
    with_fake_bw(|state| {
        let store = Bw::new().expect("unlocked");
        store.put("app:big", &big_envelope(40_000)).expect("first");

        // From here the stub refuses every delete, as the real client did.
        std::fs::write(state.join("refuse-delete"), "1").expect("arm the refusal");

        let second = Envelope::new(Scope::Personal, vec![b'j'; 40_000]);
        store
            .put("app:big", &second)
            .expect("a cleanup that fails is not a write that failed");

        assert_eq!(
            store.get("app:big").expect("get").payload,
            second.payload,
            "and the newer payload is what comes back"
        );
    });
}
