//! Inline autocomplete popups that overlay the chat surface while
//! `Mode::Chat` stays active. Two flavours:
//!
//! - **Slash command autocomplete** — triggered by `/` at the start of the
//!   input. Filters [`SLASH_COMMANDS`] as the user types.
//! - **File / folder mention** — triggered by `@` at the start of a token
//!   (start-of-input or after whitespace). Walks the workspace tree on
//!   first trigger, then filters that snapshot as the user types.
//!
//! Both popups are mutually exclusive: only one is ever active at a time.
//! Keys when active: ↑/↓ to navigate, Tab/Enter to insert, Esc to dismiss,
//! anything else falls through to the textarea (which updates the filter).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// One row in the slash autocomplete list. `name` is the canonical name
/// (without leading `/`), `desc` is a short hint shown to the right.
pub struct SlashCommand {
    pub name: &'static str,
    pub desc: &'static str,
}

/// Canonical slash commands. Aliases (`m` for `model`, `ls` for `models`,
/// etc.) are accepted by the parser in `event.rs::parse_command` but only
/// the primary name is suggested — keeps the popup list scannable.
pub const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "help",
        desc: "show inline help",
    },
    SlashCommand {
        name: "new",
        desc: "start a fresh session",
    },
    SlashCommand {
        name: "sessions",
        desc: "list recent saved sessions",
    },
    SlashCommand {
        name: "load",
        desc: "load a session by id prefix",
    },
    SlashCommand {
        name: "more",
        desc: "load 10 older messages",
    },
    SlashCommand {
        name: "model",
        desc: "open model picker or switch",
    },
    SlashCommand {
        name: "models",
        desc: "list available models",
    },
    SlashCommand {
        name: "host",
        desc: "change Ollama host",
    },
    SlashCommand {
        name: "workspace",
        desc: "change agent workspace",
    },
    SlashCommand {
        name: "compact",
        desc: "manually compact conversation",
    },
    SlashCommand {
        name: "disconnect",
        desc: "remove a BYOK provider",
    },
    SlashCommand {
        name: "clear",
        desc: "clear visible chat",
    },
    SlashCommand {
        name: "settings",
        desc: "show account, version, and configured providers",
    },
    SlashCommand {
        name: "update",
        desc: "update hmanlab to the latest npm release",
    },
    SlashCommand {
        name: "quit",
        desc: "quit hmanlab",
    },
];

/// Slash autocomplete state.
pub struct SlashPopup {
    /// Text the user has typed after the `/` (no leading slash).
    pub filter: String,
    /// Indices into [`SLASH_COMMANDS`] that match `filter`. Recomputed
    /// every time `filter` changes.
    pub matches: Vec<usize>,
    /// Currently highlighted row within `matches`.
    pub index: usize,
}

/// File-mention autocomplete state. `matches` is filtered against
/// `workspace_files` (a snapshot taken when the popup first opened).
pub struct FilePopup {
    /// Text the user has typed after the `@` (no leading `@`).
    pub filter: String,
    /// Cached, frozen-at-open list of workspace paths (relative).
    pub workspace_files: Vec<PathBuf>,
    /// Indices into `workspace_files` that match `filter`.
    pub matches: Vec<usize>,
    /// Currently highlighted row within `matches`.
    pub index: usize,
}

/// Which popup, if any, is currently overlaying the chat surface.
pub enum InlinePopup {
    None,
    Slash(SlashPopup),
    File(FilePopup),
}

impl InlinePopup {
    pub fn is_open(&self) -> bool {
        !matches!(self, InlinePopup::None)
    }
}

impl SlashPopup {
    /// Build a fresh popup for `filter` (the text after the leading slash).
    pub fn new(filter: String) -> Self {
        let matches = filter_slash(&filter);
        Self {
            filter,
            matches,
            index: 0,
        }
    }

    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.matches = filter_slash(&self.filter);
        if self.index >= self.matches.len() {
            self.index = self.matches.len().saturating_sub(1);
        }
    }
}

impl FilePopup {
    pub fn new(filter: String, workspace: &Path) -> Self {
        let workspace_files = walk_workspace(workspace);
        let matches = filter_files(&filter, &workspace_files);
        Self {
            filter,
            workspace_files,
            matches,
            index: 0,
        }
    }

    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.matches = filter_files(&self.filter, &self.workspace_files);
        if self.index >= self.matches.len() {
            self.index = self.matches.len().saturating_sub(1);
        }
    }
}

/// Filter slash commands by a typed prefix. Case-insensitive `starts_with`
/// match — anything substring-like would be confusing for a short list.
fn filter_slash(filter: &str) -> Vec<usize> {
    let lo = filter.to_lowercase();
    if lo.is_empty() {
        return (0..SLASH_COMMANDS.len()).collect();
    }
    SLASH_COMMANDS
        .iter()
        .enumerate()
        .filter(|(_, c)| c.name.starts_with(&lo))
        .map(|(i, _)| i)
        .take(20)
        .collect()
}

