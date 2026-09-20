//! The two commands that have an engine behind them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use kitbag_core::collect::{collect, Collected};
use kitbag_core::recipe::Recipes;
use kitbag_core::state::{compare, orphans, Remote, State};
use kitbag_core::{Config, Scope, Wanted};
use kitbag_providers::{apply_recipe, plan_recipe, Action, Done, PackageManager, Step};
use kitbag_vault::BackendKind;

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
