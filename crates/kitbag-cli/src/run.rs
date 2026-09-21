//! The two commands that have an engine behind them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use kitbag_catalog::{scan, Finding, Source};
use kitbag_core::collect::{collect, Collected};
use kitbag_core::recipe::Recipes;
use kitbag_core::state::{compare, orphans, Remote, State};
use kitbag_core::{Config, Scope, Wanted};
use kitbag_providers::{apply_recipe, plan_recipe, Action, Done, PackageManager, Step};
use kitbag_vault::{Backend, BackendKind};

use crate::ui::{self, Colour, Group, Mark, Row};

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub fn config_path() -> PathBuf {
    std::env::var_os("KITBAG_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config/kitbag/machine.toml"))
}

fn mark_of(state: State) -> Mark {
    match state {
        State::New => Mark::New,
        State::Changed => Mark::Changed,
        State::Unchanged => Mark::Unchanged,
        State::Unknown => Mark::Unknown,
    }
}

/// What this machine holds, grouped by whose it is and marked against a store.
pub fn status(
    backend: Option<&str>,
    wanted: Option<Wanted>,
    colour: Colour,
    width: usize,
    json: bool,
) -> Result<()> {
    let home = home();
    let cfg_path = config_path();
    let config = Config::load_or_default(&cfg_path)?.with_env_skips();
    let wanted = wanted.unwrap_or_else(|| config.wanted());

    let Collected { items, skipped } = collect(&config, &home);

    // No store named means the local side only: a machine can be looked at
    // before it has anywhere to send things.
    let remote: Remote = match backend {
        None => Remote::new(),
        Some(name) => {
            let kind: BackendKind = name.parse()?;
            let store = kind
                .open()
                .with_context(|| format!("opening the {name} store"))?;
            store
                .list()?
                .into_iter()
                .map(|l| (l.name, l.fingerprint))
                .collect()
        }
    };
    let compared = backend.is_some();

    let mut groups: BTreeMap<(String, Option<String>), Vec<Row>> = BTreeMap::new();
    let mut held_back = 0usize;

    for item in &items {
        if !wanted.accepts(&item.scope) {
            held_back += 1;
            continue;
        }
        let mark = compared.then(|| mark_of(compare(item, &remote, &home)));
        groups
            .entry((item.scope.name().to_string(), item.owner.clone()))
            .or_default()
            .push(Row {
                mark,
                name: item.name.clone(),
                detail: item.detail(),
            });
    }

    if json {
        let out = serde_json::json!({
            "items": items.iter().map(|i| serde_json::json!({
                "name": i.name,
                "scope": i.scope.name(),
                "owner": i.owner,
                "path": pretty(&i.path, &home),
                "detail": i.detail(),
                "taken": wanted.accepts(&i.scope),
                "state": compared.then(|| match compare(i, &remote, &home) {
                    State::New => "new",
                    State::Changed => "changed",
                    State::Unchanged => "unchanged",
                    State::Unknown => "unknown",
                }),
            })).collect::<Vec<_>>(),
            "skipped": skipped.iter().map(|s| serde_json::json!({
                "path": pretty(&s.path, &home),
                "why": s.reason.says(),
            })).collect::<Vec<_>>(),
            "orphans": compared.then(|| orphans(&items, &remote)),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if items.is_empty() && skipped.is_empty() {
        println!();
        println!(
            "  Nothing is tracked yet. Add a [[track]] entry to {}",
            cfg_path.display()
        );
        println!("  or run `kitbag discover` to see what is here.");
        return Ok(());
    }

    let rendered: Vec<Group> = groups
        .into_iter()
        .map(|((scope, owner), rows)| Group {
            scope: scope.parse().unwrap_or(Scope::Personal),
            owner,
            rows,
        })
        .collect();
    print!("{}", ui::render(&rendered, colour, width));

    if !skipped.is_empty() {
        println!();
        println!("  not tracked");
        for s in &skipped {
            println!("  · {}  — {}", pretty(&s.path, &home), s.reason.says());
        }
    }

    if held_back > 0 {
        println!();
        println!("  {held_back} item(s) are outside this machine's scopes ({wanted:?})");
    }

    if compared {
        let mine = orphans(&items, &remote);
        if !mine.is_empty() {
            println!();
            println!("  in the store, not sent from here");
            for name in mine {
                println!("  · {name}");
            }
        }
    } else {
        println!();
        println!("  no store named, so nothing was compared — try --backend memory");
    }

    Ok(())
}

/// What `apply` would change. Reads the machine, writes nothing.
pub fn plan(repo: &Path, colour: Colour, width: usize, json: bool) -> Result<()> {
    let home = home();
    let recipes_path = repo.join("kitbag.toml");
    let recipes = Recipes::load(&recipes_path)
        .with_context(|| format!("reading {}", recipes_path.display()))?;

    let platform = if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let manager = PackageManager::detect();

    let mut groups: Vec<Group> = Vec::new();
    let mut changes = 0usize;
    let mut as_json: Vec<serde_json::Value> = Vec::new();

    for recipe in &recipes.recipes {
        if !recipe.applies_here(platform) {
            continue;
        }
        let steps: Vec<Step> = plan_recipe(
            recipe,
            &home,
            repo,
            manager.as_ref(),
            cfg!(target_os = "macos"),
        );
        if steps.is_empty() {
            continue;
        }
        changes += steps.iter().filter(|s| s.action.is_change()).count();
        as_json.extend(steps.iter().map(|s| {
            serde_json::json!({
                "recipe": s.recipe,
                "id": s.id,
                "action": s.action.glyph().to_string(),
                "says": s.action.says(),
                "changes": s.action.is_change(),
            })
        }));

        groups.push(Group {
            // Recipes are not scoped by owner yet; the grouping shows the
            // recipe, which is what a plan is read by.
            scope: recipe
                .scope
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(Scope::Personal),
            owner: Some(recipe.name.clone()),
            rows: steps
                .into_iter()
                .map(|s| Row {
                    mark: Some(match s.action {
                        Action::None => Mark::Unchanged,
                        Action::Create(_) => Mark::New,
                        Action::Replace(_) => Mark::Changed,
                        Action::Unknown(_) => Mark::Unknown,
                    }),
                    detail: s.action.says().to_string(),
                    name: s.id,
                })
                .collect(),
        });
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&as_json)?);
        return Ok(());
    }

    if groups.is_empty() {
        println!();
        println!(
            "  No recipe in {} applies to this machine.",
            recipes_path.display()
        );
        return Ok(());
    }

    print!("{}", ui::render(&groups, colour, width));
    println!();
    if changes == 0 {
        println!("  Nothing to do — the machine already matches.");
    } else {
        println!(
            "  {changes} change(s). `kitbag apply` is not implemented yet, so nothing was written."
        );
    }
    Ok(())
}

fn pretty(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Make the machine match the recipes.
///
/// The plan is shown first and confirmed, because `apply` is the only command
/// here that writes: a person should see what is about to happen to their
/// machine while it is still about to happen. `--yes` skips the question, and
/// a run with nowhere to ask insists on it rather than assuming consent.
pub fn apply(
    repo: &Path,
    only: Option<&str>,
    yes: bool,
    colour: Colour,
    width: usize,
) -> Result<()> {
    let home = home();
    let recipes_path = repo.join("kitbag.toml");
    let recipes = Recipes::load(&recipes_path)
        .with_context(|| format!("reading {}", recipes_path.display()))?;

    let platform = if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let manager = PackageManager::detect();
    let with_defaults = cfg!(target_os = "macos");

    let chosen: Vec<_> = recipes
        .recipes
        .iter()
        .filter(|r| r.applies_here(platform))
        .filter(|r| only.is_none_or(|name| r.name == name))
        .collect();

    if chosen.is_empty() {
        println!();
        println!("  Nothing to apply: no recipe here matches.");
        return Ok(());
    }

    // What would change, before anything does.
    let mut pending = 0usize;
    let mut preview: Vec<Group> = Vec::new();
    for recipe in &chosen {
        let steps: Vec<Step> = plan_recipe(recipe, &home, repo, manager.as_ref(), with_defaults);
        pending += steps.iter().filter(|s| s.action.is_change()).count();
        preview.push(Group {
            scope: Scope::Personal,
            owner: Some(recipe.name.clone()),
            rows: steps
                .into_iter()
                .map(|s| Row {
                    mark: Some(mark_of_action(&s.action)),
                    detail: s.action.says().to_string(),
                    name: s.id,
                })
                .collect(),
        });
    }
    print!("{}", ui::render(&preview, colour, width));

    if pending == 0 {
        println!();
        println!("  Nothing to do — the machine already matches.");
        return Ok(());
    }

    if !yes && !confirm(pending)? {
        println!("  Left alone.");
        return Ok(());
    }

    println!();
    let mut changed = 0usize;
    let mut problems = 0usize;
    for recipe in &chosen {
        for applied in apply_recipe(recipe, &home, repo, manager.as_ref(), with_defaults) {
            if applied.done == Done::Skipped {
                continue;
            }
            if applied.done.changed() {
                changed += 1;
            }
            if applied.done.is_problem() {
                problems += 1;
            }
            println!(
                "  {} {:<28} {}",
                applied.done.glyph(),
                applied.id,
                applied.done.says()
            );
        }
    }

    println!();
    println!("  {changed} change(s) made.");
    if problems > 0 {
        println!("  {problems} could not be done — each says why above.");
    }
    Ok(())
}

fn mark_of_action(action: &Action) -> Mark {
    match action {
        Action::None => Mark::Unchanged,
        Action::Create(_) => Mark::New,
        Action::Replace(_) => Mark::Changed,
        Action::Unknown(_) => Mark::Unknown,
    }
}

fn confirm(pending: usize) -> Result<bool> {
    use std::io::{IsTerminal, Write};

    if !std::io::stdin().is_terminal() {
        println!();
        println!("  {pending} change(s) to make, and no terminal to ask in. Re-run with --yes.");
        return Ok(false);
    }
    print!("\n  Apply {pending} change(s)? [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

fn open_store(name: Option<&str>) -> Result<Box<dyn Backend>> {
    let name = name.unwrap_or("bw");
    let kind: BackendKind = name.parse()?;
    kind.open()
        .with_context(|| format!("opening the {name} store"))
}

/// Send what this machine holds to the store.
///
/// An item the store already holds byte for byte is not sent: comparing by
/// hash means a machine with nothing to say costs one listing, and a vault
/// does not fill up with versions of a file that never changed.
pub fn push(
    backend: Option<&str>,
    wanted: Option<Wanted>,
    dry_run: bool,
    only: &[String],
    colour: Colour,
) -> Result<()> {
    let home = home();
    let config = Config::load_or_default(&config_path())?.with_env_skips();
    let wanted = wanted.unwrap_or_else(|| config.wanted());
    let Collected { items, skipped } = collect(&config, &home);

    let progress = ui::Progress::new(colour);
    progress.say("opening the store");
    let store = open_store(backend)?;

    progress.say("reading what the store holds");
    let remote: Remote = store
        .list()?
        .into_iter()
        .map(|l| (l.name, l.fingerprint))
        .collect();
    let total = items
        .iter()
        .filter(|i| wanted.accepts(&i.scope) && !config.skips(&i.name))
        .filter(|i| only.is_empty() || only.iter().any(|n| n == &i.name))
        .count();
    let mut at = 0usize;
    // Named by this machine as its own business, in both directions.
    let mut kept_back: Vec<String> = Vec::new();
    progress.clear();

    let mut sent = 0usize;
    let mut same = 0usize;
    // One item the store will not take must not decide the fate of the other
    // thirty-eight. Each failure is reported where it happens, the run
    // continues, and the exit status at the end is the whole truth about it.
    let mut failed: Vec<(String, String)> = Vec::new();
    println!();
    for item in &items {
        if !wanted.accepts(&item.scope) {
            continue;
        }
        // Named on the command line: this run is about resolving one item,
        // and everything else stays where it is.
        if !only.is_empty() && !only.iter().any(|n| n == &item.name) {
            continue;
        }
        if config.skips(&item.name) {
            kept_back.push(item.name.clone());
            continue;
        }
        at += 1;
        progress.say(format!("{} — {at}/{total}", item.name));
        match compare(item, &remote, &home) {
            State::Unchanged => {
                same += 1;
                continue;
            }
            state => {
                if dry_run {
                    progress.clear();
                    println!("  {} {:<28} would be sent", state.glyph(), item.name);
                } else {
                    let envelope = kitbag_core::collect::envelope_for(item, &home);
                    let outcome = store.put(&item.name, &envelope);
                    progress.clear();
                    match outcome {
                        Ok(()) => println!("  {} {:<28} sent", state.glyph(), item.name),
                        Err(e) => {
                            println!("  ✗ {:<28} {}", item.name, root_cause(&e));
                            failed.push((item.name.clone(), root_cause(&e)));
                            continue;
                        }
                    }
                }
                sent += 1;
            }
        }
    }

    drop(progress);
    println!();
    if dry_run {
        println!("  {sent} to send, {same} already there. Nothing was written.");
    } else {
        println!("  {sent} sent, {same} already there.");
    }
    if !skipped.is_empty() {
        println!(
            "  {} not tracked — `kitbag status` says why.",
            skipped.len()
        );
    }
    if !kept_back.is_empty() {
        println!(
            "  {} kept on this machine: {}",
            kept_back.len(),
            kept_back.join(", ")
        );
    }
    if !failed.is_empty() {
        println!("  {} not sent:", failed.len());
        for (name, why) in &failed {
            println!("    {name}: {why}");
        }
        bail!("{} item(s) did not reach the store", failed.len());
    }
    Ok(())
}

/// The bottom of an error chain: what `bw` or the filesystem actually said,
/// rather than the sentence kitbag wrapped around it.
fn root_cause(e: &anyhow::Error) -> String {
    e.chain().last().map(|c| c.to_string()).unwrap_or_default()
}

/// Write what the store holds back onto this machine.
///
/// Only the scopes this machine takes, and every file it would overwrite is
/// kept first: a restore that quietly replaces something is the one operation
/// here nobody can undo.
pub fn restore(
    backend: Option<&str>,
    wanted: Option<Wanted>,
    dry_run: bool,
    only: &[String],
    colour: Colour,
) -> Result<()> {
    let home = home();
    let config = Config::load_or_default(&config_path())?.with_env_skips();
    let wanted = wanted.unwrap_or_else(|| config.wanted());
    let Collected { items, .. } = collect(&config, &home);

    // Where each name belongs, learnt from what this machine already tracks.
    let known: BTreeMap<&str, &Path> = items
        .iter()
        .filter_map(|i| match &i.source {
            kitbag_core::collect::Source::File(path) => Some((i.name.as_str(), path.as_path())),
            kitbag_core::collect::Source::Command { .. } => None,
        })
        .collect();

    // And which names go back through a command rather than onto a path.
    let commands: BTreeMap<&str, &str> = items
        .iter()
        .filter_map(|i| match &i.source {
            kitbag_core::collect::Source::Command { restore } => {
                Some((i.name.as_str(), restore.as_str()))
            }
            kitbag_core::collect::Source::File(_) => None,
        })
        .collect();

    let progress = ui::Progress::new(colour);
    progress.say("opening the store");
    let store = open_store(backend)?;

    let mut written = 0usize;
    let mut same = 0usize;
    let mut unplaceable = Vec::new();
    // Items belonging to a platform that is not this one.
    let mut elsewhere: Vec<String> = Vec::new();
    // Items this machine has said it keeps for itself.
    let mut kept_back: Vec<String> = Vec::new();
    let here = kitbag_core::this_platform();

    progress.say("reading what the store holds");
    let listings = store.list()?;
    let total = listings.len();

    progress.clear();
    println!();
    for (at, listing) in listings.into_iter().enumerate() {
        progress.say(format!("{} — {}/{total}", listing.name, at + 1));
        let name = listing.name.as_str();

        // Refusals that cost nothing come first. A store that reports the
        // scope in its listing can have an item turned away without the item
        // ever being fetched.
        if listing.scope.as_ref().is_some_and(|s| !wanted.accepts(s)) {
            continue;
        }
        // Named on the command line: this run is about resolving one item.
        if !only.is_empty() && !only.iter().any(|n| n == name) {
            continue;
        }
        // Named by this machine as its own business. Cheap, and before
        // anything is fetched.
        if config.skips(name) {
            kept_back.push(listing.name.clone());
            continue;
        }
        // A macOS keychain, or a bundle addressed to ~/Library, is not state
        // this machine has anywhere to put. Writing it anyway is worse than
        // skipping it, because it looks like it worked.
        if listing
            .platform
            .as_ref()
            .is_some_and(|p| !p.is_empty() && !p.iter().any(|one| one == here))
        {
            elsewhere.push(listing.name.clone());
            continue;
        }

        // State an application owns goes back through the application, which
        // is the only thing that knows what to do with it. A dry run only says
        // so, and saying so does not need the bytes.
        if let Some(restore) = commands.get(name) {
            if dry_run && listing.scope.is_some() {
                progress.clear();
                println!("  ~ {name:<28} would be piped into `{restore}`");
                written += 1;
                continue;
            }
            let envelope = store.get(name)?;
            if !wanted.accepts(&envelope.scope) {
                continue;
            }
            if dry_run {
                progress.clear();
                println!("  ~ {name:<28} would be piped into `{restore}`");
                written += 1;
                continue;
            }
            progress.clear();
            match pipe_into(restore, &envelope.payload) {
                Ok(()) => {
                    println!("  + {name:<28} into `{restore}`");
                    written += 1;
                }
                Err(e) => println!("  ! {name:<28} {e}"),
            }
            continue;
        }

        // A file already here, identical to what the store holds, needs nothing
        // fetched to establish that: the store reported the hash, and hashing
        // what is on disk is free beside a round trip to the vault.
        if let (Some(dest), Some(there)) = (known.get(name), listing.payload_hash.as_deref()) {
            if std::fs::read(dest)
                .map(|bytes| kitbag_core::payload_hash(&bytes) == there)
                .unwrap_or(false)
            {
                same += 1;
                continue;
            }
        }

        let envelope = store.get(name)?;
        if !wanted.accepts(&envelope.scope) {
            continue;
        }
        if !envelope.belongs_on(here) {
            elsewhere.push(listing.name.clone());
            continue;
        }

        // The item says how it goes back. This is reached only when this
        // machine does not already track it — a machine being restored does
        // not have the application installed, so its own config cannot know —
        // and the command is named before it runs, because it came from the
        // store rather than from anything here.
        if let Some(restore) = envelope.restore.clone() {
            progress.clear();
            if dry_run {
                println!("  ~ {name:<28} would be piped into `{restore}`  (from the store)");
                written += 1;
                continue;
            }
            match pipe_into(&restore, &envelope.payload) {
                Ok(()) => {
                    println!("  + {name:<28} into `{restore}`  (from the store)");
                    written += 1;
                }
                Err(e) => println!("  ! {name:<28} {e}"),
            }
            continue;
        }

        // Where it belongs: what the envelope says, else where this machine
        // already keeps it. The first is what makes a restore work on a
        // machine where the file does not exist yet, which is most of them.
        let dest = match envelope.destination(&home) {
            Some(path) => path,
            None => match known.get(name) {
                Some(path) => path.to_path_buf(),
                None => {
                    unplaceable.push(listing.name.clone());
                    continue;
                }
            },
        };
        let dest = &dest;

        if std::fs::read(dest)
            .map(|b| b == envelope.payload)
            .unwrap_or(false)
        {
            same += 1;
            continue;
        }
        if dry_run {
            progress.clear();
            println!(
                "  ~ {:<28} would be written to {}",
                listing.name,
                pretty(dest, &home)
            );
            written += 1;
            continue;
        }

        let kept = place(dest, &envelope.payload)
            .with_context(|| format!("writing {}", pretty(dest, &home)))?;
        progress.clear();
        match kept {
            Some(backup) => println!(
                "  ~ {:<28} {} — kept the old one at {}",
                listing.name,
                pretty(dest, &home),
                pretty(&backup, &home)
            ),
            None => println!("  + {:<28} {}", listing.name, pretty(dest, &home)),
        }
        written += 1;
    }

    drop(progress);
    println!();
    if dry_run {
        println!("  {written} to write, {same} already here. Nothing was written.");
    } else {
        println!("  {written} written, {same} already here.");
    }
    if !kept_back.is_empty() {
        println!();
        println!(
            "  {} kept as this machine's own, not taken from the store:",
            kept_back.len()
        );
        for name in &kept_back {
            println!("  · {name}");
        }
    }
    if !elsewhere.is_empty() {
        println!();
        println!(
            "  {} for another platform, so not written here:",
            elsewhere.len()
        );
        for name in &elsewhere {
            println!("  · {name}");
        }
    }
    if !unplaceable.is_empty() {
        println!();
        println!(
            "  in the store, but with no path and no way back, so there is nowhere to put them:"
        );
        for name in &unplaceable {
            println!("  · {name}");
        }
        println!("  add a [[track]] entry for each, then restore again.");
    }
    Ok(())
}

/// Write a payload where it belongs, keeping whatever was there.
///
/// Returns where the old file went, if there was one. A restore is the one
/// operation here that cannot be undone by running it again, so nothing it
/// replaces is thrown away - and what it writes is readable by its owner and
/// nobody else, because most of what comes back through here is a secret.
fn place(dest: &Path, payload: &[u8]) -> Result<Option<PathBuf>> {
    use std::os::unix::fs::PermissionsExt;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let kept = if dest.exists() {
        let backup = backup_beside(dest);
        std::fs::rename(dest, &backup)?;
        Some(backup)
    } else {
        None
    };
    std::fs::write(dest, payload)?;
    std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o600))?;
    Ok(kept)
}

/// Hand a payload to a command on its stdin.
fn pipe_into(command: &str, payload: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .as_mut()
        .expect("stdin was piped")
        .write_all(payload)?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!(
            "{}",
            err.lines().next().unwrap_or("the command failed").trim()
        );
    }
    Ok(())
}

fn backup_beside(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".backup.{stamp}"));
    PathBuf::from(name)
}