/// Filter workspace files by substring (case-insensitive). Substring rather
/// than prefix because users commonly type the filename suffix (`main.rs`)
/// without knowing the directory.
fn filter_files(filter: &str, files: &[PathBuf]) -> Vec<usize> {
    let lo = filter.to_lowercase();
    if lo.is_empty() {
        return (0..files.len()).take(50).collect();
    }
    let mut out: Vec<usize> = files
        .iter()
        .enumerate()
        .filter(|(_, p)| p.to_string_lossy().to_lowercase().contains(&lo))
        .map(|(i, _)| i)
        .collect();
    // Rank exact-prefix matches above mere contains-matches so typing
    // `src/m` puts `src/main.rs` ahead of `target/debug/.../src-mention/`.
    out.sort_by_key(|&i| {
        let s = files[i].to_string_lossy().to_lowercase();
        if s.starts_with(&lo) {
            0
        } else {
            1
        }
    });
    out.truncate(50);
    out
}

/// Directories never useful in workspace mentions — same shape as the
/// sidebar walker's skip list. Skipping these keeps a `target/`-heavy
/// repo from drowning the popup in build artefacts.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    ".jj",
    "target",
    "node_modules",
    "dist",
    "build",
    "out",
    "__pycache__",
    ".next",
    ".nuxt",
    ".turbo",
    ".venv",
    "venv",
    ".cache",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".gradle",
    ".idea",
    ".vscode",
];

/// Cap on enumerated entries — protects against very large monorepos.
/// Beyond this the popup just truncates; the user can keep typing to
/// narrow the filter.
const MAX_FILES: usize = 5000;

/// Walk `workspace` and collect relative paths of files + dirs, skipping
/// the standard build-artefact directories. Synchronous and best-effort:
/// errors mid-walk just stop that branch.
fn walk_workspace(workspace: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    walk_into(workspace, workspace, &mut out, &mut visited);
    // Sort: directories first, then alphabetical within each group.
    // Matches what the sidebar shows so the mental model is consistent.
    out.sort_by(|a, b| {
        let a_dir = workspace.join(a).is_dir();
        let b_dir = workspace.join(b).is_dir();
        b_dir.cmp(&a_dir).then_with(|| a.cmp(b))
    });
    out
}

fn walk_into(workspace: &Path, dir: &Path, out: &mut Vec<PathBuf>, visited: &mut HashSet<PathBuf>) {
    if out.len() >= MAX_FILES {
        return;
    }
    let canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    if !visited.insert(canon.clone()) {
        // Symlink loop guard — never recurse into a path we've already seen.
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && SKIP_DIRS.iter().any(|s| *s == name) {
            continue;
        }
        let full = e.path();
        let rel = full
            .strip_prefix(workspace)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| full.clone());
        out.push(rel);
        if is_dir {
            walk_into(workspace, &full, out, visited);
        }
    }
}

/// Detect what — if anything — the user is currently typing that should
/// surface a popup. Reads only the first line of the input (slash commands
/// MUST be at the very start) and walks backwards from the cursor to find
/// a recent `@` for the file popup.
///
/// Returns the trigger character (`'/'` or `'@'`) and the partial filter
/// that follows it, or `None` if neither popup applies.
pub fn detect_trigger(first_line: &str, cursor_col: usize) -> Option<(char, String)> {
    // Slash trigger: first character is `/` AND there's no whitespace
    // between it and the cursor. `/foo bar` cursor-after-space doesn't
    // trigger; `/foo` cursor-anywhere-in-foo does.
    if first_line.starts_with('/') {
        let head = &first_line[..cursor_col.min(first_line.len())];
        if !head.contains(' ') {
            // Strip the leading `/` to get the filter.
            let filter = head.strip_prefix('/').unwrap_or(head).to_string();
            return Some(('/', filter));
        }
    }

    // @ trigger: walk back from cursor to find a `@` preceded by start
    // or whitespace. The substring between that `@` and the cursor (if no
    // whitespace inside) is the filter.
    let head = &first_line[..cursor_col.min(first_line.len())];
    if let Some(at_pos) = head.rfind('@') {
        // The char before the `@` must be start-of-line OR whitespace —
        // otherwise this `@` is part of an email or other token.
        let before_ok = at_pos == 0
            || head[..at_pos]
                .chars()
                .last()
                .map(|c| c.is_whitespace())
                .unwrap_or(true);
        if before_ok {
            let after = &head[at_pos + 1..];
            // Whitespace in the trailing chunk closes the trigger.
            if !after.contains(char::is_whitespace) {
                return Some(('@', after.to_string()));
            }
        }
    }

    None
}
