//! The two commands that have an engine behind them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use kitbag_catalog::{scan, Finding, Source};
use kitbag_core::collect::{collect, Collected};
use kitbag_core::recipe::Recipes;
use kitbag_core::state::{orphans, Remote, State};
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
        State::Ahead => Mark::Ahead,
        State::Behind => Mark::Behind,
        State::Conflict => Mark::Conflict,
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
    let ledger = kitbag_core::ledger::Ledger::load(&ledger_path());

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
        let mark = compared.then(|| {
            mark_of(kitbag_core::state::compare_against(
                item, &remote, &home, &ledger,
            ))
        });
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
                "state": compared.then(|| match kitbag_core::state::compare_against(i, &remote, &home, &ledger) {
                    State::New => "new",
                    State::Changed => "changed",
                    State::Ahead => "ahead",
                    State::Behind => "behind",
                    State::Conflict => "conflict",
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

/// The one question `restore` asks. It writes over this machine's own files
/// and runs the restore commands the store carries, and both of those are
/// worth saying out loud before the first one happens.
fn confirm_restore(held: usize) -> Result<bool> {
    use std::io::{IsTerminal, Write};

    println!();
    println!("  The store holds {held} item(s) for this machine.");
    println!("  Restoring writes over files here and runs the restore commands");
    println!("  the store carries — which can install software.");
    println!("  `kitbag restore --dry-run` says exactly what, and writes nothing.");

    if !std::io::stdin().is_terminal() {
        println!();
        println!("  No terminal to ask in. Re-run with --yes.");
        return Ok(false);
    }
    print!("\n  Go ahead? [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

fn open_store(name: Option<&str>) -> Result<Box<dyn Backend + Send + Sync>> {
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
    jobs: usize,
    colour: Colour,
) -> Result<usize> {
    let home = home();
    let config = Config::load_or_default(&config_path())?.with_env_skips();
    let wanted = wanted.unwrap_or_else(|| config.wanted());
    let Collected { items, skipped } = collect(&config, &home);

    let mut ledger = kitbag_core::ledger::Ledger::load(&ledger_path());

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
    // Differences this run refused to settle on its own.
    let mut held: Vec<(String, State)> = Vec::new();
    // The store is ahead on these; they are a restore's business.
    let mut behind: Vec<String> = Vec::new();
    // Refused mid-run, and not worth contesting: see the volatile note below.
    let mut yielded: Vec<String> = Vec::new();
    // Decided, not yet written.
    let mut outgoing: Vec<(&kitbag_core::Item, State)> = Vec::new();
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
        let state = kitbag_core::state::compare_against(item, &remote, &home, &ledger);

        // Named on the command line means the question has been answered, so a
        // conflict is no longer one. Otherwise it is held back: pushing over a
        // store that has also moved writes somebody's work away, and nothing
        // here knows whose.
        if !state.is_decided() && only.is_empty() {
            held.push((item.name.clone(), state));
            continue;
        }

        // The store moved and this machine did not: sending is writing the
        // older copy over the newer one. That is a restore, not a push.
        if matches!(state, State::Behind) {
            behind.push(item.name.clone());
            continue;
        }

        match state {
            State::Unchanged => {
                // Agreement is exactly what this records. Writing it only
                // after a transfer means an item that was already identical
                // never gets a base, and can never be told apart from one
                // nobody has a record of.
                ledger.record(
                    &item.name,
                    &kitbag_core::collect::envelope_for(item, &home).fingerprint(),
                );
                same += 1;
                continue;
            }
            state => {
                if dry_run {
                    progress.clear();
                    println!("  {} {:<28} would be sent", state.glyph(), item.name);
                    sent += 1;
                } else {
                    // Decided here, written below. Choosing what to send is
                    // local and instant; sending it is a process per item, and
                    // that is the entire wait.
                    outgoing.push((item, state));
                }
            }
        }
    }

    if !outgoing.is_empty() {
        let results = send_all(&*store, &outgoing, &home, jobs, &progress);
        progress.clear();
        for (name, state, outcome) in results {
            match outcome {
                Ok(fingerprint) => {
                    // They agree now, and that is the point to compare against
                    // next time.
                    ledger.record(&name, &fingerprint);
                    println!("  {} {:<28} sent", state.glyph(), name);
                    sent += 1;
                }
                Err(why) => {
                    println!("  ✗ {name:<28} {why}");
                    failed.push((name, why));
                }
            }
        }
    }

    // Four machines pushing at the same moment is the ordinary case here, not
    // the unlucky one, and the store refuses a write whose base it has since
    // moved past: "the client copy of this cipher is out of date". That is not
    // a result to report and stop on — it is a reason to look again.
    //
    // Looking again means comparing afresh, not sending harder. Between the
    // decision and the write, another machine put something there; whether
    // this item is still this machine's to send is exactly the question the
    // comparison answers, and an item that has become a conflict in the last
    // four seconds is a conflict.
    if !dry_run && failed.iter().any(|(_, why)| why.contains("out of date")) {
        let (stale, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut failed)
            .into_iter()
            .partition(|(_, why)| why.contains("out of date"));
        failed = rest;

        progress.say("another machine wrote during this run — reading the store again");
        store.refresh()?;
        let remote: Remote = store
            .list()?
            .into_iter()
            .map(|l| (l.name, l.fingerprint))
            .collect();

        let mut again: Vec<(&kitbag_core::Item, State)> = Vec::new();
        for (name, why) in stale {
            let Some(item) = items.iter().find(|i| i.name == name) else {
                failed.push((name, why));
                continue;
            };

            // Nobody can compare these bytes — that is what `volatile` means —
            // so a copy that arrived from another machine four seconds ago is
            // exactly as good as this one, and there is no prize for winning
            // the argument. Four machines sending the same incomparable item
            // on every push will collide forever otherwise, each refusal
            // reported as a failure nobody can act on.
            if item.volatile {
                progress.clear();
                println!("  ? {name:<28} another machine's copy landed first");
                yielded.push(name);
                continue;
            }

            let state = kitbag_core::state::compare_against(item, &remote, &home, &ledger);
            if !state.is_decided() && only.is_empty() {
                held.push((name, state));
            } else if matches!(state, State::Behind) {
                behind.push(name);
            } else if matches!(state, State::Unchanged) {
                // The other machine wrote what this one was about to. There is
                // nothing left to send and the two now agree, which is the
                // thing worth recording.
                ledger.record(
                    &name,
                    &kitbag_core::collect::envelope_for(item, &home).fingerprint(),
                );
                same += 1;
            } else {
                again.push((item, state));
            }
        }

        if !again.is_empty() {
            let results = send_all(&*store, &again, &home, jobs, &progress);
            progress.clear();
            for (name, state, outcome) in results {
                match outcome {
                    Ok(fingerprint) => {
                        ledger.record(&name, &fingerprint);
                        println!("  {} {:<28} sent, second time", state.glyph(), name);
                        sent += 1;
                    }
                    Err(why) => {
                        println!("  ✗ {name:<28} {why}");
                        failed.push((name, why));
                    }
                }
            }
        }
        progress.clear();
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
    if !dry_run {
        if let Err(e) = ledger.save(&ledger_path()) {
            println!("  could not record what was exchanged: {e}");
        }
    }
    if !yielded.is_empty() {
        println!(
            "  {} left to another machine's copy: {}",
            yielded.len(),
            yielded.join(", ")
        );
        println!("  Nothing can compare those bytes, so whichever arrived is as good.");
    }
    if !behind.is_empty() {
        println!(
            "  {} newer in the store, so not sent: {}",
            behind.len(),
            behind.join(", ")
        );
        println!("  kitbag restore takes those.");
    }
    report_held(&held, "push");
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
    Ok(held.len())
}

/// What one send came to: the item, where it stood, and either the
/// fingerprint the two now agree on or what went wrong.
type Sent = (String, State, std::result::Result<String, String>);

/// Send them, several at a time.
///
/// Every write is a process started, and starting them one after another is
/// the whole of the wait: four `bw` calls take 9.8s in a row and 1.9s at once,
/// on the machine this was measured on. The client keeps a lock on its own
/// vault file, so the parallelism is in the waiting, not in the writing.
///
/// Order comes out as things finish rather than as they were listed. That is
/// what actually happened, and pretending otherwise would mean holding the
/// whole run back to print it tidily.
fn send_all(
    store: &(dyn Backend + Send + Sync),
    outgoing: &[(&kitbag_core::Item, State)],
    home: &Path,
    jobs: usize,
    progress: &ui::Progress,
) -> Vec<Sent> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let out: Mutex<Vec<Sent>> = Mutex::new(Vec::with_capacity(outgoing.len()));
    let total = outgoing.len();
    let workers = jobs.clamp(1, 16).min(total.max(1));

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let at = next.fetch_add(1, Ordering::Relaxed);
                let Some((item, state)) = outgoing.get(at) else {
                    return;
                };
                let envelope = kitbag_core::collect::envelope_for(item, home);
                let result = match store.put(&item.name, &envelope) {
                    Ok(()) => Ok(envelope.fingerprint()),
                    Err(e) => Err(explain_failure(&e)),
                };
                let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                progress.say(format!("{} — {finished}/{total}", item.name));
                out.lock().unwrap_or_else(|e| e.into_inner()).push((
                    item.name.clone(),
                    *state,
                    result,
                ));
            });
        }
    });

    out.into_inner().unwrap_or_else(|e| e.into_inner())
}

