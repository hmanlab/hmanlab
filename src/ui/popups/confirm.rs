//! `render_confirm` — destructive-action y/n dialog. Used for shell
//! commands and file writes. Renders the prompt plus a coloured diff (if
//! the tool attached one), reserves a footer line for the action keys
//! even when the body overflows, and supports keyboard scroll for long
//! diffs.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::tools;

use super::super::markdown::{parse_inline_md, wrap_styled_segments};
use super::super::theme;

pub(in crate::ui) fn render_confirm(f: &mut Frame, area: Rect, app: &mut App) {
    f.render_widget(Clear, area);

    let prompt = app
        .pending_confirm
        .as_ref()
        .map(|r| r.prompt.as_str())
        .unwrap_or("(no pending request)");

    // Always treat the confirm popup as "danger" — every prompt that lands
    // here is a destructive or shell-touching action that wants user attention.
    let block = theme::popup_block("confirm action", true).padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Reserve the bottom line for the y/n footer ALWAYS, even if the prompt
    // body would otherwise overflow it. This is the fix for "I only saw the
    // prompt, not the y/n options" on long edit_file diffs.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1), // spacer
            Constraint::Length(1), // footer
        ])
        .split(inner);
    let content_area = chunks[0];
    let footer_area = chunks[2];

    let content_width = (content_area.width as usize).saturating_sub(1).max(10);
    let mut lines: Vec<Line> = Vec::new();
    for paragraph in prompt.split('\n') {
        if paragraph.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        let segments = parse_inline_md(paragraph, Style::default());
        for spans in wrap_styled_segments(segments, content_width) {
            lines.push(Line::from(spans));
        }
    }
    // Coloured diff (only populated for edit_file / write_file). One blank
    // separator above so the header and diff body don't visually fuse.
    if let Some(req) = app.pending_confirm.as_ref() {
        if !req.diff.is_empty() {
            lines.push(Line::from(""));
            for dl in &req.diff {
                let style = match dl.kind {
                    tools::DiffLineKind::Added => Style::default().fg(theme::color::SUCCESS),
                    tools::DiffLineKind::Removed => Style::default().fg(theme::color::ERROR),
                    tools::DiffLineKind::Context => Style::default().fg(theme::color::FG_DIM),
                    tools::DiffLineKind::Summary => Style::default()
                        .fg(theme::color::WARNING)
                        .add_modifier(Modifier::BOLD),
                };
                // Wrap each diff line at content_width so long lines stay
                // inside the popup. Re-apply the colour to every wrapped
                // sub-line so a long delete doesn't turn black halfway.
                for chunk in wrap_styled_segments(vec![(dl.text.clone(), style)], content_width) {
                    lines.push(Line::from(chunk));
                }
            }
        }
    }
    // Clamp scroll so End / PgDn-past-end snaps to the last full screen.
    // `Paragraph::scroll` doesn't itself clamp, so without this an u16::MAX
    // would render an empty box.
    let total_lines = lines.len() as u16;
    let visible = content_area.height;
    let max_scroll = total_lines.saturating_sub(visible);
    if app.confirm_scroll > max_scroll {
        app.confirm_scroll = max_scroll;
    }
    let scroll = app.confirm_scroll;

    let body = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(body, content_area);

    // Footer: scroll position (if any) on the right of the action keys, so
    // long diffs make it obvious there's more below.
    let scroll_hint = if max_scroll > 0 {
        let shown_end = (scroll as usize + visible as usize).min(total_lines as usize);
        format!(
            "  ·  ↑↓ PgUp/PgDn scroll  ·  {}/{} lines",
            shown_end, total_lines
        )
    } else {
        String::new()
    };
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "[y]",
            Style::default()
                .fg(theme::color::SUCCESS)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" allow   ", Style::default().fg(theme::color::FG)),
        Span::styled(
            "[n]",
            Style::default()
                .fg(theme::color::ERROR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" deny   ", Style::default().fg(theme::color::FG)),
        Span::styled("[Esc]", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(" deny", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(scroll_hint, Style::default().fg(theme::color::FG_DIM)),
    ]));
    f.render_widget(footer, footer_area);
}
