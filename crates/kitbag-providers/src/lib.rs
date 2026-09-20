//! Turning a recipe into "what would change".
//!
//! Every provider answers one question — *what is the current state of this
//! resource* — and `plan` is nothing but that answer, collected. A provider
//! that cannot answer it reports [`Action::Unknown`] with a reason rather than
//! guessing, because a plan that quietly invents a state is worse than a plan
//! that admits a gap.
//!
//! Nothing in here writes. `apply` is a separate step that takes a plan, and a
//! plan it could not compute is not one it may execute.

use std::path::Path;
use std::process::Command as Proc;

use kitbag_core::recipe::{Command, DefaultsKey, Link, Recipe};

/// What applying this resource would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Already so.
    None,
    Create(String),
    /// Replacing something that is there and different. The string says what
    /// is being replaced, because that is the part a person wants to see
    /// before saying yes.
    Replace(String),
    Unknown(String),
}

impl Action {
    pub fn glyph(&self) -> char {
        match self {
            Action::None => '=',
            Action::Create(_) => '+',
            Action::Replace(_) => '~',
            Action::Unknown(_) => '?',
        }
    }

    pub fn says(&self) -> &str {
        match self {
            Action::None => "already so",
            Action::Create(s) | Action::Replace(s) | Action::Unknown(s) => s,
        }
    }

    pub fn is_change(&self) -> bool {
        !matches!(self, Action::None)
    }
}

/// One line of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub recipe: String,
    pub id: String,
    pub action: Action,
}

/// What a link should be, checked against what is there.
pub fn plan_link(link: &Link, home: &Path, repo: &Path) -> Action {
    let (from, to) = link.expanded(home, repo);

    match std::fs::symlink_metadata(&to) {
        Err(_) => Action::Create(format!("link → {}", show_source(&from, home, repo))),
        Ok(meta) if meta.file_type().is_symlink() => match std::fs::read_link(&to) {
            Ok(target) if target == from => Action::None,
            Ok(target) => {
                Action::Replace(format!("points at {}", show_source(&target, home, repo)))
            }
            Err(e) => Action::Unknown(format!("could not read the link: {e}")),
        },
        Ok(meta) if meta.is_dir() => Action::Unknown(format!("{} is a directory", show(&to, home))),
        // A real file in the way is the case that eats people's configs, so it
        // is called out as a replacement rather than folded into "create".
        Ok(_) => Action::Replace("a file is there already".into()),
    }
}

/// Packages are checked through the manager that owns them. With no manager on
/// the machine there is no honest answer, and saying "would install" would be
/// a guess.
pub fn plan_pkg(name: &str, manager: Option<&PackageManager>) -> Action {
    match manager {
        None => Action::Unknown("no package manager on this machine".into()),
        Some(m) => {
            if m.installed(name) {
                Action::None
            } else {
                Action::Create(format!("install with {}", m.name()))
            }
        }
    }
}

/// A macOS default, compared against what `defaults read` says now.
pub fn plan_defaults(key: &DefaultsKey) -> Action {
    let want = value_string(&key.value);
    match read_default(&key.domain, &key.key) {
        None => Action::Create(format!("{} {} = {want}", key.domain, key.key)),
        Some(now) if same_value(&now, &want, &key.kind) => Action::None,
        Some(now) => Action::Replace(format!("{} {} is {now}, want {want}", key.domain, key.key)),
    }
}

/// The escape hatch. Its `check` is what makes it plannable at all.
pub fn plan_command(cmd: &Command) -> Action {
    match run(&cmd.check) {
        Ok(true) => Action::None,
        Ok(false) => Action::Create(format!("run: {}", first_words(&cmd.run))),
        Err(e) => Action::Unknown(format!("the check could not run: {e}")),
    }
}

/// Everything a recipe would change, in the order it would be done.
pub fn plan_recipe(
    recipe: &Recipe,
    home: &Path,
    repo: &Path,
    manager: Option<&PackageManager>,
    with_defaults: bool,
) -> Vec<Step> {
    let mut steps = Vec::new();
    for name in &recipe.pkg {
        steps.push(Step {
            recipe: recipe.name.clone(),
            id: format!("pkg:{name}"),
            action: plan_pkg(name, manager),
        });
    }
    for link in &recipe.link {
        steps.push(Step {
            recipe: recipe.name.clone(),
            id: format!("link:{}", link.to),
            action: plan_link(link, home, repo),
        });
    }
    for key in &recipe.defaults {
        steps.push(Step {
            recipe: recipe.name.clone(),
            id: format!("defaults:{}.{}", key.domain, key.key),
            action: if with_defaults {
                plan_defaults(key)
            } else {
                Action::Unknown("defaults are a macOS thing".into())
            },
        });
    }
    for cmd in &recipe.command {
        steps.push(Step {
            recipe: recipe.name.clone(),
            id: format!("run:{}", first_words(&cmd.run)),
            action: plan_command(cmd),
        });
    }
    steps
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Brew,
    Apt,
}

impl PackageManager {
    /// Whichever one is on this machine.
    pub fn detect() -> Option<Self> {
        if which("brew") {
            Some(PackageManager::Brew)
        } else if which("apt-get") {
            Some(PackageManager::Apt)
        } else {
            None
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            PackageManager::Brew => "brew",
            PackageManager::Apt => "apt",
        }
    }