fn dismissed_path() -> PathBuf {
    config_path()
        .parent()
        .unwrap_or(Path::new("."))
        .join("dismissed")
}

fn dismissed() -> Vec<PathBuf> {
    std::fs::read_to_string(dismissed_path())
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// What is here that nothing is tracking yet.
///
/// The question a person cannot answer for themselves is *what have I
/// forgotten* — and the usual way to find out is to hit the missing thing a
/// week later, in the middle of something else.
pub fn discover(write: bool, dismiss: Option<&str>, json: bool) -> Result<()> {
    let home = home();
    let cfg_path = config_path();

    if let Some(path) = dismiss {
        let full = PathBuf::from(kitbag_core::config::expand(path, &home));
        let file = dismissed_path();
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut list = std::fs::read_to_string(&file).unwrap_or_default();
        list.push_str(&format!("{}\n", full.display()));
        std::fs::write(&file, list)?;
        println!(
            "  dismissed {} — it will not come up again",
            pretty(&full, &home)
        );
        return Ok(());
    }

    let config = Config::load_or_default(&cfg_path)?.with_env_skips();
    let tracked: Vec<PathBuf> = collect(&config, &home)
        .items
        .into_iter()
        .map(|i| i.path)
        .collect();

    let found = scan(&home, &tracked, &dismissed());

    if json {
        let out: Vec<_> = found
            .iter()
            .map(|f| {
                serde_json::json!({
                    "path": f.shown(&home),
                    "scope": f.scope,
                    "why": f.why,
                    "source": match f.source {
                        Source::Catalogue => "catalogue",
                        Source::Noticed => "noticed",
                    },
                    "toml": f.as_toml(&home),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if found.is_empty() {
        println!();
        println!(
            "  Nothing new. Everything this knows to look for is either tracked or dismissed."
        );
        return Ok(());
    }

    println!();
    for f in &found {
        let source = match f.source {
            Source::Catalogue => "known place",
            Source::Noticed => "noticed",
        };
        println!(
            "  {} {:<44} {} · {}",
            if f.scope == "auto" { '?' } else { '+' },
            f.shown(&home),
            f.why,
            source
        );
    }

    println!();
    if write {
        append_tracks(&cfg_path, &found, &home)?;
        println!(
            "  Added {} entry(s) to {}.",
            found.len(),
            cfg_path.display()
        );
        println!("  A file marked ? must carry its own `# scope:` line, or it will be skipped.");
    } else {
        println!("  {} not tracked. To take them all:", found.len());
        println!("    kitbag discover --write");
        println!("  or paste what you want into {}:", cfg_path.display());
        println!();
        for f in &found {
            for line in f.as_toml(&home).lines() {
                println!("    {line}");
            }
        }
        println!("    (and `kitbag discover --dismiss <path>` for the ones you never want)");
    }
    Ok(())
}

/// Add one path by hand.
pub fn track(path: &str, scope: Option<&str>, owner: Option<&str>) -> Result<()> {
    let home = home();
    let cfg_path = config_path();
    let full = PathBuf::from(kitbag_core::config::expand(path, &home));

    if !full.exists() && !path.contains('*') {
        println!(
            "  {} is not here. Tracking it anyway — say so if that is a typo.",
            pretty(&full, &home)
        );
    }

    let mut entry = format!("\n[[track]]\npath = \"{path}\"\n");
    if let Some(s) = scope {
        // Refuse a scope this version does not know, rather than writing a
        // config that will fail quietly on the next run.
        let _: kitbag_core::Scope = s.parse()?;
        entry.push_str(&format!("scope = \"{s}\"\n"));
    }
    if let Some(o) = owner {
        entry.push_str(&format!("owner = \"{o}\"\n"));
    }

    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut current = std::fs::read_to_string(&cfg_path).unwrap_or_default();
    current.push_str(&entry);
    std::fs::write(&cfg_path, current)?;

    println!("  tracking {} in {}", path, cfg_path.display());
    if scope.is_none() {
        println!("  no scope given, so the file must carry its own `# scope:` line.");
    }
    Ok(())
}

fn append_tracks(cfg_path: &Path, found: &[Finding], home: &Path) -> Result<()> {
    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut current = std::fs::read_to_string(cfg_path).unwrap_or_default();
    for f in found {
        current.push('\n');
        current.push_str(&f.as_toml(home));
    }
    std::fs::write(cfg_path, current)?;
    Ok(())
}

/// What differs between this machine and the store, item by item.
///
/// `status` says an item changed. Acting on that means knowing what changed,
/// and until now that meant reading both sides by hand. This reads them and
/// says what it found — key names, counts, sizes, hashes — and stops there,
/// because which side should win is not a thing a tool can know.
pub fn diff(backend: Option<&str>, only: &[String], colour: Colour) -> Result<()> {
    use kitbag_core::difference::{describe, Difference};

    let home = home();
    let config = Config::load_or_default(&config_path())?.with_env_skips();
    let Collected { items, .. } = collect(&config, &home);

    let progress = ui::Progress::new(colour);
    progress.say("reading what the store holds");
    let store = open_store(backend)?;
    let listing: BTreeMap<String, Option<String>> = store
        .list()?
        .into_iter()
        .map(|l| (l.name, l.fingerprint))
        .collect();
    progress.clear();

    let wanted: Vec<&kitbag_core::Item> = items
        .iter()
        .filter(|i| only.is_empty() || only.iter().any(|n| n == &i.name))
        .collect();

    let mut shown = 0usize;
    println!();
    for item in wanted {
        let Some(there_print) = listing.get(&item.name) else {
            println!("  + {:<28} not in the store", item.name);
            shown += 1;
            continue;
        };
        let here_print = kitbag_core::collect::envelope_for(item, &home).fingerprint();
        if there_print.as_deref() == Some(here_print.as_str()) {
            continue;
        }

        progress.say(format!("reading {}", item.name));
        let theirs = store.get(&item.name)?;
        progress.clear();

        let what = describe(&item.payload, &theirs.payload);
        if what.is_none() {
            // Bytes equal, envelope not: a marker moved, not a value.
            println!("  ~ {:<28} same contents, different markers", item.name);
            shown += 1;
            continue;
        }

        println!("  ~ {}", item.name);
        match what {
            Difference::Keys {
                only_here,
                only_there,
                differing,
                here_lines,
                there_lines,
            } => {
                println!("      here   {here_lines} lines");
                println!("      store  {there_lines} lines");
                if !only_here.is_empty() {
                    println!("      only here:   {}", only_here.join(" "));
                }
                if !only_there.is_empty() {
                    println!("      only there:  {}", only_there.join(" "));
                }
                if !differing.is_empty() {
                    println!("      differ:      {}", differing.join(" "));
                }
            }
            Difference::Opaque {
                here_bytes,
                there_bytes,
                here_hash,
                there_hash,
            } => {
                println!("      here   {here_bytes} bytes  {}", &here_hash[..12]);
                println!("      store  {there_bytes} bytes  {}", &there_hash[..12]);
            }
            Difference::None => unreachable!("handled above"),
        }
        shown += 1;
    }

    drop(progress);
    println!();
    if shown == 0 {
        println!("  Nothing differs.");
    } else {
        println!("  {shown} item(s) differ. Which side should win is yours to say:");
        println!("    kitbag restore --backend <name> --only <item>   take the store's");
        println!("    kitbag push    --backend <name> --only <item>   send this machine's");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_restored_secret_is_readable_by_its_owner_and_nobody_else() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("deep/.envs/a.env");

        let kept = place(&dest, b"TOKEN=x\n").unwrap();

        assert_eq!(kept, None, "there was nothing to keep");
        assert_eq!(std::fs::read(&dest).unwrap(), b"TOKEN=x\n");
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
    }

    #[test]
    fn what_was_there_is_kept_never_overwritten_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("a.env");
        std::fs::write(&dest, b"the one that was here\n").unwrap();

        let kept = place(&dest, b"the one from the store\n")
            .unwrap()
            .expect("a backup");

        assert_eq!(std::fs::read(&kept).unwrap(), b"the one that was here\n");
        assert_eq!(std::fs::read(&dest).unwrap(), b"the one from the store\n");
    }
}
