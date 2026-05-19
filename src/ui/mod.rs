//! UI render entry + always-on chrome (header + status bar).
//!
//! Each mode-specific surface lives in its own submodule:
//!   - chat.rs    — message history + input box
//!   - popups.rs  — model picker, session picker, add-model, confirm
//!   - markdown.rs — inline markdown parser + word-wrap (shared)

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::{App, Mode};

mod chat;
mod markdown;
mod popups;
mod sidebar;
pub(crate) mod theme;
mod viewer;
mod wrap_cache;

pub(crate) use sidebar::{initial_expanded, SidebarSnapshot};

/// Sidebar width (incl. its border). Skipped entirely when the terminal is
/// too narrow to fit it alongside a usable chat column.
const SIDEBAR_W: u16 = 26;
const SIDEBAR_MIN_TOTAL_W: u16 = 80;

pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(6),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(f, chunks[0], app);
    // On wide terminals, split the middle row into [sidebar | chat]. On
    // narrow terminals (< 80 cols) the sidebar is dropped so the chat
    // keeps full width — the input box already needs ~60 cols to be usable.
    let (chat_area, has_sidebar) = if chunks[1].width >= SIDEBAR_MIN_TOTAL_W {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(SIDEBAR_W), Constraint::Min(40)])
            .split(chunks[1]);
        sidebar::render_sidebar(f, cols[0], app);
        (cols[1], true)
    } else {
        (chunks[1], false)
    };
    if !has_sidebar {
        // Sidebar is hidden — make sure stale geometry from a previous wider
        // frame can't make sidebar clicks "stick" after a resize.
        app.render.sidebar_x = 0;
        app.render.sidebar_y = 0;
        app.render.sidebar_w = 0;
        app.render.sidebar_h = 0;
        app.render.sidebar_targets.clear();
    }
    // While a file is open the viewer takes the chat column. The chat panel
    // still keeps its scroll state so closing the viewer returns to exactly
    // the same conversation view.
    if app.open_file.is_some() {
        viewer::render_viewer(f, chat_area, app);
    } else {
        chat::render_chat(f, chat_area, app);
    }
    chat::render_input(f, chunks[2], app);
    render_status(f, chunks[3], app);

    // Inline autocomplete (slash / @ mention) — render LAST so it floats
    // above the chat panel without being clipped by it. Anchored just
    // above the input box.
    if app.inline_popup.is_open() {
        popups::render_inline_popup(f, chunks[2], app);
    }

    if app.mode == Mode::ModelPicker {
        popups::render_picker(f, area, app);
    }
    if app.mode == Mode::Confirm {
        popups::render_confirm(f, area, app);
    }
    if app.mode == Mode::AddModel {
        popups::render_add_model(f, area, app);
    }
    if app.mode == Mode::SessionPicker {
        popups::render_session_picker(f, area, app);
    }
    if app.mode == Mode::DisconnectPicker {
        popups::render_disconnect_picker(f, area, app);
    }
    if app.mode == Mode::TelegramSetup {
        popups::render_telegram_setup(f, area, app);
    }
    if app.mode == Mode::AgentsSetup {
        popups::render_agents_setup(f, area, app);
    }
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let host_short = mask_host(app.current_host());
    let total_tokens = app.total_prompt_tokens + app.total_completion_tokens;
    let tokens_label = format_tokens(total_tokens);

    let sep = Span::styled("  •  ", Style::default().fg(theme::color::FG_DIMMER));

    let mut spans = vec![
        Span::styled(
            " hmanlab ",
            Style::default()
                .fg(Color::Black)
                .bg(theme::color::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("model: ", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(
            app.model.as_str(),
            Style::default()
                .fg(theme::color::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        sep.clone(),
        Span::styled("host: ", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(host_short, Style::default().fg(theme::color::FG)),
        sep.clone(),
        Span::styled(
            format!("tokens: {tokens_label}"),
            Style::default().fg(theme::color::FG_DIM),
        ),
    ];

    // Per-specialist token breakdown — only rendered when `/ask` has
    // actually been used, so the header stays clean for single-model
    // sessions. Sorted by name for stable visual order across renders.
    if !app.agent_token_tally.is_empty() {
        let mut entries: Vec<(&String, &(u64, u64))> = app.agent_token_tally.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (name, (p, c)) in entries {
            spans.push(sep.clone());
            spans.push(Span::styled(
                format!("{name}: ", name = name),
                Style::default().fg(theme::color::FG_DIM),
            ));
            spans.push(Span::styled(
                format!("{}/{}", format_tokens(*p), format_tokens(*c)),
                Style::default().fg(theme::color::ASSISTANT),
            ));
        }
    }

    // Background update check tagged us — surface the upgrade hint at
    // the right end of the header so it's visible but never in the way.
    if let Some(latest) = app.update_available.as_deref() {
        spans.push(sep);
        spans.push(Span::styled(
            format!("v{latest} available — npm i -g hmanlab"),
            Style::default()
                .fg(theme::color::SUCCESS)
                .add_modifier(Modifier::BOLD),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_status(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▎ ", Style::default().fg(theme::color::ACCENT_ALT)),
            Span::styled(app.status.as_str(), Style::default().fg(theme::color::FG)),
        ])),
        chunks[0],
    );
    let help = Line::from(vec![
        Span::styled("/help", Style::default().fg(theme::color::FG)),
        Span::styled("  ·  ", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("alt+enter", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(" newline", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("  ·  ", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("drag", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(" copy", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("  ·  ", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("wheel", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(" scroll", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("  ·  ", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("^T", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(" fold ", Style::default().fg(theme::color::FG_DIMMER)),
    ]);
    f.render_widget(Paragraph::new(help).alignment(Alignment::Right), chunks[1]);
}

/// Strip scheme and port from the configured host URL — `http://192.168.3.3:11434`
/// becomes `192.168.3.3`. Keeps the underlying connection URL intact in `app.client.base`.
fn mask_host(base: &str) -> String {
    let no_scheme = base
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let no_path = no_scheme.split('/').next().unwrap_or(no_scheme);
    // IPv6 literals use brackets: `[::1]:port`. Strip from `]` onward, keep brackets.
    if no_path.starts_with('[') {
        if let Some(close) = no_path.find(']') {
            return no_path[..=close].to_string();
        }
    }
    // For everything else, drop the port if present.
    match no_path.rfind(':') {
        Some(i) => no_path[..i].to_string(),
        None => no_path.to_string(),
    }
}

/// Render a token count compactly: 832 → "832", 12345 → "12.3k", 1_500_000 → "1.5M".
fn format_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}
