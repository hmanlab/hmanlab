//! Overlay surfaces — modal popups + the floating inline-autocomplete.
//!
//! Each render fn lives in its own submodule and is wired into
//! `ui::mod::render` via the `pub(super) use` re-exports below. They
//! share only `centered_rect`, which positions a fixed-percent box in
//! the middle of the screen.
//!
//!   - `picker`     — model picker (Ctrl+M / `/model`).
//!   - `sessions`   — `/sessions` saved-session list.
//!   - `disconnect` — `/disconnect` BYOK-provider picker.
//!   - `add_model`  — paste-your-API-key dialog after picking
//!     "+ Add … key" in the model picker.
//!   - `confirm`    — destructive-action y/n dialog with diff preview.
//!   - `inline`     — floating `/cmd` and `@file` autocomplete that
//!     anchors above the input.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

mod add_model;
mod agents_setup;
mod confirm;
mod disconnect;
mod inline;
mod picker;
mod sessions;
mod telegram_setup;

pub(super) use add_model::render_add_model;
pub(super) use agents_setup::render_agents_setup;
pub(super) use confirm::render_confirm;
pub(super) use disconnect::render_disconnect_picker;
pub(super) use inline::render_inline_popup;
pub(super) use picker::render_picker;
pub(super) use sessions::render_session_picker;
pub(super) use telegram_setup::render_telegram_setup;

/// Centre a (`percent_x` % wide, `percent_y` % tall) rectangle inside `r`.
/// Used by every full-screen popup so they all share the same anchoring.
pub(super) fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
