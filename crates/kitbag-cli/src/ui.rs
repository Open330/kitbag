//! How a run looks.
//!
//! Two rules decide everything here. Output is grouped by **whose** state it
//! is, because that is the question the tool exists to answer — not by name,
//! not by file type. And colour is a hint, never the message: every marker
//! reads the same in a pipe, on a dumb terminal, and under `NO_COLOR`.

use std::fmt::Write as _;
use std::io::IsTerminal;

use kitbag_core::Scope;

/// What a run would do to one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    New,
    Changed,
    Unchanged,
    /// Only doing the work would tell — an artifact that has to be built
    /// before it can be compared, and building it is not free.
    Unknown,
}

impl Mark {
    pub fn glyph(self) -> char {
        match self {
            Mark::New => '+',
            Mark::Changed => '~',
            Mark::Unchanged => '=',
            Mark::Unknown => '?',
        }
    }

    fn colour(self) -> &'static str {
        match self {
            Mark::New => "\x1b[32m",
            Mark::Changed => "\x1b[33m",
            Mark::Unchanged => "\x1b[90m",
            Mark::Unknown => "\x1b[33m",
        }
    }
}

/// One line of a report.
#[derive(Debug, Clone)]
pub struct Row {
    pub mark: Option<Mark>,
    pub name: String,
    /// What is inside: variable names, a host count, a fingerprint. Never a
    /// value.
    pub detail: String,
}

/// A group of rows that share an owner.
#[derive(Debug, Clone)]
pub struct Group {
    pub scope: Scope,
    pub owner: Option<String>,
    pub rows: Vec<Row>,
}