/// What went wrong, in words that say what to do about it.
///
/// The store refuses a write built from a copy older than the one it holds,
/// which is how it stops two machines overwriting each other — and which is
/// exactly what happens when two of them push at once. Reported as the client
/// words it, that reads as a mystery.
fn explain_failure(e: &anyhow::Error) -> String {
    let said = root_cause(e);
    if said.contains("out of date") {
        return format!("{said} (another machine changed it during this run — push again)");
    }
    said
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
    yes: bool,
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

    let mut ledger = kitbag_core::ledger::Ledger::load(&ledger_path());
    // What this machine holds, by name, so a difference can be classified
    // before anything is fetched or written.
    let mine: BTreeMap<&str, &kitbag_core::Item> =
        items.iter().map(|i| (i.name.as_str(), i)).collect();

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
    // Differences this run refused to settle on its own.
    let mut held: Vec<(String, State)> = Vec::new();
    // This machine is ahead on these; they are a push's business.
    let mut ahead: Vec<String> = Vec::new();
    // Another machine's own, kept in the store for that machine.
    let mut theirs: Vec<String> = Vec::new();
    // Where each item is going, so two of them cannot go to one place.
    let mut claimed: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut contested: Vec<(String, String, String)> = Vec::new();
    let this_machine = config.machine_name();
    let here = kitbag_core::this_platform();

    progress.say("reading what the store holds");
    let listings = store.list()?;
    let total = listings.len();

    // Asked before anything is written, and once. `apply` has always confirmed
    // and `restore` never did, which was defensible while restoring meant
    // writing files — those are backed up first, and a backup can be put back.
    // It stopped being defensible when a tracked `programs` item made
    // `restore` able to install software: that reaches the network, takes
    // minutes, and no backup undoes it.
    //
    // Not a plan, because building one means fetching every payload to find
    // out — twice the calls, and every secret held twice as long for a
    // question `--dry-run` already answers properly.
    //
    // Skipped for `--only`, where naming the item is the answer.
    if !dry_run && !yes && only.is_empty() {
        progress.clear();
        if !confirm_restore(total)? {
            println!();
            println!("  Nothing was written.");
            return Ok(());
        }
    }

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
        // Another machine's, by name. Writing it here would put two machines
        // on one key, which is what naming it after a machine prevented.
        if listing
            .machine
            .as_deref()
            .is_some_and(|m| m != this_machine.as_str())
        {
            theirs.push(listing.name.clone());
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

        // Both sides moved since they last agreed, so taking the store's copy
        // writes this machine's work away. Held, unless it was named.
        if only.is_empty() {
            if let (Some(item), Some(there)) = (mine.get(name), listing.fingerprint.as_deref()) {
                let one = Remote::from([(name.to_string(), Some(there.to_string()))]);
                let state = kitbag_core::state::compare_against(item, &one, &home, &ledger);
                if !state.is_decided() {
                    held.push((listing.name.clone(), state));
                    continue;
                }
                // This machine moved and the store did not: taking is writing
                // the older copy over the newer one. That is a push.
                if matches!(state, State::Ahead) {
                    ahead.push(listing.name.clone());
                    continue;
                }
            }
        }

        // A file already here, identical to what the store holds, needs nothing
        // fetched to establish that: the store reported the hash, and hashing
        // what is on disk is free beside a round trip to the vault.
        if let (Some(dest), Some(there)) = (known.get(name), listing.payload_hash.as_deref()) {
            if std::fs::read(dest)
                .map(|bytes| kitbag_core::payload_hash(&bytes) == there)
                .unwrap_or(false)
            {
                if let Some(item) = mine.get(name) {
                    ledger.record(
                        name,
                        &kitbag_core::collect::envelope_for(item, &home).fingerprint(),
                    );
                }
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
        if !envelope.belongs_to(&this_machine) {
            theirs.push(listing.name.clone());
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

        // Two items wanting the same file is not a race to settle by writing
        // both: an unstamped `ssh:id_ed25519` left in the store and this
        // machine's own landed on the same path, one after the other, and
        // which key survived was decided by their order.
        if let Some(first) = claimed.get(dest.as_path()) {
            progress.clear();
            println!(
                "  ! {:<28} {} is already being written by {first}",
                listing.name,
                pretty(dest, &home)
            );
            contested.push((listing.name.clone(), first.clone(), pretty(dest, &home)));
            continue;
        }
        claimed.insert(dest.clone(), listing.name.clone());

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
        ledger.record(&listing.name, &envelope.fingerprint());
        written += 1;
    }

    drop(progress);
    if !dry_run {
        if let Err(e) = ledger.save(&ledger_path()) {
            println!("  could not record what was exchanged: {e}");
        }
    }
    println!();
    if dry_run {
        println!("  {written} to write, {same} already here. Nothing was written.");
    } else {
        println!("  {written} written, {same} already here.");
    }
    if !ahead.is_empty() {
        println!(
            "  {} newer here, so not taken: {}",
            ahead.len(),
            ahead.join(", ")
        );
        println!("  kitbag push sends those.");
    }
    report_held(&held, "restore");
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
    if !contested.is_empty() {
        println!();
        println!(
            "  {} want a file another item is already writing:",
            contested.len()
        );
        for (name, first, path) in &contested {
            println!("  · {name} and {first} both say they belong at {path}");
        }
        println!("  Only the first was written. Remove whichever of them should not exist.");
    }
    if !theirs.is_empty() {
        println!();
        println!(
            "  {} belong to another machine and stay in the store for it:",
            theirs.len()
        );
        for name in &theirs {
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
    use std::process::Stdio;

    let mut child = kitbag_core::exec::shell(command)
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
    let base = path.as_os_str().to_os_string();

    // Seconds are not fine enough. Two writes to one path inside the same
    // second gave the same backup name, and the second rename destroyed the
    // first backup — which held the only copy of a machine's own SSH key.
    for n in 0..1000 {
        let mut name = base.clone();
        if n == 0 {
            name.push(format!(".backup.{stamp}"));
        } else {
            name.push(format!(".backup.{stamp}-{n}"));
        }
        let candidate = PathBuf::from(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    let mut name = base;
    name.push(format!(".backup.{stamp}-full"));
    PathBuf::from(name)
}

fn ledger_path() -> PathBuf {
    config_path()
        .parent()
        .unwrap_or(Path::new("."))
        .join("exchanged")
}

/// Say what was not settled, and what would settle it.
fn report_held(held: &[(String, State)], verb: &str) {
    if held.is_empty() {
        return;
    }
    println!();
    println!("  {} not settled:", held.len());
    for (name, state) in held {
        let why = match state {
            State::Conflict => "both sides moved since they last agreed",
            _ => "differs, and there is no record of what they last agreed on",
        };
        println!("  {} {:<28} {why}", state.glyph(), name);
    }
    println!();
    println!("  kitbag diff --only <item>                    what differs");
    println!("  kitbag {verb} --only <item>                     settle it this way");
    println!(
        "  kitbag {} --only <item>                  or the other",
        opposite(verb)
    );
}

fn opposite(verb: &str) -> &'static str {
    if verb == "push" {
        "restore"
    } else {
        "push"
    }
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
pub fn discover(
    write: bool,
    dismiss: Option<&str>,
    backend: Option<&str>,
    json: bool,
) -> Result<()> {
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
    let Collected { items, .. } = collect(&config, &home);
    let tracked: Vec<PathBuf> = items.iter().map(|i| i.path.clone()).collect();

    let scanned = scan(&home, &tracked, &dismissed());
    let found = scanned.found;

    // Tracked is not the same as kept. An item named in this machine's config
    // and never sent is a file somebody believes is backed up, and asking the
    // config cannot tell them otherwise — only the store can.
    let mut unkept: Vec<&kitbag_core::Item> = Vec::new();
    if let Some(name) = backend {
        let store = open_store(Some(name))?;
        let held: std::collections::BTreeSet<String> =
            store.list()?.into_iter().map(|l| l.name).collect();
        unkept = items.iter().filter(|i| !held.contains(&i.name)).collect();
    }

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

    // What is installed is state too, and the one kind a store should hold as
    // a list rather than as itself. Nothing in the catalogue can find it,
    // because it is not a file.
    let tracked_programs = config
        .tracks
        .iter()
        .any(|t| t.name.as_deref() == Some("programs"));
    if !tracked_programs {
        if let Some(manager) = kitbag_providers::PackageManager::detect() {
            println!();
            println!(
                "  {} is here and nothing records what it installed.",
                manager.name()
            );
            println!("  The list, not the programs — names, and versions where there are any:");
            println!();
            println!("    [[track]]");
            println!("    name = \"programs\"");
            println!("    scope = \"personal\"");
            println!(
                "    command = {{ export = \"kitbag programs\", restore = \"kitbag programs --restore\" }}"
            );
        }
    }

    if !unkept.is_empty() {
        println!();
        println!("  {} tracked here and not in the store:", unkept.len());
        for item in &unkept {
            println!("  ! {:<28} {}", item.name, pretty(&item.path, &home));
        }
        println!("  kitbag push --backend <name> sends them.");
    }

    if !scanned.in_git.is_empty() {
        println!();
        println!(
            "  {} already kept by a git repository, so not proposed:",
            scanned.in_git.len()
        );
        for (path, repo) in &scanned.in_git {
            println!("    {:<38} {}", pretty(path, &home), pretty(repo, &home));
        }
        println!("  Whatever keeps that repository keeps these.");
    }

    if found.is_empty() {
        println!();
        println!(
            "  Nothing new. Everything this knows to look for is either tracked, dismissed, or in a repository."
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
/// Start keeping a path. The `git add` of this tool.
///
/// Editing a config file to say "keep this" is a step nobody should have to
/// take, and the reason the config exists is that a machine needs to remember
/// the answer — not that a person should have to type it in that shape.
///
/// What it works out rather than asking: a directory becomes the pattern that
/// covers the files not in it yet; a directory holding both scripts and
/// installed binaries gets `only = "scripts"`, because storing a binary built
/// for one architecture is the thing `programs` exists to avoid; and a file
/// carrying its own `# scope:` marker keeps it, since the marker travels with
/// the file and any scope written here would be a second answer to the same
/// question.
///
/// Every guess is printed. None of them is silent.
#[allow(clippy::too_many_arguments)]
pub fn add(
    paths: &[String],
    scope: Option<&str>,
    owner: Option<&str>,
    why: Option<&str>,
    everywhere: bool,
    secret: bool,
    all_files: bool,
    per_machine: bool,
) -> Result<()> {
    let home = home();
    let cfg_path = config_path();
    if let Some(s) = scope {
        // Refused here rather than written into a config that fails quietly
        // on every run from now on.
        let _: kitbag_core::Scope = s.parse()?;
    }

    let existing = std::fs::read_to_string(&cfg_path).unwrap_or_default();
    let mut adding = String::new();
    let mut rules = String::new();
    let mut added = 0usize;

    println!();
    for given in paths {
        let full = PathBuf::from(kitbag_core::config::expand(given, &home));

        // A directory is a standing answer, not a list of today's files: the
        // pattern covers the ones that are not there yet, which is most of
        // the point of saying "keep this directory".
        let (pattern, matches) = if full.is_dir() {
            let pattern = format!("{}/*", given.trim_end_matches('/'));
            (pattern, files_in(&full))
        } else if given.contains('*') {
            (given.clone(), glob_files(&full))
        } else {
            (given.clone(), vec![full.clone()])
        };

        if existing.contains(&format!("path = \"{pattern}\""))
            || adding.contains(&format!("path = \"{pattern}\""))
        {
            println!("  = {pattern:<38} already tracked");
            continue;
        }
        if matches.is_empty() && !pattern.contains('*') {
            println!("  ! {pattern:<38} not here — tracking it anyway");
        }

        // Scripts and installed binaries share a directory more often than
        // not, and only one of the two belongs in a store.
        let scripts = matches.iter().filter(|p| is_script(p)).count();
        let filter = if all_files || scripts == 0 || scripts == matches.len() {
            None
        } else {
            Some("scripts")
        };

        let mut notes: Vec<String> = Vec::new();
        let mut entry = format!("\n[[track]]\npath = \"{pattern}\"\n");
        match scope {
            Some(s) => {
                entry.push_str(&format!("scope = \"{s}\"\n"));
                notes.push(s.to_string());
            }
            None => {
                let marked = matches.iter().filter(|p| carries_a_marker(p)).count();
                if marked == matches.len() && !matches.is_empty() {
                    notes.push("scope from each file's own marker".into());
                } else {
                    notes.push(format!(
                        "{} of {} carry no `# scope:` line and will be skipped",
                        matches.len() - marked,
                        matches.len()
                    ));
                }
            }
        }
        if let Some(o) = owner {
            entry.push_str(&format!("owner = \"{o}\"\n"));
            notes.push(o.to_string());
        }
        if per_machine {
            entry.push_str("per_machine = true\n");
            notes.push("this machine's own".into());
        }
        if let Some(f) = filter {
            entry.push_str(&format!("only = \"{f}\"\n"));
            notes.push(format!(
                "scripts only — {} of {} here are not",
                matches.len() - scripts,
                matches.len()
            ));
        }
        adding.push_str(&entry);
        added += 1;
        println!("  + {pattern:<38} {}", notes.join(" · "));

        if everywhere {
            let reason = why.unwrap_or("something you named");
            let mut rule = format!("\n[[known]]\npath = \"{}\"\n", under_home(&pattern));
            rule.push_str(&format!("why = \"{reason}\"\n"));
            rule.push_str(&format!("scope = \"{}\"\n", scope.unwrap_or("personal")));
            rule.push_str(&format!(
                "kind = \"{}\"\n",
                if secret { "secret" } else { "setup" }
            ));
            if per_machine {
                rule.push_str("per_machine = true\n");
            }
            if let Some(f) = filter {
                rule.push_str(&format!("only = \"{f}\"\n"));
            }
            rules.push_str(&rule);
        }
    }

    if added == 0 {
        println!();
        println!("  Nothing added.");
        return Ok(());
    }

    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cfg_path, format!("{existing}{adding}"))?;
    println!();
    println!("  Added to {}.", cfg_path.display());

    if !rules.is_empty() {
        let rule_path = kitbag_catalog::catalogue_path(&home);
        if let Some(parent) = rule_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let was = std::fs::read_to_string(&rule_path).unwrap_or_default();
        std::fs::write(&rule_path, format!("{was}{rules}"))?;
        println!(
            "  And to {}, so every machine looks there.",
            rule_path.display()
        );
    }

    println!("  `kitbag tracked` lists it; `kitbag push` sends it.");
    Ok(())
}

/// One level of a directory, files only.
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect()
}

fn glob_files(pattern: &Path) -> Vec<PathBuf> {
    match glob::glob(&pattern.to_string_lossy()) {
        Ok(found) => found
            .filter_map(Result::ok)
            .filter(|p| p.is_file())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Does this file begin `#!`? The same question the collector asks, and the
/// only honest way to tell a script from a binary that shares its directory.
fn is_script(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 2];
    file.read_exact(&mut head).is_ok() && &head == b"#!"
}

/// Does it say whose it is? A marker travels with the file, so a file that
/// has one needs no scope written anywhere else.
fn carries_a_marker(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .take(20)
                .any(|l| l.contains("scope:") && l.trim_start().starts_with(['#', '/', '-', ';']))
        })
        .unwrap_or(false)
}

/// A catalogue entry is relative to the home directory, because it is a rule
/// for every machine and no two of them spell a home directory the same way.
fn under_home(pattern: &str) -> String {
    pattern
        .trim_start_matches("~/")
        .trim_start_matches("./")
        .to_string()
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
    use kitbag_core::difference::describe;

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
        describe_one(item, &theirs.payload, colour);
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

/// The three roles a line in a difference can have. Colour is a second way of
/// saying what the label already says, for reading quickly — never the only
/// way, so a terminal without it loses nothing.
fn tinted(colour: Colour, code: &str, text: &str) -> String {
    match colour {
        Colour::Always => format!("\x1b[{code}m{text}\x1b[0m"),
        Colour::Never => text.to_string(),
    }
}

fn only_here(colour: Colour) -> String {
    tinted(colour, "32", "only here: ")
}
fn only_there(colour: Colour) -> String {
    tinted(colour, "36", "only there:")
}
fn differs(colour: Colour) -> String {
    tinted(colour, "33", "differ:    ")
}

/// A list, shortened. An archive can hold dozens of paths and a question has
/// one screen; the count is what matters and the first few say what kind.
fn show_some(label: &str, names: &[String]) {
    if names.is_empty() {
        return;
    }
    const SHOWN: usize = 6;
    let head = names
        .iter()
        .take(SHOWN)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    if names.len() > SHOWN {
        println!("      {label} {head} … and {} more", names.len() - SHOWN);
    } else {
        println!("      {label} {head}");
    }
}

/// What an answer at the prompt means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Mine,
    Theirs,
    Skip,
    Quit,
}

/// Read an answer. Anything not understood is a skip, and so is an empty
/// line: the key that is easiest to hit by accident must be the one that
/// writes nothing over anything.
pub fn choice(answer: &str) -> Choice {
    match answer.trim().to_ascii_lowercase().as_str() {
        "m" | "mine" => Choice::Mine,
        "t" | "theirs" => Choice::Theirs,
        "q" | "quit" => Choice::Quit,
        _ => Choice::Skip,
    }
}

/// What `diff` prints for one item, so `resolve` can show the same thing
/// without a second round trip to the store.
fn describe_one(item: &kitbag_core::Item, theirs: &[u8], colour: Colour) {
    use kitbag_core::difference::{describe, Difference};

    match describe(&item.payload, theirs) {
        Difference::Keys {
            only_here,
            only_there,
            differing,
            here_lines,
            there_lines,
        } => {
            println!("      here   {here_lines} lines");
            println!("      store  {there_lines} lines");
            show_some(&self::only_here(colour), &only_here);
            show_some(&self::only_there(colour), &only_there);
            show_some(&differs(colour), &differing);
            if only_here.is_empty() && only_there.is_empty() && differing.is_empty() {
                // Every setting agrees and the bytes do not: a comment, an
                // ordering, a blank line. Saying nothing here left a person
                // staring at two line counts.
                println!("      every setting agrees — the difference is elsewhere in the file");
            }
        }
        Difference::Archive {
            only_here,
            only_there,
            differing,
            here_count,
            there_count,
        } => {
            println!("      here   {here_count} files");
            println!("      store  {there_count} files");
            show_some(&self::only_here(colour), &only_here);
            show_some(&self::only_there(colour), &only_there);
            show_some(&differs(colour), &differing);
        }
        Difference::Text {
            here_lines,
            there_lines,
            directives,
        } => {
            println!("      here   {here_lines} lines");
            println!("      store  {there_lines} lines");
            if directives.is_empty() {
                println!("      the same kinds of line — the difference is in what they say");
            }
            for (name, delta) in directives {
                // The operand is what a person wrote; the directive is what
                // kind of thing it is, and only the second is said here.
                let (label, code) = if delta > 0 {
                    (format!("{delta} more"), "36")
                } else {
                    (format!("{} fewer", -delta), "32")
                };
                println!(
                    "      {:<12} {} in the store",
                    tinted(colour, code, &name),
                    label
                );
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
        Difference::None => println!("      the same contents under different markers"),
    }
}

/// Settle the differences neither side can settle alone, one at a time.
///
/// Everything needed was already here — `diff` says what differs and `--only`
/// acts on one item — but only as three commands typed per conflict, with the
/// names copied between them. This asks.
///
/// Nothing is decided for the person: every item is shown and every answer is
/// theirs, and the answer that needs no thought — Enter — is the one that does
/// nothing.
pub fn resolve(backend: Option<&str>, wanted: Option<Wanted>, colour: Colour) -> Result<()> {
    use std::io::{IsTerminal, Write};

    let home = home();
    let config = Config::load_or_default(&config_path())?.with_env_skips();
    let wanted = wanted.unwrap_or_else(|| config.wanted());
    let Collected { items, .. } = collect(&config, &home);
    let mut ledger = kitbag_core::ledger::Ledger::load(&ledger_path());

    let progress = ui::Progress::new(colour);
    progress.say("reading what the store holds");
    let store = open_store(backend)?;
    let remote: Remote = store
        .list()?
        .into_iter()
        .map(|l| (l.name, l.fingerprint))
        .collect();
    // Ended, not just cleared: what follows is a conversation, and a spinner
    // redrawing itself underneath a question is not a good one.
    drop(progress);

    let unsettled: Vec<&kitbag_core::Item> = items
        .iter()
        .filter(|i| wanted.accepts(&i.scope) && !config.skips(&i.name))
        .filter(|i| !kitbag_core::state::compare_against(i, &remote, &home, &ledger).is_decided())
        .collect();

    println!();
    if unsettled.is_empty() {
        println!("  Nothing to settle.");
        return Ok(());
    }

    if !std::io::stdin().is_terminal() {
        println!(
            "  {} to settle, and no terminal to ask in:",
            unsettled.len()
        );
        for item in &unsettled {
            println!("  · {}", item.name);
        }
        println!();
        println!("  kitbag push --only <item>      settle it this way");
        println!("  kitbag restore --only <item>   or the other");
        return Ok(());
    }

    let total = unsettled.len();
    let mut settled = 0usize;
    let mut unreadable: Vec<(String, String)> = Vec::new();
    for (at, item) in unsettled.iter().enumerate() {
        // One item the store will not hand over must not end the conversation
        // about the other three. The client crashed on one of these, and the
        // whole run stopped.
        let theirs = match store.get(&item.name) {
            Ok(envelope) => envelope,
            Err(e) => {
                println!("  ! {}  ({}/{total})", item.name, at + 1);
                println!("      could not be read: {}", root_cause(&e));
                println!("      left alone\n");
                unreadable.push((item.name.clone(), root_cause(&e)));
                continue;
            }
        };

        println!("  ! {}  ({}/{total})", item.name, at + 1);
        describe_one(item, &theirs.payload, colour);

        print!("      [m]ine  [t]heirs  [s]kip  [q]uit > ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;

        match choice(&answer) {
            Choice::Mine => {
                let envelope = kitbag_core::collect::envelope_for(item, &home);
                // One item the store will not take must not end the
                // conversation about the rest: another machine pushing at the
                // same moment makes the store refuse this write, and that is
                // a thing to say and carry on from.
                match store.put(&item.name, &envelope) {
                    Ok(()) => {
                        ledger.record(&item.name, &envelope.fingerprint());
                        println!("      sent this machine's\n");
                        settled += 1;
                    }
                    Err(e) => {
                        println!("      not sent: {}\n", explain_failure(&e));
                        unreadable.push((item.name.clone(), explain_failure(&e)));
                    }
                }
            }
            Choice::Theirs => {
                match theirs.destination(&home) {
                    Some(dest) => {
                        let kept = place(&dest, &theirs.payload)?;
                        match kept {
                            Some(backup) => println!(
                                "      took the store's — the old one is at {}\n",
                                pretty(&backup, &home)
                            ),
                            None => println!("      took the store's\n"),
                        }
                    }
                    None => match &item.source {
                        kitbag_core::collect::Source::Command { restore } => {
                            pipe_into(restore, &theirs.payload)?;
                            println!("      took the store's, into `{restore}`\n");
                        }
                        kitbag_core::collect::Source::File(_) => {
                            println!("      nowhere to put it; left alone\n");
                            continue;
                        }
                    },
                }
                ledger.record(&item.name, &theirs.fingerprint());
                settled += 1;
            }
            Choice::Quit => {
                println!("      stopped\n");
                break;
            }
            Choice::Skip => println!("      left alone\n"),
        }
    }

    if let Err(e) = ledger.save(&ledger_path()) {
        println!("  could not record what was exchanged: {e}");
    }
    println!("  {settled} settled, {} left.", total - settled);
    if !unreadable.is_empty() {
        println!();
        println!("  {} did not settle:", unreadable.len());
        for (name, why) in &unreadable {
            println!("  · {name}: {why}");
        }
    }
    Ok(())
}

/// Write down what is installed here, or put it back.
///
/// A store should never hold a binary: it is large, it is built for one
/// architecture, and whoever published it will hand it over again. What is
/// worth keeping is the list — what was installed, by which manager, at which
/// version where one can be asked for.
///
/// Putting it back installs what is missing and removes nothing. A machine is
/// allowed to have more than the list; the list is what it must not lack.
pub fn programs(
    backend: Option<&str>,
    from: Option<&str>,
    list: bool,
    restore: bool,
    dry_run: bool,
    colour: Colour,
) -> Result<()> {
    use kitbag_providers::programs;

    if list {
        let store = open_store(backend)?;
        let mut names: Vec<String> = store
            .list()?
            .into_iter()
            .map(|l| l.name)
            .filter(|n| n == "programs" || n.starts_with("programs@"))
            .collect();
        names.sort();
        println!();
        if names.is_empty() {
            println!("  No machine has written its list yet.");
            println!("  kitbag push sends this one's.");
            return Ok(());
        }
        for name in &names {
            let whose = name.strip_prefix("programs@").unwrap_or("this machine");
            println!("  {whose:<28} {name}");
        }
        println!();
        println!(
            "  {} list(s). kitbag programs --from <machine> reads one.",
            names.len()
        );
        return Ok(());
    }

    // Where the list comes from: another machine's, standard input, or this
    // machine itself. Only the last one is a question about the machine; the
    // other two are a question about the store.
    let text = match (from, restore) {
        (Some(whose), _) => {
            let store = open_store(backend)?;
            let name = if whose.contains('@') || whose == "programs" {
                whose.to_string()
            } else {
                format!("programs@{whose}")
            };
            let envelope = store.get(&name).with_context(|| {
                format!("{name}: no such list — `kitbag programs --list` says which exist")
            })?;
            String::from_utf8_lossy(&envelope.payload).to_string()
        }
        (None, true) => {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
            text
        }
        (None, false) => {
            print!("{}", programs::write(&here_and_declared()?));
            return Ok(());
        }
    };

    if !restore {
        print!("{text}");
        return Ok(());
    }

    let wanted = programs::read(&text);
    if wanted.is_empty() {
        println!();
        println!("  Nothing listed.");
        return Ok(());
    }

    let mine = here_and_declared()?;
    let here: std::collections::BTreeSet<(String, String)> = mine
        .iter()
        .map(|p| (p.manager.clone(), p.name.clone()))
        .collect();

    let progress = ui::Progress::new(colour);
    let mut installed = 0usize;
    let mut already = 0usize;
    let mut unknown: Vec<String> = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();

    println!();
    for p in &wanted {
        if here.contains(&(p.manager.clone(), p.name.clone())) || already_here(p) {
            already += 1;
            continue;
        }
        let Some(argv) = programs::install_command(p) else {
            unknown.push(format!("{}:{}", p.manager, p.name));
            continue;
        };
        // A declared program's install line came out of the store, and a line
        // out of the store is named before it runs — the same rule the restore
        // commands follow. The rest are argv this tool built itself.
        let shown = if p.manager == kitbag_providers::programs::SCRIPT {
            format!("`{}`  (from the list)", p.install)
        } else {
            argv.join(" ")
        };
        if dry_run {
            progress.clear();
            println!("  + {:<28} {shown}", p.name);
            installed += 1;
            continue;
        }
        if p.manager == kitbag_providers::programs::SCRIPT {
            progress.clear();
            println!("  + {:<28} {shown}", p.name);
        }
        progress.say(format!("installing {}", p.name));
        let out = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .output();
        progress.clear();
        match out {
            Ok(o) if o.status.success() => {
                println!("  + {:<28} {}", p.name, p.manager);
                installed += 1;
            }
            Ok(o) => {
                let why = String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("no message")
                    .trim()
                    .to_string();
                println!("  ✗ {:<28} {why}", p.name);
                failed.push((p.name.clone(), why));
            }
            Err(e) => {
                println!("  ✗ {:<28} {e}", p.name);
                failed.push((p.name.clone(), e.to_string()));
            }
        }
    }

    drop(progress);
    println!();
    if dry_run {
        println!("  {installed} to install, {already} already here. Nothing was run.");
    } else {
        println!("  {installed} installed, {already} already here.");
    }
    if !unknown.is_empty() {
        println!(
            "  {} from a manager this does not drive: {}",
            unknown.len(),
            unknown.join(", ")
        );
    }
    if !failed.is_empty() {
        println!("  {} did not install:", failed.len());
        for (name, why) in &failed {
            println!("    {name}: {why}");
        }
        bail!("{} program(s) did not install", failed.len());
    }
    Ok(())
}

/// What the managers admit to, plus what this machine declared and actually
/// has. A declaration that has never been acted on is a plan, not a fact, so
/// it stays out of a list of what is installed.
fn here_and_declared() -> Result<Vec<kitbag_providers::programs::Program>> {
    use kitbag_providers::programs;
    let config = Config::load_or_default(&config_path())?;
    let mut all = programs::installed();
    for d in &config.programs {
        if let Some(p) = programs::declared(
            &d.name,
            &d.install,
            d.version_from.as_deref(),
            d.present.as_deref(),
        ) {
            all.push(p);
        }
    }
    all.sort();
    all.dedup();
    Ok(all)
}

/// Is a program from somebody else's list already here? Asked only of the
/// declared ones: a manager's own entries are answered by asking the manager,
/// and these have no manager to ask. The test travels with the item, so a
/// program whose command is not its name is not reinstalled every restore.
fn already_here(p: &kitbag_providers::programs::Program) -> bool {
    p.manager == kitbag_providers::programs::SCRIPT && kitbag_providers::programs::is_here(p)
}

/// One command, from a machine that has never done this to a machine whose
/// state is in a store.
///
/// The pieces all existed — `discover`, `track`, `push`, `resolve` — and
/// needing four of them in the right order, one of which lives in another
/// repository, is a way of saying "this is for people who already know how it
/// works". A backup is one question with several parts, so it is asked as one.
///
/// Nothing here is new behaviour. Every step is the command of the same name,
/// which is deliberate: a walkthrough that did its own thing would be a second
/// implementation to keep honest.
pub fn backup(
    backend: Option<&str>,
    wanted: Option<Wanted>,
    jobs: usize,
    colour: Colour,
) -> Result<()> {
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() {
        bail!(
            "kitbag backup asks questions, and there is no terminal to ask in.\n\
             The steps on their own: kitbag discover --write, then kitbag push"
        );
    }

    let home = home();
    let cfg_path = config_path();

    println!();
    println!("  Backing up what this machine holds that is yours.");
    println!("  Nothing is sent until the last step, and it says what first.");

    // 1 — where it goes.
    let chosen = match backend {
        Some(name) => name.to_string(),
        None => {
            let known = "bw, op, pass, age";
            let answer = ask(&format!("\n  1/4  Which store? ({known})\n       [bw] "))?;
            if answer.is_empty() {
                "bw".to_string()
            } else {
                answer
            }
        }
    };
    // Asked for now rather than at the end: a name that is not a backend
    // should be a question, not four minutes of work and then a question.
    let _: BackendKind = chosen.parse()?;

    // 2 — what this machine is willing to hold. Only asked of a machine that
    // has never said, because changing it later is a decision with weight.
    if !cfg_path.exists() {
        println!();
        println!("  2/4  This machine has no config yet.");
        println!("       Scope says whose an item is: personal, work, shared.");
        println!("       A machine takes the ones it names and ignores the rest.");
        // Parsed before it is written, so a typo is caught here rather than by
        // every command from now on — and asked again rather than fatal.
        let scopes = loop {
            let answer = ask("\n       Which scopes does this machine take? [personal] ")?;
            let answer = if answer.is_empty() {
                "personal".to_string()
            } else {
                answer
            };
            match Wanted::parse(&answer) {
                Ok(_) => break answer,
                Err(e) => println!("       {e}"),
            }
        };
        let list: Vec<String> = scopes
            .split(',')
            .map(|s| format!("\"{}\"", s.trim()))
            .filter(|s| s != "\"\"")
            .collect();
        if let Some(parent) = cfg_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&cfg_path, format!("scopes = [{}]\n", list.join(", ")))?;
        println!("       Wrote {}.", cfg_path.display());
    } else {
        println!();
        println!("  2/4  Using {}.", cfg_path.display());
    }

    // 3 — what is here that nothing keeps. One at a time, because "take all
    // of these" is not an answer anybody can give about their own home
    // directory without looking.
    let config = Config::load_or_default(&cfg_path)?.with_env_skips();
    let Collected { items, .. } = collect(&config, &home);
    let tracked: Vec<PathBuf> = items.iter().map(|i| i.path.clone()).collect();
    let scanned = scan(&home, &tracked, &dismissed());
    let found = scanned.found;

    if !scanned.in_git.is_empty() {
        println!();
        println!(
            "       {} already kept by a git repository, so not offered:",
            scanned.in_git.len()
        );
        for (path, repo) in &scanned.in_git {
            println!(
                "         {:<38} {}",
                pretty(path, &home),
                pretty(repo, &home)
            );
        }
    }

    println!();
    if found.is_empty() {
        println!("  3/4  Nothing untracked that this knows to look for.");
    } else {
        // The whole list, then one answer. Asking item by item splits a
        // decision a person makes by looking down a list into as many
        // decisions as there are items, and makes them answer for the first
        // one without knowing what the fifth is.
        // Ordered the way it is about to be printed, so the numbers run down
        // the page instead of jumping between groups. The number is what
        // somebody types; it has to be where their eye already is.
        let mut left: Vec<Finding> = found.clone();
        left.sort_by(|a, b| {
            let rank = |k: kitbag_catalog::Kind| match k {
                kitbag_catalog::Kind::Secret => 0,
                kitbag_catalog::Kind::Setup => 1,
            };
            rank(a.kind)
                .cmp(&rank(b.kind))
                .then_with(|| a.shown(&home).cmp(&b.shown(&home)))
        });
        let take: Vec<Finding> = loop {
            println!("  3/4  {} thing(s) here that nothing keeps.", left.len());
            // Grouped, because forty lines of paths is a list somebody scrolls
            // past and two short lists are two they read. Numbered across both,
            // since the number is what they type.
            for kind in [kitbag_catalog::Kind::Secret, kitbag_catalog::Kind::Setup] {
                if !left.iter().any(|f| f.kind == kind) {
                    continue;
                }
                println!();
                println!("  {}", kind.heading());
                for (n, f) in left.iter().enumerate() {
                    if f.kind != kind {
                        continue;
                    }
                    println!("    {:>2}  {:<38} {}", n + 1, f.shown(&home), f.why);
                    if f.scope == "auto" {
                        println!("        needs its own `# scope:` line, or it is skipped");
                    } else {
                        println!("        proposed as {}", f.scope);
                    }
                }
            }
            println!();
            println!("       [all] · `none` · numbers like `1 3 5` or `2-4`");
            println!("       `d 2` dismisses one for good, and asks again");
            match parse_pick(&ask("       > ")?, left.len()) {
                Picked::All => break left.clone(),
                Picked::None => break Vec::new(),
                Picked::Some(idx) => break idx.iter().map(|i| left[*i].clone()).collect(),
                Picked::Dismiss(idx) => {
                    for i in &idx {
                        dismiss_path(&left[*i].path)?;
                        println!("       dismissed {}", left[*i].shown(&home));
                    }
                    let mut keep: Vec<Finding> = Vec::new();
                    for (n, f) in left.into_iter().enumerate() {
                        if !idx.contains(&n) {
                            keep.push(f);
                        }
                    }
                    left = keep;
                    println!();
                    if left.is_empty() {
                        break Vec::new();
                    }
                }
                Picked::Unclear(why) => {
                    println!("       {why}");
                    println!();
                }
            }
        };
        if take.is_empty() {
            println!("       Nothing added.");
        } else {
            append_tracks(&cfg_path, &take, &home)?;
            println!("       Added {} to {}.", take.len(), cfg_path.display());
        }
    }

    // The one thing no scan can find, because it is not a file.
    let config = Config::load_or_default(&cfg_path)?.with_env_skips();
    let records_programs = config
        .tracks
        .iter()
        .any(|t| t.name.as_deref() == Some("programs"));
    if !records_programs {
        if let Some(manager) = PackageManager::detect() {
            println!();
            println!(
                "       {} is here and nothing records what it installed.",
                manager.name()
            );
            println!("       The list, not the programs — a kilobyte, not gigabytes.");
            if ask("       Record it? [Y/n] ")?.to_lowercase() != "n" {
                let mut current = std::fs::read_to_string(&cfg_path).unwrap_or_default();
                current.push_str(
                    "\n[[track]]\nname = \"programs\"\nscope = \"personal\"\nper_machine = true\n\
                     command = { export = \"kitbag programs\", restore = \"kitbag programs --restore\" }\n",
                );
                std::fs::write(&cfg_path, current)?;
                println!("       Added.");
            }
        }
    }

    // 4 — what would be sent, then sending it. `push --dry-run` is the plan,
    // and it is the same code that does the work a moment later.
    println!();
    println!("  4/4  What would be sent:");
    push(Some(&chosen), wanted.clone(), true, &[], jobs, colour)?;

    if ask("\n  Send these? [y/N] ")?.to_lowercase() != "y" {
        println!("  Nothing was sent.");
        return Ok(());
    }

    let held = push(Some(&chosen), wanted.clone(), false, &[], jobs, colour)?;

    if held > 0 {
        println!();
        println!("  {held} item(s) moved on both sides and were left alone.");
        if ask("  Settle them now, one at a time? [y/N] ")?.to_lowercase() == "y" {
            resolve(Some(&chosen), wanted, colour)?;
        } else {
            println!("  kitbag resolve, when you are ready.");
        }
    }

    println!();
    println!("  Done. `kitbag status --backend {chosen}` says where things stand.");
    Ok(())
}

/// What somebody typed at a numbered list.
#[derive(Debug, PartialEq, Eq)]
enum Picked {
    All,
    None,
    Some(Vec<usize>),
    Dismiss(Vec<usize>),
    /// Not understood, and why. Never a silent "nothing", because at this
    /// prompt "nothing" and "I mistyped" look identical afterwards.
    Unclear(String),
}

/// Read a selection against a list of `count` items, as zero-based indices.
///
/// Empty means everything: the list was just read, and the common answer at
/// the end of reading it is yes. Saying no takes a word, which is the right
/// way round — nobody types `none` by accident.
fn parse_pick(answer: &str, count: usize) -> Picked {
    let answer = answer.trim().to_lowercase();
    if answer.is_empty() || answer == "all" || answer == "a" {
        return Picked::All;
    }
    if answer == "none" || answer == "n" {
        return Picked::None;
    }

    let (dismissing, rest) = match answer.strip_prefix('d') {
        Some(rest) => (true, rest.trim().to_string()),
        None => (false, answer.clone()),
    };
    if dismissing && rest.is_empty() {
        return Picked::Unclear("`d` needs a number: `d 2`, or `d 2 4`.".into());
    }

    let mut picked: Vec<usize> = Vec::new();
    for token in rest.split(|c: char| c == ',' || c.is_whitespace()) {
        if token.is_empty() {
            continue;
        }
        // `2-4` is three answers written the way people write three answers.
        let bounds: Vec<&str> = token.splitn(2, '-').collect();
        let range = match bounds.as_slice() {
            [one] => one.parse::<usize>().map(|n| (n, n)),
            [from, to] => match (from.parse::<usize>(), to.parse::<usize>()) {
                (Ok(a), Ok(b)) => Ok((a, b)),
                _ => return Picked::Unclear(format!("`{token}` is not a number or a range.")),
            },
            _ => return Picked::Unclear(format!("`{token}` is not a number or a range.")),
        };
        let Ok((from, to)) = range else {
            return Picked::Unclear(format!("`{token}` is not a number or a range."));
        };
        if from == 0 || to == 0 || from > count || to > count {
            return Picked::Unclear(format!("there is no {token} — the list has {count}."));
        }
        if from > to {
            return Picked::Unclear(format!("`{token}` counts backwards."));
        }
        for n in from..=to {
            if !picked.contains(&(n - 1)) {
                picked.push(n - 1);
            }
        }
    }
    if picked.is_empty() {
        return Picked::Unclear("that picked nothing — `none` if that is what you meant.".into());
    }
    picked.sort_unstable();
    if dismissing {
        Picked::Dismiss(picked)
    } else {
        Picked::Some(picked)
    }
}

/// What `discover` looks for, and where to add to it.
///
/// The built-in list is the places that are the same on most machines. It
/// cannot know where somebody keeps their work, so the interesting half of
/// this command is the last paragraph: the file to write, and the shape of a
/// line in it.
pub fn catalogue(json: bool) -> Result<()> {
    use kitbag_catalog::{catalogue_path, Kind, Origin};

    let home = home();
    let (all, trouble) = kitbag_catalog::catalogue(&home);
    let path = catalogue_path(&home);

    if json {
        let out: Vec<_> = all
            .iter()
            .map(|k| {
                serde_json::json!({
                    "path": k.path,
                    "scope": k.scope,
                    "why": k.why,
                    "kind": match k.kind { Kind::Secret => "secret", Kind::Setup => "setup" },
                    "only": k.only,
                    "per_machine": k.per_machine,
                    "yours": k.origin == Origin::Yours,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if let Some(why) = &trouble {
        println!();
        println!("  ! {why}");
    }

    for (kind, origin) in [
        (Kind::Secret, Origin::BuiltIn),
        (Kind::Setup, Origin::BuiltIn),
        (Kind::Secret, Origin::Yours),
        (Kind::Setup, Origin::Yours),
    ] {
        let rows: Vec<&kitbag_catalog::Known> = all
            .iter()
            .filter(|k| k.kind == kind && k.origin == origin)
            .collect();
        if rows.is_empty() {
            continue;
        }
        println!();
        println!(
            "  {} · {}",
            kind.heading(),
            match origin {
                Origin::BuiltIn => "built in",
                Origin::Yours => "yours",
            }
        );
        for k in rows {
            let mut notes: Vec<String> = vec![k.scope.clone()];
            if let Some(only) = &k.only {
                notes.push(format!("only {only}"));
            }
            if k.per_machine {
                notes.push("per machine".into());
            }
            println!("    ~/{:<36} {:<22} {}", k.path, notes.join(", "), k.why);
        }
    }

    println!();
    println!("  {} place(s) looked for. Add your own in", all.len());
    println!("  {}:", path.display());
    println!();
    println!("    [[known]]");
    println!("    path = \"work/deploy/*\"          # under your home");
    println!("    why  = \"deploy scripts\"");
    println!("    scope = \"work\"                  # personal by default");
    println!("    kind  = \"setup\"                 # or \"secret\"; setup by default");
    println!("    only  = \"scripts\"               # optional: files beginning `#!`");
    println!();
    println!("  A path already listed above replaces that entry rather than");
    println!("  adding a second one, which is how a place kitbag guessed wrong");
    println!("  about gets corrected.");
    Ok(())
}

/// What this machine has been told to keep, as it was told.
///
/// `status` shows the items a track produced, which is the right answer to a
/// different question. Somebody who typed `kitbag add ~/work/deploy` and got
/// back two file names has no way to see the line they wrote, or to notice
/// that a third file in that directory was filtered out — and a filter nobody
/// can see is one nobody can correct.
pub fn tracked(json: bool) -> Result<()> {
    let home = home();
    let cfg_path = config_path();
    let config = Config::load_or_default(&cfg_path)?.with_env_skips();

    if config.tracks.is_empty() {
        println!();
        println!("  Nothing tracked yet. `kitbag add <path>` starts.");
        println!("  `kitbag discover` says what is here that nothing keeps.");
        return Ok(());
    }

    let mut rows: Vec<(String, String, String)> = Vec::new();
    for track in &config.tracks {
        let what = match (&track.path, &track.command) {
            (Some(path), _) => path.clone(),
            (None, Some(_)) => track
                .name
                .clone()
                .unwrap_or_else(|| "(a command with no name)".into()),
            (None, None) => "(neither a path nor a command)".into(),
        };

        let mut notes: Vec<String> = Vec::new();
        match &track.scope {
            Some(s) => notes.push(s.clone()),
            None if track.command.is_some() => {}
            None => notes.push("scope from each file's own marker".into()),
        }
        if let Some(owner) = &track.owner {
            notes.push(owner.clone());
        }
        if track.per_machine {
            notes.push("this machine's own".into());
        }
        if track.volatile {
            notes.push("not comparable".into());
        }
        if !track.platform.is_empty() {
            notes.push(track.platform.join("/"));
        }
        if let Some(only) = &track.only {
            notes.push(format!("{only} only"));
        }

        let holds = match &track.path {
            None => "a command".to_string(),
            Some(pattern) => {
                let expanded = kitbag_core::config::expand(pattern, &home);
                let all = if expanded.contains('*') || expanded.contains('?') {
                    glob_files(Path::new(&expanded))
                } else if Path::new(&expanded).is_file() {
                    vec![PathBuf::from(&expanded)]
                } else {
                    Vec::new()
                };
                let kept = match track.only.as_deref() {
                    Some("scripts") => all.iter().filter(|p| is_script(p)).count(),
                    Some(_) => 0,
                    None => all.len(),
                };
                // The left-out count is the whole reason this command exists:
                // it is the only place a filter's effect is visible.
                match (all.len(), all.len() - kept) {
                    (0, _) => "nothing here".to_string(),
                    (n, 0) => format!("{n} file(s)"),
                    (n, out) => format!("{n} file(s), {out} filtered out"),
                }
            }
        };
        rows.push((what, notes.join(" · "), holds));
    }

    if json {
        let out: Vec<_> = rows
            .iter()
            .map(|(what, notes, holds)| {
                serde_json::json!({ "track": what, "notes": notes, "holds": holds })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!();
    println!("  from {}", cfg_path.display());
    println!();
    // Measured, not guessed: one keychain path under ~/Library is longer than
    // any fixed column, and a row that overflows takes the whole table's
    // alignment with it.
    let first = rows
        .iter()
        .map(|(w, _, _)| w.chars().count())
        .max()
        .unwrap_or(0);
    let second = rows
        .iter()
        .map(|(_, n, _)| n.chars().count())
        .max()
        .unwrap_or(0);
    for (what, notes, holds) in &rows {
        println!("    {what:<first$}  {notes:<second$}  {holds}");
    }
    println!();
    println!("  {} track(s).", rows.len());
    if !config.skip.is_empty() {
        println!(
            "  Kept on this machine in both directions: {}",
            config.skip.join(", ")
        );
    }
    println!("  `kitbag status` shows the items these come to.");
    Ok(())
}

/// One line from the person, with the question left on screen.
fn ask(question: &str) -> Result<String> {
    use std::io::Write;
    print!("{question}");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().to_string())
}

/// Never propose this path again.
fn dismiss_path(path: &Path) -> Result<()> {
    let file = dismissed_path();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut list = std::fs::read_to_string(&file).unwrap_or_default();
    list.push_str(&format!("{}\n", path.display()));
    std::fs::write(&file, list)?;
    Ok(())
}

#[cfg(test)]
mod tests {

    use super::{parse_pick, Picked};

    #[test]
    fn an_empty_answer_at_a_list_just_read_means_all_of_it() {
        assert_eq!(parse_pick("", 3), Picked::All);
        assert_eq!(parse_pick("  ", 3), Picked::All);
        assert_eq!(parse_pick("all", 3), Picked::All);
        // and saying no takes a word, which is the right way round
        assert_eq!(parse_pick("none", 3), Picked::None);
    }

    #[test]
    fn numbers_come_back_as_positions_in_the_list() {
        assert_eq!(parse_pick("1 3", 3), Picked::Some(vec![0, 2]));
        assert_eq!(parse_pick("1,3", 3), Picked::Some(vec![0, 2]));
        assert_eq!(parse_pick("3 1", 3), Picked::Some(vec![0, 2]));
        // a range is three answers written the way people write three answers
        assert_eq!(parse_pick("2-4", 5), Picked::Some(vec![1, 2, 3]));
        // and saying the same one twice is not two of it
        assert_eq!(parse_pick("2 2-3", 5), Picked::Some(vec![1, 2]));
    }

    #[test]
    fn a_number_that_is_not_in_the_list_is_a_question_not_a_selection() {
        // Silently taking the ones that did exist is how somebody ends up
        // believing they backed up something they did not.
        assert!(matches!(parse_pick("1 9", 3), Picked::Unclear(_)));
        assert!(matches!(parse_pick("0", 3), Picked::Unclear(_)));
        assert!(matches!(parse_pick("two", 3), Picked::Unclear(_)));
        assert!(matches!(parse_pick("3-1", 5), Picked::Unclear(_)));
    }

    #[test]
    fn dismissing_says_which_and_never_means_all_of_them() {
        assert_eq!(parse_pick("d 2", 3), Picked::Dismiss(vec![1]));
        assert_eq!(parse_pick("d 1 3", 3), Picked::Dismiss(vec![0, 2]));
        // `d` on its own would be "dismiss everything" read generously, and
        // generous is the wrong way to read a permanent refusal.
        assert!(matches!(parse_pick("d", 3), Picked::Unclear(_)));
        assert!(matches!(parse_pick("d 9", 3), Picked::Unclear(_)));
    }

    #[test]
    fn an_answer_is_read_generously_but_a_blank_one_does_nothing() {
        use super::{choice, Choice};
        for yes in ["m", "M", "mine", " mine ", "MINE"] {
            assert_eq!(choice(yes), Choice::Mine, "{yes:?}");
        }
        for theirs in ["t", "T", "theirs", " theirs\n"] {
            assert_eq!(choice(theirs), Choice::Theirs, "{theirs:?}");
        }
        assert_eq!(choice("q"), Choice::Quit);
        assert_eq!(choice("quit"), Choice::Quit);
    }

    #[test]
    fn anything_not_understood_writes_nothing_over_anything() {
        // The key easiest to hit by accident is Enter, and one of the two
        // real answers overwrites a machine's copy of a credential. So the
        // accident has to be the one that does nothing.
        for unclear in ["", "\n", " ", "y", "yes", "mnie", "both", "?"] {
            assert_eq!(choice(unclear), Choice::Skip, "{unclear:?}");
        }
    }

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
