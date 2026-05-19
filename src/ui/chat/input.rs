//! `render_input` — the bottom textarea. The block's title and border
//! colour reflect the input's current mode (generating, slash command,
//! Y/N pending, normal), so the box state is scannable from across the
//! screen without reading the actual contents.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Span,
    widgets::Padding,
    Frame,
};

use crate::app::App;

use super::super::theme;

pub(in crate::ui) fn render_input(f: &mut Frame, area: Rect, app: &mut App) {
    let first_line = app.input.lines().first().cloned().unwrap_or_default();
    let is_cmd = first_line.trim_start().starts_with('/');

    // Title encodes input mode; border colour echoes it so the box state
    // is scannable from across the screen.
    let (title, border_color) = if app.turn.is_generating() {
        (
            "▎ generating · Ctrl+C to cancel".to_string(),
            theme::color::WARNING,
        )
    } else if is_cmd {
        (
            "▎ command · Enter to run".to_string(),
            theme::color::ACCENT_ALT,
        )
    } else if app.yn_pending {
        (
            "▎ [Y] yes  ·  [N] no  ·  type to override".to_string(),
            theme::color::ASSISTANT,
        )
    } else {
        ("▎ message".to_string(), theme::color::ACCENT)
    };

    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1));
    app.input.set_block(block);
    // Stash the inner content width so the event handler can soft-wrap
    // typed characters before they push the cursor off the right edge.
    // -2 for the rounded borders, -2 for the horizontal padding.
    app.render.input_inner_w = area.width.saturating_sub(4);
    f.render_widget(&app.input, area);
}