impl Group {
    fn heading(&self) -> String {
        match &self.owner {
            Some(o) => format!("{} · {}", self.scope, o),
            None => self.scope.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    Always,
    Never,
}

impl Colour {
    /// `--color`, then `NO_COLOR`, then whether anyone is watching.
    pub fn resolve(flag: Option<&str>) -> Self {
        match flag {
            Some("always") => Colour::Always,
            Some("never") => Colour::Never,
            _ => {
                if std::env::var_os("NO_COLOR").is_some() || !std::io::stdout().is_terminal() {
                    Colour::Never
                } else {
                    Colour::Always
                }
            }
        }
    }

    fn wrap(self, code: &str, s: &str) -> String {
        match self {
            Colour::Always => format!("{code}{s}\x1b[0m"),
            Colour::Never => s.to_string(),
        }
    }
}

/// personal first, mixed last: the clear cases before the ones that need a
/// second thought.
fn rank(scope: &Scope) -> u8 {
    match scope {
        Scope::Personal => 0,
        Scope::Shared => 1,
        Scope::Work => 2,
        Scope::Mixed { .. } => 3,
        Scope::Local => 4,
    }
}

/// The whole report, as it appears on a terminal.
pub fn render(groups: &[Group], colour: Colour, width: usize) -> String {
    let mut sorted: Vec<&Group> = groups.iter().collect();
    sorted.sort_by_key(|g| (rank(&g.scope), g.heading()));

    let name_col = 26;
    let mut out = String::new();

    for group in sorted {
        let heading = colour.wrap("\x1b[1m", &group.heading());
        let _ = writeln!(out, "\n  {heading}  ({})", group.rows.len());

        for (i, row) in group.rows.iter().enumerate() {
            let last = i + 1 == group.rows.len();
            let (branch, cont) = if last {
                ("  └── ", "      ")
            } else {
                ("  ├── ", "  │   ")
            };

            let mark = match row.mark {
                Some(m) => colour.wrap(m.colour(), &m.glyph().to_string()) + " ",
                None => String::new(),
            };
            let mark_width = if row.mark.is_some() { 2 } else { 0 };

            let avail = width.saturating_sub(6 + mark_width + name_col + 1).max(24);
            for (n, line) in wrap(&row.detail, avail).into_iter().enumerate() {
                if n == 0 {
                    let _ = writeln!(out, "{branch}{mark}{:<name_col$} {line}", row.name);
                } else {
                    let pad = " ".repeat(mark_width + name_col + 1);
                    let _ = writeln!(out, "{cont}{pad}{line}");
                }
            }
        }
    }

    let tally = tally(groups);
    if !tally.is_empty() {
        let _ = writeln!(out, "\n  {}", tally.join("   "));
    }
    out
}

fn tally(groups: &[Group]) -> Vec<String> {
    let mut counts = [0usize; 4];
    for row in groups.iter().flat_map(|g| &g.rows) {
        match row.mark {
            Some(Mark::New) => counts[0] += 1,
            Some(Mark::Changed) => counts[1] += 1,
            Some(Mark::Unchanged) => counts[2] += 1,
            Some(Mark::Unknown) => counts[3] += 1,
            None => {}
        }
    }
    let labels = ["new", "changed", "unchanged", "not comparable"];
    let marks = [Mark::New, Mark::Changed, Mark::Unchanged, Mark::Unknown];
    counts
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(i, n)| format!("{} {n} {}", marks[i].glyph(), labels[i]))
        .collect()
}

/// Wrap on spaces, so a long list of variable names stays under the item it
/// belongs to instead of running off the side of the terminal.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(mark: Mark, name: &str, detail: &str) -> Row {
        Row {
            mark: Some(mark),
            name: name.into(),
            detail: detail.into(),
        }
    }

    fn sample() -> Vec<Group> {
        vec![
            Group {
                scope: Scope::Work,
                owner: Some("acme".into()),
                rows: vec![
                    row(Mark::Changed, "env:ci", "CI_TOKEN CI_URL"),
                    row(Mark::Unchanged, "env:reports", "REPORTS_KEY"),
                ],
            },
            Group {
                scope: Scope::Personal,
                owner: None,
                rows: vec![row(Mark::New, "file:npmrc", "registry _authToken")],
            },
        ]
    }

    #[test]
    fn personal_comes_before_work() {
        let out = render(&sample(), Colour::Never, 100);
        let personal = out.find("personal").unwrap();
        let work = out.find("work · acme").unwrap();
        assert!(personal < work, "{out}");
    }

    #[test]
    fn reads_the_same_without_colour() {
        let plain = render(&sample(), Colour::Never, 100);
        assert!(!plain.contains('\x1b'), "escape codes leaked into a pipe");
        for glyph in ['+', '~', '='] {
            assert!(plain.contains(glyph), "{glyph} missing from {plain}");
        }
    }

    #[test]
    fn colour_is_only_a_hint() {
        let coloured = render(&sample(), Colour::Always, 100);
        let plain = render(&sample(), Colour::Never, 100);
        let stripped: String = strip_ansi(&coloured);
        assert_eq!(stripped, plain);
    }

    #[test]
    fn the_last_row_of_a_group_closes_it() {
        let out = render(&sample(), Colour::Never, 100);
        assert!(out.contains("└──"));
        assert!(out.contains("├──"));
    }

    #[test]
    fn a_long_detail_wraps_under_itself_not_off_the_screen() {
        let names = (0..12)
            .map(|i| format!("VARIABLE_NUMBER_{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let groups = vec![Group {
            scope: Scope::Personal,
            owner: None,
            rows: vec![row(Mark::New, "env:big", &names)],
        }];
        let out = render(&groups, Colour::Never, 80);
        assert!(out.lines().all(|l| l.chars().count() <= 80), "{out}");
        assert!(out.contains("VARIABLE_NUMBER_11"));
    }

    #[test]
    fn the_tally_counts_what_was_shown() {
        let out = render(&sample(), Colour::Never, 100);
        assert!(out.contains("+ 1 new"), "{out}");
        assert!(out.contains("~ 1 changed"), "{out}");
        assert!(out.contains("= 1 unchanged"), "{out}");
    }

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}