    pub fn installed(&self, pkg: &str) -> bool {
        let out = match self {
            PackageManager::Brew => Proc::new("brew").args(["list", "--versions", pkg]).output(),
            PackageManager::Apt => Proc::new("dpkg-query")
                .args(["-W", "-f=${Status}", pkg])
                .output(),
        };
        match out {
            Ok(o) => match self {
                PackageManager::Brew => o.status.success(),
                PackageManager::Apt => String::from_utf8_lossy(&o.stdout).contains("install ok"),
            },
            Err(_) => false,
        }
    }
}

fn which(bin: &str) -> bool {
    Proc::new("command")
        .args(["-v", bin])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        || std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|p| p.join(bin).is_file()))
            .unwrap_or(false)
}

fn read_default(domain: &str, key: &str) -> Option<String> {
    let out = Proc::new("defaults")
        .args(["read", domain, key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn run(cmd: &str) -> std::io::Result<bool> {
    Ok(Proc::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()?
        .status
        .success())
}

fn value_string(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `defaults read` prints a bool as 0 or 1, which is not what the recipe says.
fn same_value(now: &str, want: &str, kind: &str) -> bool {
    if kind == "bool" {
        let truthy = ["1", "true", "yes"];
        return truthy.contains(&now) == truthy.contains(&want);
    }
    now == want
}

fn first_words(cmd: &str) -> String {
    let words: Vec<&str> = cmd.split_whitespace().take(3).collect();
    let joined = words.join(" ");
    if cmd.split_whitespace().count() > 3 {
        format!("{joined}…")
    } else {
        joined
    }
}

/// A source path reads better relative to whatever it is under: the repo for a
/// file the recipes ship, the home for anything else. An absolute temp path
/// tells a reader nothing and wraps over two lines doing it.
fn show_source(path: &Path, home: &Path, repo: &Path) -> String {
    if let Ok(rest) = path.strip_prefix(repo) {
        return rest.display().to_string();
    }
    show(path, home)
}

fn show(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn link(to: &str) -> Link {
        Link {
            from: "configs/.zshrc".into(),
            to: to.into(),
        }
    }

    #[test]
    fn a_link_that_is_not_there_would_be_created() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let action = plan_link(&link("~/.zshrc"), home.path(), repo.path());
        assert!(matches!(action, Action::Create(_)), "{action:?}");
    }

    #[test]
    fn a_link_that_already_points_there_is_nothing_to_do() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::create_dir_all(repo.path().join("configs")).unwrap();
        let src = repo.path().join("configs/.zshrc");
        fs::write(&src, "").unwrap();
        std::os::unix::fs::symlink(&src, home.path().join(".zshrc")).unwrap();

        assert_eq!(
            plan_link(&link("~/.zshrc"), home.path(), repo.path()),
            Action::None
        );
    }

    #[test]
    fn a_link_pointing_somewhere_else_says_where() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let elsewhere = home.path().join("other");
        fs::write(&elsewhere, "").unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.path().join(".zshrc")).unwrap();

        match plan_link(&link("~/.zshrc"), home.path(), repo.path()) {
            Action::Replace(what) => assert!(what.contains("other"), "{what}"),
            other => panic!("expected a replacement, got {other:?}"),
        }
    }

    #[test]
    fn a_real_file_in_the_way_is_a_replacement_not_a_create() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::write(home.path().join(".zshrc"), "someone's own config").unwrap();

        let action = plan_link(&link("~/.zshrc"), home.path(), repo.path());
        assert!(matches!(action, Action::Replace(_)), "{action:?}");
        assert!(action.is_change());
    }

    #[test]
    fn a_package_with_no_manager_is_unknown_rather_than_assumed() {
        let action = plan_pkg("ripgrep", None);
        assert!(matches!(action, Action::Unknown(_)), "{action:?}");
    }

    #[test]
    fn a_command_whose_check_passes_is_already_done() {
        let cmd = Command {
            run: "curl -LsSf https://example.test | sh".into(),
            check: "true".into(),
        };
        assert_eq!(plan_command(&cmd), Action::None);
    }

    #[test]
    fn a_command_whose_check_fails_would_run() {
        let cmd = Command {
            run: "curl -LsSf https://example.test | sh".into(),
            check: "false".into(),
        };
        match plan_command(&cmd) {
            Action::Create(what) => assert!(what.starts_with("run: curl"), "{what}"),
            other => panic!("expected to run, got {other:?}"),
        }
    }

    #[test]
    fn a_bool_default_reads_back_as_a_number_and_still_matches() {
        assert!(same_value("1", "true", "bool"));
        assert!(same_value("0", "false", "bool"));
        assert!(!same_value("0", "true", "bool"));
        assert!(!same_value("42", "43", "int"));
    }

    #[test]
    fn a_plan_names_every_resource_a_recipe_touches() {
        let recipe: Recipe = toml::from_str(
            r#"
            name = "shell"
            pkg = ["definitely-not-a-real-package-xyz"]
            link = [{ from = "configs/.zshrc", to = "~/.zshrc" }]
        "#,
        )
        .unwrap();
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();

        let steps = plan_recipe(&recipe, home.path(), repo.path(), None, false);

        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].id, "pkg:definitely-not-a-real-package-xyz");
        assert_eq!(steps[1].id, "link:~/.zshrc");
        assert!(steps.iter().all(|s| s.recipe == "shell"));
    }
}
