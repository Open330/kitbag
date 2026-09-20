//! The three providers that turn most of the remaining shell into data.
//!
//! Classifying a working installer said the same thing three times: what is
//! left after packages, links and system settings is *fetch this pinned
//! thing*, *clone that repository*, and *merge into a file an application also
//! writes to*. Each is the same few lines every time somebody writes it by
//! hand, and each has a way of going wrong that a provider can rule out.

use std::path::Path;
use std::process::Command;

use kitbag_core::recipe::{Clone_, Download, Merge};
use sha2::{Digest, Sha256};

use crate::apply::Done;
use crate::Action;

// ---------------------------------------------------------------------------
// download — pinned to a hash, or it is not a description of a machine
// ---------------------------------------------------------------------------

pub fn plan_download(d: &Download, home: &Path) -> Action {
    let target = d.target(home);
    if d.extract {
        // An unpacked archive cannot be hashed back, so the marker beside it
        // records which archive it came from.
        let marker = extract_marker(&target, &d.sha256);
        return if marker.exists() {
            Action::None
        } else if target.exists() {
            Action::Replace(format!("unpack {} into {}", short_url(&d.url), d.to))
        } else {
            Action::Create(format!("unpack {} into {}", short_url(&d.url), d.to))
        };
    }

    match std::fs::read(&target) {
        Ok(bytes) if sha256(&bytes) == d.sha256 => Action::None,
        Ok(_) => Action::Replace(format!("{} is there, but not this version", d.to)),
        Err(_) => Action::Create(format!("fetch {}", short_url(&d.url))),
    }
}

pub fn apply_download(d: &Download, home: &Path) -> Done {
    match plan_download(d, home) {
        Action::None => return Done::Skipped,
        Action::Unknown(why) => return Done::Refused(why),
        _ => {}
    }

    let tmp = match tempfile::NamedTempFile::new() {
        Ok(t) => t,
        Err(e) => return Done::Failed(e.to_string()),
    };

    // curl rather than an HTTP stack of our own: it is on every machine this
    // runs on, it already knows about proxies and certificates, and carrying
    // TLS for one download means carrying every CVE in it. https only - a
    // recipe that fetches over plain http is fetching whatever the network
    // decides to hand back.
    //
    // A file:// url is read directly instead, which keeps the https-only rule
    // absolute and lets the tests exercise everything but the network.
    if let Some(path) = d.url.strip_prefix("file://") {
        if let Err(e) = std::fs::copy(path, tmp.path()) {
            return Done::Failed(format!("{path}: {e}"));
        }
    } else {
        let fetched = Command::new("curl")
            .args([
                "-LsSf",
                "--proto",
                "=https",
                "--tlsv1.2",
                &d.url,
                "-o",
                &tmp.path().to_string_lossy(),
            ])
            .output();
        match fetched {
            Ok(o) if o.status.success() => {}
            Ok(o) => return Done::Failed(last_line(&String::from_utf8_lossy(&o.stderr))),
            Err(e) => return Done::Failed(e.to_string()),
        }
    }

    // Checked before anything is written where it will be used. A download
    // nobody verified is a download somebody else chose.
    let bytes = match std::fs::read(tmp.path()) {
        Ok(b) => b,
        Err(e) => return Done::Failed(e.to_string()),
    };
    let got = sha256(&bytes);
    if got != d.sha256 {
        return Done::Failed(format!(
            "checksum mismatch: wanted {}…, got {}…",
            &d.sha256[..d.sha256.len().min(12)],
            &got[..12]
        ));
    }

    let target = d.target(home);
    if d.extract {
        if let Err(e) = std::fs::create_dir_all(&target) {
            return Done::Failed(e.to_string());
        }
        let out = Command::new("tar")
            .args([
                "-xzf",
                &tmp.path().to_string_lossy(),
                "-C",
                &target.to_string_lossy(),
            ])
            .output();
        match out {
            Ok(o) if o.status.success() => {}
            Ok(o) => return Done::Failed(last_line(&String::from_utf8_lossy(&o.stderr))),
            Err(e) => return Done::Failed(e.to_string()),
        }
        if let Err(e) = std::fs::write(extract_marker(&target, &d.sha256), &d.sha256) {
            return Done::Failed(e.to_string());
        }
        Done::Created(format!("unpacked into {}", d.to))
    } else {
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Done::Failed(e.to_string());
            }
        }
        match std::fs::write(&target, &bytes) {
            Ok(()) => Done::Created(format!("fetched {}", d.to)),
            Err(e) => Done::Failed(e.to_string()),
        }
    }
}

