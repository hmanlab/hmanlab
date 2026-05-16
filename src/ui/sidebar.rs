//! Workspace sidebar — a tree of the agent workspace directory the user can
//! click to expand/collapse. Rebuilt each frame from the current
//! `app.expanded_dirs` set and `app.workspace`; cheap because we only
//! recurse into expanded directories.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
    Frame,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::app::App;

/// Hard cap on total entries — protects against a pathological expansion
/// (deeply nested monorepo) from blowing up the render. A trailing `…` row
/// signals truncation.
const MAX_ENTRIES: usize = 1000;

/// Directory names that are virtually never useful in a code-review sidebar.
/// Skipped at every depth. Shared with the seeding helper below so the
/// initial expanded set matches what the renderer will actually show.
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

struct Entry {
    depth: u8,
    name: String,
    is_dir: bool,
    /// Absolute path on disk — joined as we recurse so the click handler can
    /// open the file or toggle its directory without re-walking from the root.
    path: PathBuf,
}

/// Seed the initial `expanded_dirs` set for a workspace: workspace root
/// itself, plus its immediate (visible) child directories. Makes the first
/// launch show one level of contents at a glance while leaving deeper
/// directories collapsed.
pub(crate) fn initial_expanded(workspace: &Path) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    // The root is always implicitly expanded — keep it in the set so the
    // walk function can use a single membership check at every level.
    out.insert(workspace.to_path_buf());
    let Ok(read) = std::fs::read_dir(workspace) else {
        return out;
    };
    for e in read.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir || SKIP_DIRS.iter().any(|s| *s == name) {
            continue;
        }
        out.insert(e.path());
    }
    out
}

pub(super) fn render_sidebar(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" workspace ")
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);

    // Stash inner geometry so `event::handle_mouse` can hit-test clicks
    // and route wheel events.
    app.sidebar_x = inner.x;
    app.sidebar_y = inner.y;
    app.sidebar_w = inner.width;
    app.sidebar_h = inner.height;
    app.sidebar_targets.clear();

    let mut entries: Vec<Entry> = Vec::new();
    walk(&app.workspace, 0, &app.expanded_dirs, &mut entries);

    let mut lines: Vec<Line> = Vec::with_capacity(entries.len() + 1);

    let basename = app
        .workspace
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| app.workspace.display().to_string());
    let max_w = inner.width as usize;
    // Root header — always "expanded" visually; no toggle.
    lines.push(Line::from(Span::styled(
        truncate(&format!("▾ {basename}/"), max_w),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));

    for (line_offset, e) in entries.iter().enumerate() {
        let indent = "  ".repeat(e.depth as usize + 1);
        let is_truncation = e.name == "…";

        // Build the row: dirs get a chevron prefix indicating expand state;
        // files get just an indent. Truncation sentinel is rendered as a
        // dim placeholder with no chevron.
        let label = if is_truncation {
            e.name.clone()
        } else if e.is_dir {
            let chevron = if app.expanded_dirs.contains(&e.path) {
                '▾'
            } else {
                '▸'
            };
            format!("{chevron} {}/", e.name)
        } else {
            e.name.clone()
        };
        let max_label = max_w.saturating_sub(indent.chars().count());
        let display = truncate(&label, max_label);
        let style = if is_truncation {
            Style::default().fg(Color::DarkGray)
        } else if e.is_dir {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(vec![
            Span::raw(indent),
            Span::styled(display, style),
        ]));

        // Record logical line index (1 + offset, because line 0 is the root
        // header). The click handler converts screen Y → logical via
        // `(screen_y - sidebar_y) + sidebar_scroll`.
        if !is_truncation {
            let logical = (line_offset as u16).saturating_add(1);
            app.sidebar_targets.push((logical, e.path.clone(), e.is_dir));
        }
    }

    // Clamp scroll to a valid range so wheel-past-end snaps back when the
    // tree gets shorter (e.g. after collapsing).
    let total = lines.len() as u16;
    let visible = inner.height;
    let max_scroll = total.saturating_sub(visible);
    if app.sidebar_scroll > max_scroll {
        app.sidebar_scroll = max_scroll;
    }

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.sidebar_scroll, 0));
    f.render_widget(para, area);
}

fn walk(dir: &Path, depth: u8, expanded: &HashSet<PathBuf>, out: &mut Vec<Entry>) {
    // Only descend into directories the user has expanded. The workspace
    // root is pre-seeded into `expanded` (see `initial_expanded`), so the
    // first call always enters at least one level.
    if !expanded.contains(dir) || out.len() >= MAX_ENTRIES {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<(String, bool)> = Vec::new();
    for e in read.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // Conventional file-tree behaviour: hide dotfiles. Cuts noise from
        // .git ignore artefacts, lockfile companions, editor configs, etc.
        if name.starts_with('.') {
            continue;
        }
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && SKIP_DIRS.iter().any(|s| *s == name) {
            continue;
        }
        items.push((name, is_dir));
    }
    items.sort_by(|(an, ad), (bn, bd)| {
        bd.cmp(ad)
            .then_with(|| an.to_lowercase().cmp(&bn.to_lowercase()))
    });
    for (name, is_dir) in items {
        let full = dir.join(&name);
        if out.len() >= MAX_ENTRIES {
            out.push(Entry {
                depth,
                name: "…".into(),
                is_dir: false,
                path: full,
            });
            return;
        }
        out.push(Entry {
            depth,
            name: name.clone(),
            is_dir,
            path: full.clone(),
        });
        if is_dir {
            walk(&full, depth + 1, expanded, out);
        }
    }
}

/// Truncate a label to `max` display columns, appending `…` if cut. Uses
/// char count as a proxy for display width — fine for ASCII source paths.
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
