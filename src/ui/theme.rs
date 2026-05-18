//! Central theme — Catppuccin Mocha-derived palette + helper builders.
//!
//! Every renderer pulls colors and `Block` builders from here instead of
//! inlining `Color::Cyan` / `Color::Magenta` ad-hoc. That keeps the look
//! coherent across header, sidebar, chat, popups, and viewer.

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, BorderType, Borders},
};

/// Color tokens. Values come from Catppuccin Mocha; the palette is
/// documented at <https://catppuccin.com/palette>. RGB literals render
/// on truecolor terminals; ratatui's terminal layer degrades to
/// 256-color or 16-color on legacy terminals automatically.
pub mod color {
    use ratatui::style::Color;

    // Accents
    pub const ACCENT: Color = Color::Rgb(250, 179, 135); // peach
    #[allow(dead_code)] // exposed for future breath-color / focus gradient use
    pub const ACCENT_DIM: Color = Color::Rgb(180, 130, 95);
    pub const ACCENT_ALT: Color = Color::Rgb(203, 166, 247); // mauve

    // Role markers
    pub const USER: Color = Color::Rgb(166, 227, 161); // green
    pub const ASSISTANT: Color = Color::Rgb(137, 220, 235); // sky
    pub const SYSTEM: Color = Color::Rgb(180, 190, 254); // lavender
    pub const TOOL: Color = Color::Rgb(137, 220, 235); // sky
    pub const TOOL_ERROR: Color = Color::Rgb(243, 139, 168); // red

    // Surfaces
    pub const FG: Color = Color::Rgb(205, 214, 244); // text
    pub const FG_DIM: Color = Color::Rgb(108, 112, 134); // overlay0
    pub const FG_DIMMER: Color = Color::Rgb(69, 71, 90); // surface1

    // Borders
    pub const BORDER_ACTIVE: Color = ACCENT; // peach
    pub const BORDER_IDLE: Color = Color::Rgb(69, 71, 90); // surface1

    // Card surface — used to group consecutive read-only tool messages into
    // a single borderless tile (see `ui::chat::render_chat`). One step up
    // from the chat panel's transparent canvas so the card reads as a
    // distinct, slightly-elevated block without needing a frame.
    pub const BG_CARD: Color = Color::Rgb(49, 50, 68); // catppuccin surface0
    /// Hovered card row — one elevation brighter than `BG_CARD` so the
    /// pointer's current target reads clearly as "clickable" without a
    /// chevron or arrow icon.
    pub const BG_CARD_HOVER: Color = Color::Rgb(88, 91, 112); // catppuccin surface2

    // Status / diff
    pub const SUCCESS: Color = USER;
    pub const ERROR: Color = TOOL_ERROR;
    pub const WARNING: Color = Color::Rgb(249, 226, 175); // yellow
}

/// Horizontal inset inside any panel border. One col on each side gives
/// content breathing room without burning real estate on narrow terms.
#[allow(dead_code)] // documents the convention; renderers currently call Padding::horizontal(1) directly
pub const PANEL_PAD_H: u16 = 1;

/// Build a rounded panel-style `Block` with focus-aware border colour.
/// Pass an empty string for `title` to render a borderless-title rounded
/// container.
pub fn panel_block(title: &str, focused: bool) -> Block<'_> {
    let border_color = if focused {
        color::BORDER_ACTIVE
    } else {
        color::BORDER_IDLE
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color));
    if !title.is_empty() {
        block = block.title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(if focused {
                    color::ACCENT
                } else {
                    color::FG_DIM
                })
                .add_modifier(Modifier::BOLD),
        ));
    }
    block
}

/// Build a rounded "popup" block — distinctly styled with the alt-accent
/// (mauve) so popups feel like overlays rather than another panel.
/// `danger=true` swaps the border to red for destructive confirmations
/// (run_command, write_file with diff that drops content, etc.).
pub fn popup_block(title: &str, danger: bool) -> Block<'_> {
    let border_color = if danger {
        color::ERROR
    } else {
        color::ACCENT_ALT
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        ))
}

/// Role label + color used at the head of each chat message.
/// The leading `▎` is a colored vertical bar that continues down the
/// message body too (see `chat::render_chat`), giving each turn an
/// OpenCode-style "card with a colored gutter" look.
pub fn role_label(role: &str) -> (&'static str, Color) {
    match role {
        "user" => ("▎ user", color::USER),
        "assistant" => ("▎ assistant", color::ASSISTANT),
        "info" => ("▎ system", color::SYSTEM),
        "summary" => ("▎ compacted", color::SYSTEM),
        "tool" => ("▎ tool", color::TOOL),
        _ => ("?", color::FG_DIM),
    }
}
