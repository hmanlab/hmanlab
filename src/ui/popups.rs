//! Overlay surfaces: model picker, session picker, add-model dialog,
//! confirm popup. All four share the `centered_rect` helper.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::{AddModelStep, App, PickerEntry};
use crate::tools;

use super::markdown::{parse_inline_md, wrap_styled_segments};

pub(super) fn render_picker(f: &mut Frame, full: Rect, app: &App) {
    let area = centered_rect(60, 60, full);
    f.render_widget(Clear, area);
    let items: Vec<ListItem> = app
        .picker_entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let label = e.display();
            let entry_color = match e {
                PickerEntry::Ollama(_) => Color::White,
                PickerEntry::Extra(_) => Color::Magenta,
                PickerEntry::AddZaiSubscription
                | PickerEntry::AddZaiUsage
                | PickerEntry::AddOllamaCloud
                | PickerEntry::AddOpenCode => Color::Green,
            };
            let style = if i == app.picker_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(entry_color)
            };
            ListItem::new(format!(" {} ", label)).style(style)
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" select model — ↑↓ Enter Esc "),
    );
    f.render_widget(list, area);
}

pub(super) fn render_disconnect_picker(f: &mut Frame, full: Rect, app: &App) {
    let area = centered_rect(60, 50, full);
    f.render_widget(Clear, area);
    let items: Vec<ListItem> = app
        .disconnect_entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let label = format!(" {} — {} ", e.label, e.preview);
            let style = if i == app.disconnect_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Red)
            };
            ListItem::new(label).style(style)
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title(format!(
                " disconnect provider ({}) — ↑↓ Enter Esc ",
                app.disconnect_entries.len()
            )),
    );
    f.render_widget(list, area);
}

pub(super) fn render_session_picker(f: &mut Frame, full: Rect, app: &App) {
    let area = centered_rect(80, 70, full);
    f.render_widget(Clear, area);
    let items: Vec<ListItem> = app
        .session_picker_items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let id_str = s.id.replace('-', "");
            let short = &id_str[..id_str.len().min(8)];
            let model = s.model.as_deref().unwrap_or("?");
            let title: String = s.title.chars().take(70).collect();
            let label = format!(" {short}  [{model}]  {title} ");
            let style = if i == app.session_picker_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " sessions ({}) — ↑↓ Enter Esc ",
                app.session_picker_items.len()
            )),
    );
    f.render_widget(list, area);
}