fn extract_marker(dir: &Path, sha: &str) -> std::path::PathBuf {
    dir.join(format!(".kitbag-{}", &sha[..sha.len().min(16)]))
}

fn sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn short_url(url: &str) -> String {
    url.rsplit('/').next().unwrap_or(url).to_string()
}

// ---------------------------------------------------------------------------
// clone — somebody else's repository
// ---------------------------------------------------------------------------

pub fn plan_clone(c: &Clone_, home: &Path) -> Action {
    let target = c.target(home);
    if target.join(".git").is_dir() {
        Action::None
    } else if target.exists() {
        // Cloning into a directory that is already something else would either
        // fail or merge two unrelated trees. Neither is a thing to do quietly.
        Action::Unknown(format!("{} exists and is not a clone", c.to))
    } else {
        Action::Create(format!("clone {}", short_url(&c.repo)))
    }
}

pub fn apply_clone(c: &Clone_, home: &Path) -> Done {
    match plan_clone(c, home) {
        Action::None => return Done::Skipped,
        Action::Unknown(why) => return Done::Refused(why),
        _ => {}
    }
    let target = c.target(home);
    if let Some(parent) = target.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Done::Failed(e.to_string());
        }
    }
    let out = Command::new("git")
        .args(["clone", "--depth", "1", &c.repo, &target.to_string_lossy()])
        .output();
    match out {
        Ok(o) if o.status.success() => Done::Created(format!("cloned into {}", c.to)),
        Ok(o) => Done::Failed(last_line(&String::from_utf8_lossy(&o.stderr))),
        Err(e) => Done::Failed(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// merge — a file the application writes to as well
// ---------------------------------------------------------------------------

/// Deep-merge `ours` into `theirs`: our keys win, theirs survive.
///
/// The direction matters. An application writes its own state into these files
/// (a window position, a last-opened list, a device id), and replacing the file
/// throws that away every time a recipe is applied.
pub fn merge_json(theirs: &serde_json::Value, ours: &serde_json::Value) -> serde_json::Value {
    match (theirs, ours) {
        (serde_json::Value::Object(t), serde_json::Value::Object(o)) => {
            let mut out = t.clone();
            for (k, v) in o {
                let merged = match t.get(k) {
                    Some(existing) => merge_json(existing, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            serde_json::Value::Object(out)
        }
        // A list is a value, not a container to merge into: appending would
        // duplicate on every run.
        (_, ours) => ours.clone(),
    }
}

fn merged_text(m: &Merge, home: &Path, repo: &Path) -> Result<String, String> {
    let (from, to) = m.expanded(home, repo);
    let ours = std::fs::read_to_string(&from).map_err(|e| format!("{}: {e}", from.display()))?;
    let theirs = std::fs::read_to_string(&to).unwrap_or_default();

    match m.format.as_str() {
        "json" => {
            let ours: serde_json::Value =
                serde_json::from_str(&ours).map_err(|e| format!("{}: {e}", from.display()))?;
            let theirs: serde_json::Value = if theirs.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&theirs).map_err(|e| format!("{}: {e}", to.display()))?
            };
            serde_json::to_string_pretty(&merge_json(&theirs, &ours)).map_err(|e| e.to_string())
        }
        "toml" => {
            let ours: serde_json::Value = toml::from_str(&ours)
                .map_err(|e| format!("{}: {e}", from.display()))
                .and_then(|v: toml::Value| serde_json::to_value(v).map_err(|e| e.to_string()))?;
            let theirs: serde_json::Value = if theirs.trim().is_empty() {
                serde_json::json!({})
            } else {
                toml::from_str(&theirs)
                    .map_err(|e| format!("{}: {e}", to.display()))
                    .and_then(|v: toml::Value| serde_json::to_value(v).map_err(|e| e.to_string()))?
            };
            let merged = merge_json(&theirs, &ours);
            toml::to_string_pretty(&merged).map_err(|e| e.to_string())
        }
        other => Err(format!("no idea how to merge {other}")),
    }
}

pub fn plan_merge(m: &Merge, home: &Path, repo: &Path) -> Action {
    let (_, to) = m.expanded(home, repo);
    match merged_text(m, home, repo) {
        Err(why) => Action::Unknown(why),
        Ok(wanted) => {
            let now = std::fs::read_to_string(&to).unwrap_or_default();
            if same_document(&now, &wanted, &m.format) {
                Action::None
            } else if now.is_empty() {
                Action::Create(format!("write {}", m.to))
            } else {
                Action::Replace(format!("merge into {}", m.to))
            }
        }
    }
}

/// Two files that parse to the same document are the same file, whatever the
/// formatting - otherwise every run would rewrite a file it had just written.
fn same_document(a: &str, b: &str, format: &str) -> bool {
    match format {
        "json" => match (
            serde_json::from_str::<serde_json::Value>(a),
            serde_json::from_str::<serde_json::Value>(b),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => a.trim() == b.trim(),
        },
        "toml" => match (
            toml::from_str::<toml::Value>(a),
            toml::from_str::<toml::Value>(b),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => a.trim() == b.trim(),
        },
        _ => a.trim() == b.trim(),
    }
}

pub fn apply_merge(m: &Merge, home: &Path, repo: &Path) -> Done {
    let action = plan_merge(m, home, repo);
    match &action {
        Action::None => return Done::Skipped,
        Action::Unknown(why) => return Done::Refused(why.clone()),
        _ => {}
    }
    let wanted = match merged_text(m, home, repo) {
        Ok(text) => text,
        Err(why) => return Done::Refused(why),
    };
    let (_, to) = m.expanded(home, repo);
    if let Some(parent) = to.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Done::Failed(e.to_string());
        }
    }
    match std::fs::write(&to, wanted) {
        Ok(()) => match action {
            Action::Create(what) => Done::Created(what),
            _ => Done::Replaced {
                what: format!("merged into {}", m.to),
                backup: None,
            },
        },
        Err(e) => Done::Failed(e.to_string()),
    }
}

fn last_line(s: &str) -> String {
    s.lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or("no output")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn download(to: &str, sha: &str) -> Download {
        Download {
            url: "file:///dev/null".into(),
            sha256: sha.into(),
            to: to.into(),
            extract: false,
        }
    }

    #[test]
    fn a_file_that_already_hashes_right_is_nothing_to_do() {
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join("thing"), b"contents").unwrap();
        let d = download("~/thing", &sha256(b"contents"));

        assert_eq!(plan_download(&d, home.path()), Action::None);
    }

    #[test]
    fn a_file_of_the_wrong_version_is_a_replacement_not_a_create() {
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join("thing"), b"an older one").unwrap();
        let d = download("~/thing", &sha256(b"contents"));

        assert!(matches!(plan_download(&d, home.path()), Action::Replace(_)));
    }

    #[test]
    fn a_download_that_does_not_match_its_hash_is_never_written() {
        let home = tempfile::tempdir().unwrap();
        let source = home.path().join("source");
        fs::write(&source, b"what the server actually served").unwrap();

        let d = Download {
            url: format!("file://{}", source.display()),
            sha256: sha256(b"what the recipe expected"),
            to: "~/thing".into(),
            extract: false,
        };
        let done = apply_download(&d, home.path());

        match &done {
            Done::Failed(why) => assert!(why.contains("checksum"), "{why}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(
            !home.path().join("thing").exists(),
            "nothing may be written when the hash is wrong"
        );
    }

    #[test]
    fn a_download_that_matches_lands_where_it_was_asked_to() {
        let home = tempfile::tempdir().unwrap();
        let source = home.path().join("source");
        fs::write(&source, b"the right bytes").unwrap();

        let d = Download {
            url: format!("file://{}", source.display()),
            sha256: sha256(b"the right bytes"),
            to: "~/deep/thing".into(),
            extract: false,
        };

        assert!(apply_download(&d, home.path()).changed());
        assert_eq!(
            fs::read(home.path().join("deep/thing")).unwrap(),
            b"the right bytes"
        );
    }

    #[test]
    fn a_clone_that_is_there_is_left_alone_and_a_stranger_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let c = Clone_ {
            repo: "https://example.test/x.git".into(),
            to: "~/plugins".into(),
        };
        assert!(matches!(plan_clone(&c, home.path()), Action::Create(_)));

        fs::create_dir_all(home.path().join("plugins/.git")).unwrap();
        assert_eq!(plan_clone(&c, home.path()), Action::None);

        let d = Clone_ {
            repo: "https://example.test/y.git".into(),
            to: "~/not-a-clone".into(),
        };
        fs::create_dir(home.path().join("not-a-clone")).unwrap();
        assert!(matches!(plan_clone(&d, home.path()), Action::Unknown(_)));
    }

    #[test]
    fn merging_keeps_what_the_application_put_there() {
        let theirs = serde_json::json!({
            "windowPosition": [100, 200],
            "editor": { "theme": "dark", "fontSize": 12 }
        });
        let ours = serde_json::json!({
            "editor": { "fontSize": 14 },
            "hooks": ["ours"]
        });

        let merged = merge_json(&theirs, &ours);

        assert_eq!(merged["windowPosition"], serde_json::json!([100, 200]));
        assert_eq!(merged["editor"]["theme"], "dark", "theirs survives");
        assert_eq!(merged["editor"]["fontSize"], 14, "ours wins");
        assert_eq!(merged["hooks"], serde_json::json!(["ours"]));
    }

    #[test]
    fn a_list_is_replaced_rather_than_appended_to() {
        let theirs = serde_json::json!({ "plugins": ["a", "b"] });
        let ours = serde_json::json!({ "plugins": ["a"] });
        // Appending would grow the list on every apply.
        assert_eq!(
            merge_json(&theirs, &ours)["plugins"],
            serde_json::json!(["a"])
        );
    }

    #[test]
    fn a_json_settings_file_is_merged_and_then_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::write(
            repo.path().join("ours.json"),
            r#"{"editor": {"fontSize": 14}}"#,
        )
        .unwrap();
        fs::create_dir_all(home.path().join(".app")).unwrap();
        fs::write(
            home.path().join(".app/settings.json"),
            r#"{"deviceId": "abc", "editor": {"theme": "dark"}}"#,
        )
        .unwrap();

        let m = Merge {
            from: "ours.json".into(),
            to: "~/.app/settings.json".into(),
            format: "json".into(),
        };

        assert!(matches!(
            plan_merge(&m, home.path(), repo.path()),
            Action::Replace(_)
        ));
        assert!(apply_merge(&m, home.path(), repo.path()).changed());

        let after: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(home.path().join(".app/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            after["deviceId"], "abc",
            "the application's own state survived"
        );
        assert_eq!(after["editor"]["fontSize"], 14);
        assert_eq!(after["editor"]["theme"], "dark");

        // and a second run has nothing to do
        assert_eq!(plan_merge(&m, home.path(), repo.path()), Action::None);
    }

    #[test]
    fn a_toml_file_merges_the_same_way() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::write(repo.path().join("ours.toml"), "model = \"fast\"\n").unwrap();
        fs::write(home.path().join("config.toml"), "trusted = true\n").unwrap();

        let m = Merge {
            from: "ours.toml".into(),
            to: "~/config.toml".into(),
            format: "toml".into(),
        };
        assert!(apply_merge(&m, home.path(), repo.path()).changed());

        let after = fs::read_to_string(home.path().join("config.toml")).unwrap();
        assert!(after.contains("trusted = true"), "{after}");
        assert!(after.contains("model = \"fast\""), "{after}");
        assert_eq!(plan_merge(&m, home.path(), repo.path()), Action::None);
    }
}
