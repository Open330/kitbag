//! The machines that may log in here.
//!
//! Adding a machine should not mean editing `authorized_keys` on every other
//! one by hand. That is how a fleet ends up with no two machines agreeing:
//! one trusting eight keys, another two, and three of those keys with no
//! comment and no known owner.
//!
//! The shortcut people take instead — handing the new machine a copy of an
//! existing private key — makes the whole fleet one identity. It cannot be
//! revoked for one machine, and it tells you nothing about which machine
//! logged in. So: one key per machine, and a list saying which are yours.
//!
//! **Merging never deletes.** A host may hold keys this list has never seen —
//! a CI runner, an agent, a phone — and replacing the file would lock them out
//! without a word. `revoke` is the only thing that removes, and only what it
//! is told to.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::run::home;

fn auth_path() -> PathBuf {
    home().join(".ssh/authorized_keys")
}

fn hosts_path() -> PathBuf {
    std::env::var_os("KITBAG_TRUSTED_HOSTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".ssh/trusted-hosts"))
}

fn key_path() -> PathBuf {
    home().join(".ssh/id_ed25519")
}

/// The type and the base64 body. The comment is a label that changes freely,
/// so it must take no part in deciding whether two lines are the same key.
pub fn material(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    let kind = parts.next()?;
    let body = parts.next()?;
    kind.starts_with("ssh-")
        .then(|| format!("{kind} {body}"))
        .or_else(|| kind.starts_with("ecdsa-").then(|| format!("{kind} {body}")))
}

pub fn comment(line: &str) -> String {
    let rest: Vec<&str> = line.split_whitespace().skip(2).collect();
    if rest.is_empty() {
        "(no comment)".into()
    } else {
        rest.join(" ")
    }
}

/// Add what is missing. Nothing is removed, ever.
pub fn merge(existing: &str, incoming: &str) -> (String, usize) {
    let known: Vec<String> = existing.lines().filter_map(material).collect();
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    let mut added = 0;
    for line in incoming.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(m) = material(line) else { continue };
        if known.contains(&m) || out.contains(&m) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        added += 1;
    }
    (out, added)
}

/// Remove the keys a person named, and only those.
pub fn revoke(
    existing: &str,
    targets: &[String],
    fingerprints: &[(String, String)],
) -> (String, usize) {
    let mut out = String::new();
    let mut removed = 0;
    for line in existing.lines() {
        let keep = if line.trim().is_empty() || line.starts_with('#') {
            true
        } else {
            let fp = fingerprints
                .iter()
                .find(|(l, _)| l == line)
                .map(|(_, f)| f.as_str())
                .unwrap_or("");
            !targets
                .iter()
                .any(|t| line.contains(t.as_str()) || fp.contains(t.as_str()))
        };
        if keep {
            out.push_str(line);
            out.push('\n');
        } else {
            removed += 1;
        }
    }
    (out, removed)
}

