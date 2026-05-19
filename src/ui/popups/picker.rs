//! `render_picker` — the model picker overlay (Ctrl+M or `/model`).
//!
//! Lists Ollama-discovered models, BYOK extras, and "+ Add … key" rows
//! for each unconfigured provider. Colour-codes by source so the user
//! can tell at a glance which selections come from where.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Clear, List, ListItem, Padding},
    Frame,
};

use crate::app::{App, PickerEntry};

use super::super::theme;
use super::centered_rect;

pub(in crate::ui) fn render_picker(f: &mut Frame, full: Rect, app: &App) {
    let area = centered_rect(60, 60, full);
    f.render_widget(Clear, area);
    let items: Vec<ListItem> = app
        .model_picker
        .items
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let label = e.display();
            let entry_color = match e {
                PickerEntry::Ollama(_) => theme::color::FG,
                PickerEntry::Extra(_) => theme::color::ASSISTANT,
                PickerEntry::AddProvider(_) => theme::color::USER,
            };
            let style = if i == app.model_picker.index {
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(theme::color::ACCENT_ALT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(entry_color)
            };
            ListItem::new(format!(" {} ", label)).style(style)
        })
        .collect();
    let list = List::new(items).block(
        theme::popup_block("select model — ↑↓ Enter Esc", false).padding(Padding::horizontal(1)),
    );
    f.render_widget(list, area);
}