pub(super) fn render_add_model(f: &mut Frame, full: Rect, app: &mut App) {
    let area = centered_rect(70, 40, full);
    f.render_widget(Clear, area);

    let (title, body, hint) = match app.add_model_step {
        AddModelStep::Key => {
            let (title, body) = match app.add_model_provider.as_str() {
                p if p == crate::config::OLLAMA_CLOUD_PROVIDER => (
                    " add Ollama Cloud key ".to_string(),
                    "Paste your Ollama Cloud API key (generate one at \
                     https://ollama.com/settings/keys). After saving, the \
                     free-tier models (glm-4.7, gpt-oss:120b-cloud, \
                     qwen3-coder-next) become selectable in /model and you'll \
                     be switched to glm-4.7 by default.\n\n\
                     Note: glm-5.1, glm-5, deepseek, kimi, and minimax are \
                     subscription-only on Ollama Cloud — selecting them \
                     returns a 403 until you upgrade at ollama.com/upgrade.\n\n\
                     The key is stored in ~/.config/hmanlab/config.json (mode \
                     0600) and only sent to ollama.com — never to hmanlab-api."
                        .to_string(),
                ),
                p if p == crate::config::OPENCODE_PROVIDER => (
                    " add OpenCode Go key ".to_string(),
                    "Paste your OpenCode API key (generate one at \
                     https://opencode.ai/zen). This provider points at the \
                     Go subscription endpoint — requests bill against your \
                     Go plan, not pay-per-credit.\n\n\
                     After saving, the Go-tier coding models become \
                     selectable in /model: glm-5.1, glm-5, \
                     qwen3.6/3.5-plus, kimi-k2.5/k2.6, minimax-m2.5/m2.7. \
                     Default is glm-5.1.\n\n\
                     Heads-up: Free-tier models (big-pickle, *-free) live \
                     on Zen's endpoint, not Go's — they're not in this \
                     provider's catalog and would 401 ModelError if added \
                     manually. Closed-weight models (claude-*, gpt-*, \
                     gemini-*) use non-OpenAI wire shapes and aren't routed \
                     through this provider yet.\n\n\
                     The key is stored in ~/.config/hmanlab/config.json (mode \
                     0600) and only sent to opencode.ai — never to hmanlab-api."
                        .to_string(),
                ),
                p if p == crate::config::ZAI_USAGE_PROVIDER => (
                    " add z.ai (usage-based) key ".to_string(),
                    "Paste your z.ai usage-based API key. After saving, all three \
                     z.ai models (glm-4.7, glm-4.6, glm-5.1) become selectable in \
                     /model and you'll be switched to glm-4.7 by default.\n\n\
                     The key is stored in ~/.config/hmanlab/config.json (mode \
                     0600) and only sent to z.ai — never to hmanlab-api."
                        .to_string(),
                ),
                _ => (
                    " add z.ai key ".to_string(),
                    "Paste your z.ai coding-plan API key. After saving, all three \
                     z.ai models (glm-4.7, glm-4.6, glm-5.1) become selectable in \
                     /model and you'll be switched to glm-4.7 by default.\n\n\
                     The key is stored in ~/.config/hmanlab/config.json (mode \
                     0600) and only sent to z.ai — never to hmanlab-api."
                        .to_string(),
                ),
            };
            (title, body, "Enter to save  ·  Esc to cancel")
        }
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .title(title)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    let content_width = (inner.width as usize).saturating_sub(2).max(10);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(2),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    for paragraph in body.split('\n') {
        if paragraph.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        let segments = parse_inline_md(paragraph, Style::default());
        for spans in wrap_styled_segments(segments, content_width) {
            lines.push(Line::from(spans));
        }
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(Color::Gray)),
        chunks[0],
    );

    app.add_model_input.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White))
            .title(" input "),
    );
    f.render_widget(&app.add_model_input, chunks[1]);

    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
        chunks[2],
    );
}

pub(super) fn render_confirm(f: &mut Frame, full: Rect, app: &App) {
    // Bigger popup than before — edit_file/write_file prompts include old/new
    // string previews that easily overflow the 70x40 box the original
    // run_command-style confirm was sized for.
    let area = centered_rect(80, 60, full);
    f.render_widget(Clear, area);

    let prompt = app
        .pending_confirm
        .as_ref()
        .map(|r| r.prompt.as_str())
        .unwrap_or("(no pending request)");

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Confirm action ")
        .padding(Padding::horizontal(1));
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
                    tools::DiffLineKind::Added => Style::default().fg(Color::Green),
                    tools::DiffLineKind::Removed => Style::default().fg(Color::Red),
                    tools::DiffLineKind::Context => {
                        Style::default().fg(Color::DarkGray)
                    }
                    tools::DiffLineKind::Summary => Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                };
                // Wrap each diff line at content_width so long lines stay
                // inside the popup. Re-apply the colour to every wrapped
                // sub-line so a long delete doesn't turn black halfway.
                for chunk in wrap_styled_segments(
                    vec![(dl.text.clone(), style)],
                    content_width,
                ) {
                    lines.push(Line::from(chunk));
                }
            }
        }
    }
    let visible = content_area.height as usize;
    let overflow = lines.len().saturating_sub(visible);
    if overflow > 0 {
        // Mark truncation in the last visible line so the user knows there
        // was more they didn't see — typical for a write_file with a big
        // content blob. The footer remains untouched below.
        let last_idx = visible.saturating_sub(1);
        lines.truncate(last_idx);
        lines.push(Line::from(Span::styled(
            format!("…({} more lines hidden — deny + ask AI to shorten if needed)", overflow),
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )));
    }

    let body = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(body, content_area);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "[y]",
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" allow   "),
        Span::styled(
            "[n]",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" deny   "),
        Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
        Span::raw(" deny"),
    ]));
    f.render_widget(footer, footer_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