/// `ssh-keygen -lf -`. Shelling out rather than parsing key formats here: the
/// tool that defines the format is the one that should read it.
fn fingerprint(line: &str) -> Option<String> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new("ssh-keygen")
        .args(["-lf", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(line.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    out.status.success().then(|| {
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_string()
    })
}

fn read_auth() -> String {
    std::fs::read_to_string(auth_path()).unwrap_or_else(|_| {
        "# scope: personal\n# the machines that may log in as this user\n".into()
    })
}

fn write_auth(contents: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = auth_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    std::fs::write(&path, contents)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn hosts(args: &[String]) -> Result<Vec<String>> {
    if !args.is_empty() {
        return Ok(args.to_vec());
    }
    let path = hosts_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        bail!(
            "no hosts given, and {} does not exist — one ssh alias per line",
            path.display()
        );
    };
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect())
}

fn ssh(host: &str, command: &str, stdin: Option<&str>) -> Result<String> {
    use std::io::Write;
    use std::process::Stdio;

    let mut cmd = Command::new("ssh");
    cmd.args([
        "-o",
        "ConnectTimeout=10",
        "-o",
        "BatchMode=yes",
        host,
        command,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = cmd.spawn()?;
    if let Some(text) = stdin {
        child.stdin.as_mut().unwrap().write_all(text.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("{host}: unreachable");
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

// ---------------------------------------------------------------------------

pub fn list(args: &[String]) -> Result<()> {
    println!();
    println!("  this machine");
    show(&read_auth());

    for host in hosts(args).unwrap_or_default() {
        println!();
        println!("  {host}");
        match ssh(&host, "cat ~/.ssh/authorized_keys", None) {
            Ok(text) => show(&text),
            Err(e) => println!("  · {e}"),
        }
    }
    Ok(())
}

fn show(text: &str) {
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        println!(
            "  · {:<52} {}",
            fingerprint(line).unwrap_or_else(|| "(unreadable)".into()),
            comment(line)
        );
    }
}

/// This machine joins the list, with its own key.
pub fn register(new_key: bool) -> Result<()> {
    let key = key_path();
    if new_key || !key.exists() {
        if key.exists() {
            let retired = home().join(format!(
                ".ssh/retired-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs()
            ));
            std::fs::create_dir_all(&retired)?;
            std::fs::rename(&key, retired.join("id_ed25519"))?;
            let pub_key = key.with_extension("pub");
            if pub_key.exists() {
                std::fs::rename(&pub_key, retired.join("id_ed25519.pub"))?;
            }
            println!("  the previous key is at {}", retired.display());
        }
        let user = std::env::var("USER").unwrap_or_else(|_| "someone".into());
        let host = hostname();
        let status = Command::new("ssh-keygen")
            .args([
                "-t",
                "ed25519",
                "-N",
                "",
                "-C",
                &format!("{user}@{host}"),
                "-f",
                &key.to_string_lossy(),
            ])
            .stdout(std::process::Stdio::null())
            .status()?;
        if !status.success() {
            bail!("ssh-keygen would not make a key");
        }
        println!("  made a key for {user}@{host}");
    }

    let pub_key = std::fs::read_to_string(key.with_extension("pub"))
        .context("this machine has no public key to add")?;
    let (merged, added) = merge(&read_auth(), &pub_key);
    write_auth(&merged)?;

    if added == 0 {
        println!("  already in the list");
    } else {
        println!("  added {} to the list", comment(pub_key.trim()));
    }
    if new_key {
        println!("  no host trusts this key yet — run `kitbag trust sync` from here,");
        println!("  while the retired key is still around to get you in.");
    }
    Ok(())
}

/// Collect every host's key, then give every host the union.
pub fn sync(args: &[String]) -> Result<()> {
    let hosts = hosts(args)?;
    let mut list = read_auth();

    println!();
    println!("  collecting");
    for host in &hosts {
        match ssh(host, "cat ~/.ssh/id_ed25519.pub", None) {
            Ok(key) if !key.trim().is_empty() => {
                let (merged, added) = merge(&list, &key);
                list = merged;
                println!(
                    "  · {host:<16} {}",
                    if added == 0 {
                        "known".to_string()
                    } else {
                        format!("added {}", comment(key.trim()))
                    }
                );
            }
            _ => println!("  · {host:<16} unreachable, or it has no ed25519 key"),
        }
    }
    write_auth(&list)?;

    println!();
    println!("  distributing");
    for host in &hosts {
        // The merge runs on the far side, so a host keeps whatever it holds
        // that this list has never heard of.
        let script = r#"umask 077; mkdir -p ~/.ssh; chmod 700 ~/.ssh; touch ~/.ssh/authorized_keys; n=0
while IFS= read -r line; do
  case "$line" in ""|\#*) continue ;; esac
  m=$(printf '%s\n' "$line" | awk '{print $1, $2}')
  grep -qF "$m" ~/.ssh/authorized_keys || { printf '%s\n' "$line" >> ~/.ssh/authorized_keys; n=$((n+1)); }
done
chmod 600 ~/.ssh/authorized_keys; echo "$n""#;
        match ssh(host, script, Some(&list)) {
            Ok(n) => println!("  · {host:<16} {} new", n.trim()),
            Err(e) => println!("  · {e}"),
        }
    }
    Ok(())
}

/// Drop a key here and everywhere named.
pub fn revoke_cmd(targets: &[String], hosts_args: &[String]) -> Result<()> {
    if targets.is_empty() {
        bail!("nothing to revoke — name a fingerprint or a comment");
    }

    let existing = read_auth();
    let fingerprints: Vec<(String, String)> = existing
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| fingerprint(l).map(|f| (l.to_string(), f)))
        .collect();
    let (kept, removed) = revoke(&existing, targets, &fingerprints);
    if removed > 0 {
        write_auth(&kept)?;
    }
    println!();
    println!("  this machine: {removed} removed");

    for host in hosts(hosts_args).unwrap_or_default() {
        let filter = targets
            .iter()
            .map(|t| format!("grep -v -- {}", shell_quote(t)))
            .collect::<Vec<_>>()
            .join(" | ");
        let script = format!(
            r#"f=~/.ssh/authorized_keys; [ -f "$f" ] || exit 0
cp "$f" "$f.backup.$(date +%s)"
before=$(grep -c . "$f")
tmp=$(mktemp); {filter} < "$f" > "$tmp"; cat "$tmp" > "$f"; rm -f "$tmp"; chmod 600 "$f"
echo $((before - $(grep -c . "$f")))"#
        );
        match ssh(&host, &script, None) {
            Ok(n) => println!("  {host}: {} removed", n.trim()),
            Err(e) => println!("  {e}"),
        }
    }
    println!();
    println!("  a key removed here is still on any host that was unreachable.");
    Ok(())
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn hostname() -> String {
    Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "machine".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIaaaa laptop";
    const B: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIbbbb desktop";

    #[test]
    fn a_key_is_the_same_key_whatever_it_is_labelled() {
        let renamed = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIaaaa a new name for it";
        assert_eq!(material(A), material(renamed));
        assert_ne!(material(A), material(B));
    }

    #[test]
    fn merging_adds_what_is_missing() {
        let (merged, added) = merge(&format!("{A}\n"), &format!("{A}\n{B}\n"));
        assert_eq!(added, 1);
        assert!(merged.contains("laptop") && merged.contains("desktop"));
    }

    #[test]
    fn merging_never_removes_what_it_does_not_know() {
        let theirs = "ssh-rsa AAAAB3Nza ci-runner\n";
        let (merged, _) = merge(theirs, &format!("{A}\n"));
        assert!(
            merged.contains("ci-runner"),
            "a key this list never saw must survive: {merged}"
        );
    }

    #[test]
    fn merging_twice_changes_nothing_the_second_time() {
        let (once, _) = merge("", &format!("{A}\n{B}\n"));
        let (twice, added) = merge(&once, &format!("{A}\n{B}\n"));
        assert_eq!(added, 0);
        assert_eq!(once, twice);
    }

    #[test]
    fn a_comment_is_read_back_or_said_to_be_missing() {
        assert_eq!(comment(A), "laptop");
        assert_eq!(comment("ssh-ed25519 AAAAC3Nza"), "(no comment)");
    }

    #[test]
    fn revoking_removes_only_what_was_named() {
        let list = format!("# a header\n{A}\n{B}\n");
        let (kept, removed) = revoke(&list, &["desktop".into()], &[]);

        assert_eq!(removed, 1);
        assert!(kept.contains("laptop"));
        assert!(!kept.contains("desktop"));
        assert!(kept.contains("# a header"), "comments are not keys");
    }

    #[test]
    fn revoking_by_fingerprint_finds_a_key_with_no_comment_at_all() {
        let bare = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIcccc";
        let list = format!("{A}\n{bare}\n");
        let fingerprints = vec![(bare.to_string(), "SHA256:abcdef".to_string())];

        let (kept, removed) = revoke(&list, &["SHA256:abcdef".into()], &fingerprints);

        assert_eq!(removed, 1);
        assert!(kept.contains("laptop"));
        assert!(!kept.contains("cccc"));
    }

    #[test]
    fn a_target_with_a_quote_in_it_cannot_escape_the_shell() {
        let quoted = shell_quote("it's; rm -rf /");
        assert_eq!(quoted, r"'it'\''s; rm -rf /'");
    }
}
