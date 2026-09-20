//! The two commands that have an engine behind them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use kitbag_core::collect::{collect, Collected};
use kitbag_core::recipe::Recipes;
use kitbag_core::state::{compare, orphans, Remote, State};
use kitbag_core::Envelope;
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
) -> Result<()> {
    let home = home();
    let cfg_path = config_path();
    let config = Config::load_or_default(&cfg_path)?;
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
                .map(|l| (l.name, l.payload_hash))
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
        let mark = compared.then(|| mark_of(compare(item, &remote)));
        groups
            .entry((item.scope.name().to_string(), item.owner.clone()))
            .or_default()
            .push(Row {
                mark,
                name: item.name.clone(),
                detail: item.detail(),
            });
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
pub fn plan(repo: &Path, colour: Colour, width: usize) -> Result<()> {
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
pub fn push(backend: Option<&str>, wanted: Option<Wanted>, dry_run: bool) -> Result<()> {
    let home = home();
    let config = Config::load_or_default(&config_path())?;
    let wanted = wanted.unwrap_or_else(|| config.wanted());
    let Collected { items, skipped } = collect(&config, &home);

    let store = open_store(backend)?;
    let remote: Remote = store
        .list()?
        .into_iter()
        .map(|l| (l.name, l.payload_hash))
        .collect();

    let mut sent = 0usize;
    let mut same = 0usize;
    println!();
    for item in &items {
        if !wanted.accepts(&item.scope) {
            continue;
        }
        match compare(item, &remote) {
            State::Unchanged => {
                same += 1;
                continue;
            }
            state => {
                if dry_run {
                    println!("  {} {:<28} would be sent", state.glyph(), item.name);
                } else {
                    let envelope = Envelope::new(item.scope.clone(), item.payload.clone())
                        .with_owner(item.owner.clone());
                    store
                        .put(&item.name, &envelope)
                        .with_context(|| format!("sending {}", item.name))?;
                    println!("  {} {:<28} sent", state.glyph(), item.name);
                }
                sent += 1;
            }
        }
    }

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
    Ok(())
}

/// Write what the store holds back onto this machine.
///
/// Only the scopes this machine takes, and every file it would overwrite is
/// kept first: a restore that quietly replaces something is the one operation
/// here nobody can undo.
pub fn restore(backend: Option<&str>, wanted: Option<Wanted>, dry_run: bool) -> Result<()> {
    let home = home();
    let config = Config::load_or_default(&config_path())?;
    let wanted = wanted.unwrap_or_else(|| config.wanted());
    let Collected { items, .. } = collect(&config, &home);

    // Where each name belongs, learnt from what this machine already tracks.
    let known: BTreeMap<&str, &Path> = items
        .iter()
        .map(|i| (i.name.as_str(), i.path.as_path()))
        .collect();

    let store = open_store(backend)?;
    let mut written = 0usize;
    let mut same = 0usize;
    let mut unplaceable = Vec::new();

    println!();
    for listing in store.list()? {
        let envelope = store.get(&listing.name)?;
        if !wanted.accepts(&envelope.scope) {
            continue;
        }
        let Some(dest) = known.get(listing.name.as_str()) else {
            // The store knows an item this machine has never tracked, so there
            // is nowhere to put it without guessing at a path.
            unplaceable.push(listing.name.clone());
            continue;
        };

        if std::fs::read(dest)
            .map(|b| b == envelope.payload)
            .unwrap_or(false)
        {
            same += 1;
            continue;
        }
        if dry_run {
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

    println!();
    if dry_run {
        println!("  {written} to write, {same} already here. Nothing was written.");
    } else {
        println!("  {written} written, {same} already here.");
    }
    if !unplaceable.is_empty() {
        println!();
        println!("  in the store, but this machine does not track them, so there is nowhere to put them:");
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

fn backup_beside(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".backup.{stamp}"));
    PathBuf::from(name)
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
