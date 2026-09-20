//! What a machine should have on it.
//!
//! Recipes are data, not code. That is a finding rather than a preference:
//! twenty-two modules of a working installer were classified, and more than
//! four fifths of them were a package list, a link, or a `defaults` key once
//! the providers existed (DESIGN.md, appendix A).
//!
//! The imperative fifth is almost all *somebody else's installer* being
//! fetched and run, so there is one escape hatch — and it must carry a
//! `check`, because a step that cannot say whether it is already done cannot
//! take part in a plan, and the plan is the whole contract.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::expand;

#[derive(Debug, Default, Deserialize)]
pub struct Recipes {
    #[serde(default, rename = "recipe")]
    pub recipes: Vec<Recipe>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Recipe {
    pub name: String,
    /// `macos`, `linux`; absent means everywhere.
    #[serde(default)]
    pub platform: Option<String>,
    /// Who this is for. A work machine does not install a personal toy.
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub pkg: Vec<String>,
    #[serde(default)]
    pub link: Vec<Link>,
    #[serde(default)]
    pub defaults: Vec<DefaultsKey>,
    #[serde(default)]
    pub command: Vec<Command>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Link {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefaultsKey {
    pub domain: String,
    pub key: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: toml::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Command {
    pub run: String,
    /// How to tell it has already been done. Without this the step cannot be
    /// planned, only performed, and a tool that performs without planning is
    /// the thing this design exists to avoid.
    pub check: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RecipeError {
    #[error("{path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

impl Recipes {
    pub fn load(path: &Path) -> Result<Self, RecipeError> {
        let text = std::fs::read_to_string(path).map_err(|source| RecipeError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| RecipeError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }
}

impl Recipe {
    /// Does this recipe apply to the machine running it?
    pub fn applies_here(&self, platform: &str) -> bool {
        match &self.platform {
            None => true,
            Some(p) => p == platform,
        }
    }
}

impl Link {
    pub fn expanded(&self, home: &Path, repo: &Path) -> (PathBuf, PathBuf) {
        let from = if self.from.starts_with('~') || self.from.starts_with('/') {
            PathBuf::from(expand(&self.from, home))
        } else {
            repo.join(&self.from)
        };
        (from, PathBuf::from(expand(&self.to, home)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [[recipe]]
        name = "shell"
        pkg = ["zsh", "fzf"]
        link = [{ from = "configs/.zshrc", to = "~/.zshrc" }]

        [[recipe]]
        name = "macos"
        platform = "macos"
        defaults = [
          { domain = "com.apple.dock", key = "tilesize", type = "int", value = 42 },
        ]

        [[recipe]]
        name = "rust"
        [[recipe.command]]
        run = "curl -LsSf https://sh.rustup.rs | sh -s -- -y"
        check = "rustup --version"
    "#;

    #[test]
    fn reads_packages_links_defaults_and_commands() {
        let r: Recipes = toml::from_str(SAMPLE).unwrap();
        assert_eq!(r.recipes.len(), 3);
        assert_eq!(r.recipes[0].pkg, ["zsh", "fzf"]);
        assert_eq!(r.recipes[0].link[0].to, "~/.zshrc");
        assert_eq!(r.recipes[1].defaults[0].key, "tilesize");
        assert_eq!(r.recipes[2].command[0].check, "rustup --version");
    }

    #[test]
    fn a_platform_recipe_stays_on_its_platform() {
        let r: Recipes = toml::from_str(SAMPLE).unwrap();
        assert!(
            r.recipes[0].applies_here("linux"),
            "no platform means everywhere"
        );
        assert!(r.recipes[1].applies_here("macos"));
        assert!(!r.recipes[1].applies_here("linux"));
    }

    #[test]
    fn a_command_without_a_check_is_refused_at_load() {
        let err = toml::from_str::<Recipes>(
            r#"
            [[recipe]]
            name = "x"
            [[recipe.command]]
            run = "make install"
        "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("check"), "{err}");
    }

    #[test]
    fn a_link_source_is_relative_to_the_repo_and_a_target_to_the_home() {
        let r: Recipes = toml::from_str(SAMPLE).unwrap();
        let (from, to) = r.recipes[0].link[0].expanded(Path::new("/home/x"), Path::new("/repo"));
        assert_eq!(from, Path::new("/repo/configs/.zshrc"));
        assert_eq!(to, Path::new("/home/x/.zshrc"));
    }
}
