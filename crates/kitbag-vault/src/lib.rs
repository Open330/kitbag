//! Where the values live.
//!
//! kitbag does not implement a secret store. It borrows one, so that a person
//! keeps using what they already trust - and so that losing interest in kitbag
//! does not strand their secrets inside it.
//!
//! Backends, in the order they are worth writing:
//!
//! | backend | why |
//! | --- | --- |
//! | `bw`   | Bitwarden / Vaultwarden: free tier, self-hostable |
//! | `age`  | no server at all - an encrypted file beside a public repo |
//! | `op`   | 1Password: the best developer CLI in the category |
//! | `pass` | GPG and a git repo, for people who already have it |
//!
//! `age` matters more than its position suggests: it is what makes the tool
//! usable by someone with no vault at all.

use anyhow::Result;
use kitbag_core::Scope;

/// What the store knows about an item without being asked for its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteItem {
    pub name: String,
    pub scope: Option<Scope>,
    pub owner: Option<String>,
    /// Hash of the payload, so an unchanged item is never rewritten.
    pub payload_hash: Option<String>,
}

pub trait Backend {
    /// Every item this store holds for us - names, scopes and hashes, never
    /// values. One call, so comparing a machine against the store is cheap.
    fn list(&self) -> Result<Vec<RemoteItem>>;

    /// The value of one item. Callers keep it as briefly as they can.
    fn get(&self, name: &str) -> Result<Vec<u8>>;

    /// Create or update. Never deletes: a store may hold things this machine
    /// knows nothing about, and guessing otherwise loses somebody's key.
    fn put(&self, name: &str, payload: &[u8], meta: &RemoteItem) -> Result<()>;
}
