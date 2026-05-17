//! Input event handling — keyboard and mouse — plus the helpers that turn
//! user actions (commands, picker choices, Y/N quick-replies) into chat
//! state changes and outbound agent calls.

use anyhow::Result;
use base64::Engine;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::io::Write;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tui_textarea::Input;

use crate::api::ApiOp;
use crate::config::ExtraModel;
use crate::ollama::{ChatMessage, Client};

use super::inline::{self, FilePopup, InlinePopup, SlashPopup, SLASH_COMMANDS};
use super::{
    fresh_textarea, AddModelStep, App, AppAction, DisconnectEntry, Mode, PickerEntry, StreamMsg,
};

enum Command {
    Model(Option<String>),
    ListModels,
    Clear,
    Quit,
    Help,
    Host(String),
    New,
    ListSessions,
    Load(String),
    More,
    Workspace(String),
    Compact,
    Disconnect(String),
    Update,
    Settings,
    Unknown(String),
}

/// If the current binary's path looks like a cargo-managed install,
/// return a short identifier (the matched path fragment) so `/update`
/// can suggest the right upgrade channel instead of running npm.
fn cargo_install_hint() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let s = exe.to_string_lossy().to_string();
    // `.cargo/bin/hmanlab` covers `cargo install`; `target/release` and
    // `target/debug` cover devs running from a local checkout.
    for needle in [".cargo/bin", "target/release", "target/debug"] {
        if s.contains(needle) {
            return Some(needle.to_string());
        }
    }
    None
}

fn parse_command(text: &str) -> Option<Command> {
    let t = text.trim();
    if !t.starts_with('/') {
        return None;
    }
    let body = &t[1..];
    let (head, rest) = match body.split_once(char::is_whitespace) {
        Some((h, r)) => (h.to_ascii_lowercase(), r.trim().to_string()),
        None => (body.to_ascii_lowercase(), String::new()),
    };
    Some(match head.as_str() {
        "model" | "m" => Command::Model(if rest.is_empty() { None } else { Some(rest) }),
        "models" | "ls" => Command::ListModels,
        "clear" | "cls" | "reset" => Command::Clear,
        "quit" | "exit" | "q" | "bye" => Command::Quit,
        "help" | "?" | "h" => Command::Help,
        "host" | "connect" => Command::Host(rest),
        "new" | "n" => Command::New,
        "sessions" | "history" | "hist" => Command::ListSessions,
        "load" | "open" => Command::Load(rest),
        "more" | "older" => Command::More,
        "workspace" | "ws" | "cwd" => Command::Workspace(rest),
        "compact" | "compress" | "summarize" => Command::Compact,
        "disconnect" | "logout" | "signout" => Command::Disconnect(rest),
        "update" | "upgrade" | "selfupdate" => Command::Update,
        "settings" | "whoami" | "account" | "me" => Command::Settings,
        other => Command::Unknown(other.to_string()),
    })
}

impl App {
    pub async fn handle_event(
        &mut self,
        event: Event,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) -> Result<AppAction> {
        match event {
            Event::Mouse(m) => {
                self.handle_mouse(m);
                Ok(AppAction::Continue)
            }
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return Ok(AppAction::Continue);
                }
                match self.mode {
                    Mode::ModelPicker => Ok(self.handle_picker(key)),
                    Mode::Confirm => Ok(self.handle_confirm(key)),
                    Mode::AddModel => Ok(self.handle_add_model(key)),
                    Mode::SessionPicker => Ok(self.handle_session_picker(key, tx)),
                    Mode::DisconnectPicker => Ok(self.handle_disconnect_picker(key)),
                    Mode::Chat => Ok(self.handle_chat(key, tx)),
                }
            }
            _ => Ok(AppAction::Continue),
        }
    }

    fn handle_mouse(&mut self, m: MouseEvent) {
        if self.mode != Mode::Chat {
            return;
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.sel_start = Some((m.column, m.row));
                self.sel_end = Some((m.column, m.row));
                self.selecting = true;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.selecting => {
                self.sel_end = Some((m.column, m.row));
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if !self.selecting {
                    return;
                }
                self.selecting = false;
                // No drag → treat as a click. Sidebar clicks open files;
                // chat clicks toggle the tool block under the cursor.
                if self.sel_start == self.sel_end {
                    if let Some((col, row)) = self.sel_start {
                        if self.point_in_sidebar(col, row) {
                            self.try_open_sidebar_at(row);
                        } else {
                            let in_chat_col =
                                col >= self.chat_x && col < self.chat_x.saturating_add(self.chat_w);
                            if in_chat_col && self.open_file.is_none() {
                                self.try_toggle_tool_at(row);
                            }
                        }
                    }
                    self.sel_start = None;
                    self.sel_end = None;
                } else {
                    self.copy_selection_to_clipboard();
                }
            }
            MouseEventKind::ScrollUp => {
                // Route wheel events to whichever panel the cursor is over.
                if self.point_in_sidebar(m.column, m.row) {
                    self.sidebar_scroll = self.sidebar_scroll.saturating_sub(3);
                } else if let Some(file) = self.open_file.as_mut() {
                    file.scroll = file.scroll.saturating_sub(3);
                } else {
                    self.follow = false;
                    self.scroll = self.scroll.saturating_sub(3);
                }
            }
            MouseEventKind::ScrollDown => {
                if self.point_in_sidebar(m.column, m.row) {
                    // The renderer clamps to a valid max each frame, so a
                    // saturating add here is enough — wheel-past-end stops
                    // cleanly without us computing max_scroll up-front.
                    self.sidebar_scroll = self.sidebar_scroll.saturating_add(3);
                } else if let Some(file) = self.open_file.as_mut() {
                    file.scroll = file.scroll.saturating_add(3);
                } else {
                    let prev = self.scroll;
                    self.scroll = self.scroll.saturating_add(3);
                    if self.scroll == prev {
                        self.follow = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn point_in_sidebar(&self, col: u16, row: u16) -> bool {
        self.sidebar_w != 0
            && col >= self.sidebar_x
            && col < self.sidebar_x.saturating_add(self.sidebar_w)
            && row >= self.sidebar_y
            && row < self.sidebar_y.saturating_add(self.sidebar_h)
    }

    /// Resolve a sidebar click: directories toggle expanded state, files
    /// open in the viewer. `screen_y` is the raw row from the mouse event;
    /// we convert it to a logical line via `(screen_y - sidebar_y) +
    /// sidebar_scroll` before looking up the entry, so scrolled-out rows
    /// resolve correctly once they're brought back into view.
    fn try_open_sidebar_at(&mut self, screen_y: u16) {
        if screen_y < self.sidebar_y {
            return;
        }
        let logical_y = (screen_y - self.sidebar_y).saturating_add(self.sidebar_scroll);
        let hit = self
            .sidebar_targets
            .iter()
            .find(|(row, _, _)| *row == logical_y)
            .map(|(_, path, is_dir)| (path.clone(), *is_dir));
        match hit {
            // Toggle expand/collapse. The next render uses the updated
            // set; the walker re-builds the tree from scratch each frame.
            // Collapsing the inner `if` into a match guard would push the
            // side-effecting `remove` into the guard expression AND force
            // an extra catch-all arm to keep exhaustiveness, so allow the
            // lint here.
            #[allow(clippy::collapsible_match)]
            Some((path, true)) => {
                if !self.expanded_dirs.remove(&path) {
                    self.expanded_dirs.insert(path);
                }
            }
            Some((path, false)) => {
                self.open_workspace_file(path);
            }
            None => {}
        }
    }

    /// Key routing while the file viewer is open. Esc dismisses; arrow /
    /// page / home / end keys move through the file. Everything else is
    /// swallowed so the chat input doesn't pick up stray characters and the
    /// user can't accidentally fire a command (e.g. Ctrl+N) while reading.
    fn handle_viewer_key(&mut self, key: KeyEvent) -> AppAction {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(file) = self.open_file.as_mut() else {
            return AppAction::Continue;
        };
        match key.code {
            KeyCode::Esc => {
                self.open_file = None;
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                file.scroll = file.scroll.saturating_add(10);
            }
            KeyCode::PageUp => {
                file.scroll = file.scroll.saturating_sub(10);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                file.scroll = file.scroll.saturating_add(1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                file.scroll = file.scroll.saturating_sub(1);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                file.scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                file.scroll = u16::MAX;
            }
            // Ctrl+C remains an escape hatch so the user can always close.
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_file = None;
            }
            _ => {}
        }
        AppAction::Continue
    }

    fn try_toggle_tool_at(&mut self, screen_y: u16) {
        let top = self.chat_y;
        let bottom = top.saturating_add(self.chat_h);
        if screen_y < top || screen_y >= bottom {
            return;
        }
        let logical_y = self.scroll.saturating_add(screen_y - top);
        for &(idx, ls, le) in &self.message_line_ranges {
            if logical_y >= ls && logical_y < le {
                if let Some(msg) = self.messages.get(idx) {
                    if msg.role == "tool" && !self.expanded_tools.remove(&idx) {
                        self.expanded_tools.insert(idx);
                    }
                }
                return;
            }
        }
    }

    fn copy_selection_to_clipboard(&mut self) {
        let (Some(start), Some(end)) = (self.sel_start, self.sel_end) else {
            return;
        };
        // Normalize so start is the top-left, end is the bottom-right.
        let ((sx, sy), (ex, ey)) = if (start.1, start.0) <= (end.1, end.0) {
            (start, end)
        } else {
            (end, start)
        };
        let cx = self.chat_x;
        let cy = self.chat_y;
        let cw = self.chat_w;
        let ch = self.chat_h;
        if cw == 0 || ch == 0 {
            self.sel_start = None;
            self.sel_end = None;
            return;
        }
        let row_lo = sy.max(cy);
        let row_hi = ey.min(cy.saturating_add(ch).saturating_sub(1));
        if row_lo > row_hi {
            self.sel_start = None;
            self.sel_end = None;
            return;
        }

        let mut lines_out: Vec<String> = Vec::new();
        for screen_y in row_lo..=row_hi {
            let logical_idx = (self.scroll as usize) + ((screen_y - cy) as usize);
            let line = self
                .rendered_text_lines
                .get(logical_idx)
                .map(|s| s.as_str())
                .unwrap_or("");
            let row_start_col = if screen_y == sy {
                (sx.saturating_sub(cx)) as usize
            } else {
                0
            };
            let row_end_col = if screen_y == ey {
                (ex.saturating_sub(cx)) as usize + 1
            } else {
                usize::MAX
            };
            let chars: Vec<char> = line.chars().collect();
            let s = row_start_col.min(chars.len());
            let e = row_end_col.min(chars.len());
            let piece: String = if s < e {
                chars[s..e].iter().collect()
            } else {
                String::new()
            };
            lines_out.push(piece.trim_end().to_string());
        }
        while lines_out.last().is_some_and(|l| l.is_empty()) {
            lines_out.pop();
        }
        let buf: String = lines_out.join("\n");
        self.sel_start = None;
        self.sel_end = None;
        if buf.is_empty() {
            return;
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(buf.as_bytes());
        let mut out = std::io::stdout();
        let _ = write!(out, "\x1b]52;c;{}\x07", encoded);
        let _ = out.flush();
        self.status = format!("Copied {} chars to clipboard", buf.len());
    }

    fn toggle_all_tools(&mut self) {
        // Toggles two fold categories together: tool result blocks AND the
        // assistant `<think>` reasoning blocks. Either one being expanded
        // counts as "expanded" — collapse them all; otherwise expand them all.
        let tool_indices: Vec<usize> = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "tool")
            .map(|(i, _)| i)
            .collect();
        let thought_indices: Vec<usize> = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "assistant" && m.content.contains("</think>"))
            .map(|(i, _)| i)
            .collect();
        let any_expanded = !self.expanded_tools.is_empty() || !self.expanded_thoughts.is_empty();
        if any_expanded {
            self.expanded_tools.clear();
            self.expanded_thoughts.clear();
            self.status = "Folds collapsed".into();
        } else {
            self.expanded_tools = tool_indices.into_iter().collect();
            self.expanded_thoughts = thought_indices.into_iter().collect();
            self.status = "Folds expanded".into();
        }
    }

    fn handle_session_picker(
        &mut self,
        key: KeyEvent,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Chat;
                self.status = "Cancelled".into();
            }
            KeyCode::Up | KeyCode::Char('k') if self.session_picker_index > 0 => {
                self.session_picker_index -= 1;
            }
            KeyCode::Down | KeyCode::Char('j')
                if self.session_picker_index + 1 < self.session_picker_items.len() =>
            {
                self.session_picker_index += 1;
            }
            KeyCode::Enter => {
                if let Some(s) = self
                    .session_picker_items
                    .get(self.session_picker_index)
                    .cloned()
                {
                    self.mode = Mode::Chat;
                    // Reuse the existing load-by-prefix path: just pass the
                    // full id so load_session resolves it cleanly.
                    self.load_session(s.id, tx);
                }
            }
            _ => {}
        }
        AppAction::Continue
    }

    fn handle_picker(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Chat,
            KeyCode::Up | KeyCode::Char('k') if self.picker_index > 0 => {
                self.picker_index -= 1;
            }
            KeyCode::Down | KeyCode::Char('j')
                if self.picker_index + 1 < self.picker_entries.len() =>
            {
                self.picker_index += 1;
            }
            KeyCode::Enter => {
                if let Some(entry) = self.picker_entries.get(self.picker_index).cloned() {
                    match entry {
                        PickerEntry::Ollama(name) => {
                            self.model = name.clone();
                            self.selected_extra = None;
                            self.status = format!("Switched to {}", name);
                            self.mode = Mode::Chat;
                        }
                        PickerEntry::Extra(m) => {
                            self.model = m.name.clone();
                            self.status = format!("Switched to [{}] {}", m.provider, m.name);
                            self.selected_extra = Some(m);
                            self.mode = Mode::Chat;
                        }
                        PickerEntry::AddZaiSubscription => {
                            self.begin_add_model(crate::config::ZAI_SUBSCRIPTION_PROVIDER);
                        }
                        PickerEntry::AddZaiUsage => {
                            self.begin_add_model(crate::config::ZAI_USAGE_PROVIDER);
                        }
                        PickerEntry::AddOllamaCloud => {
                            self.begin_add_model(crate::config::OLLAMA_CLOUD_PROVIDER);
                        }
                        PickerEntry::AddOpenCode => {
                            self.begin_add_model(crate::config::OPENCODE_PROVIDER);
                        }
                    }
                } else {
                    self.mode = Mode::Chat;
                }
            }
            _ => {}
        }
        AppAction::Continue
    }

    fn handle_confirm(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                if let Some(req) = self.pending_confirm.take() {
                    let _ = req.responder.send(true);
                    self.push_info(format!("✓ Allowed: {}", req.prompt));
                }
                self.mode = Mode::Chat;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                if let Some(req) = self.pending_confirm.take() {
                    let _ = req.responder.send(false);
                    self.push_info(format!("✗ Denied: {}", req.prompt));
                }
                self.mode = Mode::Chat;
            }
            _ => {}
        }
        AppAction::Continue
    }

    fn handle_chat(&mut self, key: KeyEvent, tx: &mpsc::UnboundedSender<StreamMsg>) -> AppAction {
        // File-viewer overlay takes over input while open. Esc closes it,
        // scroll keys page through the file, everything else is swallowed
        // so the chat input stays inert until the user dismisses the file.
        if self.open_file.is_some() {
            return self.handle_viewer_key(key);
        }

        // Inline autocomplete popup (slash / @ mention) intercept — must
        // come before the global Esc, Up/Down scroll, and Enter-submit
        // handlers so the popup gets first dibs on its own navigation keys.
        if self.inline_popup.is_open() && key.modifiers.is_empty() {
            match key.code {
                KeyCode::Esc => {
                    self.inline_popup = InlinePopup::None;
                    return AppAction::Continue;
                }
                KeyCode::Up => {
                    self.inline_popup_select_prev();
                    return AppAction::Continue;
                }
                KeyCode::Down => {
                    self.inline_popup_select_next();
                    return AppAction::Continue;
                }
                KeyCode::Tab => {
                    self.inline_popup_complete();
                    return AppAction::Continue;
                }
                KeyCode::Enter => {
                    // Completion-on-Enter — popup absorbs Enter, user
                    // presses Enter again to actually submit. Matches the
                    // shell-autocomplete convention.
                    self.inline_popup_complete();
                    return AppAction::Continue;
                }
                _ => {}
            }
        }

        // Y/N quick-reply intercept — only when the AI just asked a yes/no
        // question and the input is empty. Plain key, no modifiers.
        if self.yn_pending
            && !self.generating
            && key.modifiers.is_empty()
            && self.input.lines().iter().all(|l| l.is_empty())
        {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.yn_pending = false;
                    self.awaiting_yn_followup = true;
                    self.inject_hidden_user("yes", tx);
                    return AppAction::Continue;
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.yn_pending = false;
                    self.inject_hidden_user("no, that's enough", tx);
                    return AppAction::Continue;
                }
                KeyCode::Esc => {
                    self.yn_pending = false;
                    return AppAction::Continue;
                }
                // Any other plain key dismisses the hint and falls through
                // to the normal input pipeline below.
                _ => {
                    self.yn_pending = false;
                }
            }
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => return AppAction::Quit,
                KeyCode::Char('c') => {
                    if self.generating {
                        self.cancel();
                    } else {
                        return AppAction::Quit;
                    }
                    return AppAction::Continue;
                }
                KeyCode::Char('m') => {
                    self.open_picker();
                    return AppAction::Continue;
                }
                KeyCode::Char('l') => {
                    self.clear_history();
                    return AppAction::Continue;
                }
                KeyCode::Char('n') => {
                    self.new_session();
                    return AppAction::Continue;
                }
                KeyCode::Char('t') => {
                    self.toggle_all_tools();
                    return AppAction::Continue;
                }
                _ => {}
            }
        }

        match key.code {
            // Esc never quits — too easy to hit mid-conversation. Priority
            // order: interrupt an in-flight generation, then clear a draft,
            // then no-op. Quit stays available via /quit, /exit, Ctrl+Q,
            // and Ctrl+C (when nothing's running).
            KeyCode::Esc => {
                if self.generating {
                    self.cancel();
                } else {
                    let has_text = self.input.lines().iter().any(|l| !l.is_empty());
                    if has_text {
                        self.reset_input();
                        self.inline_popup = InlinePopup::None;
                    }
                }
                return AppAction::Continue;
            }
            KeyCode::PageUp => {
                self.follow = false;
                self.scroll = self.scroll.saturating_sub(5);
                return AppAction::Continue;
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(5);
                return AppAction::Continue;
            }
            // Wheel scroll arrives as Up/Down arrow keys (DEC 1007). Route to chat scroll
            // when the input is a single line so multi-line composition still uses
            // arrows for cursor movement.
            KeyCode::Up if self.input.lines().len() <= 1 => {
                self.follow = false;
                self.scroll = self.scroll.saturating_sub(3);
                return AppAction::Continue;
            }
            KeyCode::Down if self.input.lines().len() <= 1 => {
                let prev = self.scroll;
                self.scroll = self.scroll.saturating_add(3);
                if self.scroll == prev {
                    self.follow = true;
                }
                return AppAction::Continue;
            }
            KeyCode::Home => {
                self.follow = false;
                self.scroll = 0;
                return AppAction::Continue;
            }
            KeyCode::End => {
                self.follow = true;
                return AppAction::Continue;
            }
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                if !self.generating {
                    return self.submit(tx);
                }
                return AppAction::Continue;
            }
            _ => {}
        }

        let input: Input = key.into();
        self.input.input(input);
        // After the textarea consumed the key, re-evaluate whether a `/`
        // slash command or `@` file mention is being typed and surface /
        // dismiss the popup accordingly.
        self.update_inline_popup();
        AppAction::Continue
    }

    /// Refresh `inline_popup` based on the current input state. Called
    /// after every key the textarea consumed.
    fn update_inline_popup(&mut self) {
        let first = self.input.lines().first().cloned().unwrap_or_default();
        let (row, col) = self.input.cursor();
        if row != 0 {
            self.inline_popup = InlinePopup::None;
            return;
        }
        match inline::detect_trigger(&first, col) {
            Some(('/', filter)) => match &mut self.inline_popup {
                InlinePopup::Slash(p) => p.set_filter(filter),
                _ => self.inline_popup = InlinePopup::Slash(SlashPopup::new(filter)),
            },
            Some(('@', filter)) => match &mut self.inline_popup {
                InlinePopup::File(p) => p.set_filter(filter),
                _ => {
                    self.inline_popup = InlinePopup::File(FilePopup::new(filter, &self.workspace));
                }
            },
            _ => {
                self.inline_popup = InlinePopup::None;
            }
        }
    }

    fn inline_popup_select_prev(&mut self) {
        match &mut self.inline_popup {
            InlinePopup::Slash(p) => {
                if p.index > 0 {
                    p.index -= 1;
                }
            }
            InlinePopup::File(p) => {
                if p.index > 0 {
                    p.index -= 1;
                }
            }
            InlinePopup::None => {}
        }
    }

    fn inline_popup_select_next(&mut self) {
        match &mut self.inline_popup {
            InlinePopup::Slash(p) => {
                if p.index + 1 < p.matches.len() {
                    p.index += 1;
                }
            }
            InlinePopup::File(p) => {
                if p.index + 1 < p.matches.len() {
                    p.index += 1;
                }
            }
            InlinePopup::None => {}
        }
    }

    /// Insert the currently-highlighted completion into the input at the
    /// trigger position, replacing the partial filter the user typed.
    /// Closes the popup either way.
    ///
    /// Subtlety: tui-textarea's `delete_str(n)` deletes n chars **after**
    /// the cursor, not before, so we jump the cursor to the start of the
    /// range we want to replace, delete forward, then insert.
    fn inline_popup_complete(&mut self) {
        let (row, col) = self.input.cursor();
        if row != 0 {
            self.inline_popup = InlinePopup::None;
            return;
        }
        match &self.inline_popup {
            InlinePopup::Slash(p) => {
                if let Some(&idx) = p.matches.get(p.index) {
                    let cmd = SLASH_COMMANDS[idx].name;
                    // Slash trigger always starts at column 0 — jump there
                    // and delete the `/<filter>` we're replacing.
                    self.input.move_cursor(tui_textarea::CursorMove::Jump(0, 0));
                    self.input.delete_str(col);
                    self.input.insert_str(format!("/{cmd} "));
                }
            }
            InlinePopup::File(p) => {
                if let Some(&idx) = p.matches.get(p.index) {
                    let path = p.workspace_files[idx].to_string_lossy().to_string();
                    let first = self.input.lines().first().cloned().unwrap_or_default();
                    let head_len = col.min(first.chars().count());
                    let head: String = first.chars().take(head_len).collect();
                    if let Some(at_byte) = head.rfind('@') {
                        let at_char_pos = head[..at_byte].chars().count();
                        let to_delete = head_len.saturating_sub(at_char_pos);
                        self.input
                            .move_cursor(tui_textarea::CursorMove::Jump(0, at_char_pos as u16));
                        self.input.delete_str(to_delete);
                        self.input.insert_str(format!("@{path} "));
                    }
                }
            }
            InlinePopup::None => {}
        }
        self.inline_popup = InlinePopup::None;
    }

    fn submit(&mut self, tx: &mpsc::UnboundedSender<StreamMsg>) -> AppAction {
        let text = self.input.lines().join("\n").trim().to_string();
        if text.is_empty() {
            return AppAction::Continue;
        }
        self.yn_pending = false;
        self.awaiting_yn_followup = false;
        self.reset_input();

        if let Some(cmd) = parse_command(&text) {
            return self.handle_command(cmd, tx);
        }

        self.send_to_llm(text, tx);
        AppAction::Continue
    }

    fn handle_command(&mut self, cmd: Command, tx: &mpsc::UnboundedSender<StreamMsg>) -> AppAction {
        match cmd {
            Command::Model(None) => self.open_picker(),
            Command::Model(Some(name)) => self.switch_model(&name),
            Command::ListModels => self.list_models_inline(),
            Command::Clear => self.clear_history(),
            Command::Quit => return AppAction::Quit,
            Command::Help => self.show_help_inline(),
            Command::Host(url) => self.switch_host(url, tx),
            Command::New => self.new_session(),
            Command::ListSessions => self.list_sessions_inline(tx),
            Command::Load(prefix) => self.load_session(prefix, tx),
            Command::More => self.load_more(tx),
            Command::Workspace(path) => self.switch_workspace(path),
            Command::Compact => self.start_compact(tx, None),
            Command::Disconnect(name) => self.handle_disconnect(&name),
            Command::Update => self.start_update(tx),
            Command::Settings => self.show_settings(tx),
            Command::Unknown(name) => {
                self.push_info(format!(
                    "Unknown command: /{name}\nType /help to see available commands."
                ));
                self.status = format!("Unknown: /{name}");
            }
        }
        AppAction::Continue
    }

    fn rebuild_picker_entries(&mut self) {
        let mut entries: Vec<PickerEntry> = self
            .models
            .iter()
            .map(|m| PickerEntry::Ollama(m.clone()))
            .collect();
        for em in &self.extra_models {
            entries.push(PickerEntry::Extra(em.clone()));
        }
        // Each provider's "+ Add … key" row only appears while that provider
        // is unconfigured. Once a key is saved, the corresponding models
        // already show up in the section above via `extra_models`.
        if self.zai_api_key.is_none() {
            entries.push(PickerEntry::AddZaiSubscription);
        }
        if self.zai_usage_api_key.is_none() {
            entries.push(PickerEntry::AddZaiUsage);
        }
        if self.ollama_cloud_api_key.is_none() {
            entries.push(PickerEntry::AddOllamaCloud);
        }
        if self.opencode_api_key.is_none() {
            entries.push(PickerEntry::AddOpenCode);
        }
        self.picker_entries = entries;
    }

    fn open_picker(&mut self) {
        if self.models.is_empty() && self.extra_models.is_empty() {
            self.push_info(
                "No models yet. Connect Ollama with /host <url>, or add a BYOK model from the picker.".into(),
            );
        }
        self.rebuild_picker_entries();
        self.mode = Mode::ModelPicker;
        // Cursor on the currently active model if it's in the list, else 0.
        // For Extras we match BOTH provider + name so that picking the same
        // model under a different plan (e.g. glm-4.7 on usage vs subscription)
        // doesn't land the cursor on the wrong row.
        let active_provider = self.selected_extra.as_ref().map(|e| e.provider.as_str());
        self.picker_index = self
            .picker_entries
            .iter()
            .position(|e| match e {
                PickerEntry::Ollama(n) => active_provider.is_none() && n == &self.model,
                PickerEntry::Extra(m) => {
                    active_provider.is_some_and(|p| p == m.provider) && m.name == self.model
                }
                PickerEntry::AddZaiSubscription
                | PickerEntry::AddZaiUsage
                | PickerEntry::AddOllamaCloud
                | PickerEntry::AddOpenCode => false,
            })
            .unwrap_or(0);
    }

    /// Begin the "+ Add … key" flow for the given provider. Single step:
    /// collect the API key. After save, all known models for that provider
    /// become available in the picker.
    fn begin_add_model(&mut self, provider: &str) {
        self.add_model_provider = provider.to_string();
        self.add_model_step = AddModelStep::Key;
        self.add_model_input = fresh_textarea();
        let (placeholder, label) = match provider {
            p if p == crate::config::ZAI_USAGE_PROVIDER => {
                ("Paste your z.ai usage-based API key", "z.ai usage-based")
            }
            p if p == crate::config::OLLAMA_CLOUD_PROVIDER => (
                "Paste your Ollama Cloud API key (from https://ollama.com/settings/keys)",
                "Ollama Cloud",
            ),
            p if p == crate::config::OPENCODE_PROVIDER => (
                "Paste your OpenCode API key (from https://opencode.ai/zen)",
                "OpenCode",
            ),
            _ => ("Paste your z.ai coding-plan API key", "z.ai subscription"),
        };
        self.add_model_input.set_placeholder_text(placeholder);
        self.mode = Mode::AddModel;
        self.status = format!("Adding {label} key — Esc to cancel");
    }

    fn handle_add_model(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Chat;
                self.status = "Add model cancelled".into();
                return AppAction::Continue;
            }
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                let val = self.add_model_input.lines().join("").trim().to_string();
                if val.is_empty() {
                    return AppAction::Continue;
                }
                let provider = self.add_model_provider.clone();
                // `default_model` is the model the active selection switches
                // to after saving — chosen per-provider so the user can chat
                // immediately without re-opening the picker.
                let (label, default_model) = match provider.as_str() {
                    p if p == crate::config::ZAI_USAGE_PROVIDER => {
                        self.zai_usage_api_key = Some(val);
                        self.ensure_zai_models_for(&provider);
                        ("z.ai usage-based", crate::config::ZAI_DEFAULT_MODEL)
                    }
                    p if p == crate::config::OLLAMA_CLOUD_PROVIDER => {
                        self.ollama_cloud_api_key = Some(val);
                        self.ensure_ollama_cloud_models();
                        ("Ollama Cloud", crate::config::OLLAMA_CLOUD_DEFAULT_MODEL)
                    }
                    p if p == crate::config::OPENCODE_PROVIDER => {
                        self.opencode_api_key = Some(val);
                        self.ensure_opencode_models();
                        ("OpenCode", crate::config::OPENCODE_DEFAULT_MODEL)
                    }
                    _ => {
                        self.zai_api_key = Some(val);
                        self.ensure_zai_models_for(&provider);
                        ("z.ai subscription", crate::config::ZAI_DEFAULT_MODEL)
                    }
                };
                self.persist_config();
                let name = default_model.to_string();
                let target_extra = self
                    .extra_models
                    .iter()
                    .find(|m| m.provider == provider && m.name == name)
                    .cloned();
                self.model = name;
                self.selected_extra = target_extra;
                self.mode = Mode::Chat;
                self.status = format!("{label} key saved · using {}", self.model);
                return AppAction::Continue;
            }
            _ => {}
        }
        let input: Input = key.into();
        self.add_model_input.input(input);
        AppAction::Continue
    }

    fn switch_model(&mut self, name: &str) {
        let lower = name.to_lowercase();
        let exact = self
            .models
            .iter()
            .find(|m| m.eq_ignore_ascii_case(name))
            .cloned();
        let partial: Vec<String> = self
            .models
            .iter()
            .filter(|m| m.to_lowercase().contains(&lower))
            .cloned()
            .collect();
        let chosen = if let Some(e) = exact {
            Some(e)
        } else if partial.len() == 1 {
            Some(partial[0].clone())
        } else if partial.is_empty() {
            let available = if self.models.is_empty() {
                "(none — connect with /host <url>)".into()
            } else {
                self.models.join("\n  ")
            };
            self.push_info(format!(
                "No model matches '{name}'. Available:\n  {available}"
            ));
            self.status = format!("No match: {name}");
            None
        } else {
            self.push_info(format!(
                "Multiple matches for '{name}':\n  {}",
                partial.join("\n  ")
            ));
            self.status = format!("Ambiguous: {name}");
            None
        };
        if let Some(m) = chosen {
            self.model = m;
            // /model <name> only matches Ollama-discovered models, so clear
            // any active extra. To switch to an extra-provider model, use
            // the picker (Ctrl+M).
            self.selected_extra = None;
            self.push_info(format!("Switched to model: {}", self.model));
            self.status = format!("Model: {}", self.model);
        }
    }

    fn list_models_inline(&mut self) {
        if self.models.is_empty() {
            self.push_info("No models. Try /host <url> first.".into());
            return;
        }
        let list: Vec<String> = self
            .models
            .iter()
            .map(|m| {
                if m == &self.model {
                    format!("  * {m}  (current)")
                } else {
                    format!("    {m}")
                }
            })
            .collect();
        self.push_info(format!(
            "Available models ({}):\n{}",
            self.models.len(),
            list.join("\n")
        ));
    }

    fn clear_history(&mut self) {
        self.messages.clear();
        self.expanded_tools.clear();
        self.expanded_thoughts.clear();
        self.total_prompt_tokens = 0;
        self.total_completion_tokens = 0;
        self.last_prompt_tokens = 0;
        self.pending_after_compact = None;
        self.scroll = 0;
        self.follow = true;
        self.status = "History cleared (current session continues)".into();
    }

    fn new_session(&mut self) {
        self.messages.clear();
        self.expanded_tools.clear();
        self.expanded_thoughts.clear();
        self.total_prompt_tokens = 0;
        self.total_completion_tokens = 0;
        self.last_prompt_tokens = 0;
        self.pending_after_compact = None;
        self.scroll = 0;
        self.follow = true;
        self.loaded_session_id = None;
        self.oldest_loaded_msg_id = None;
        if let Some(tx) = &self.api_tx {
            let _ = tx.send(ApiOp::EndSession);
            self.push_info("New session started. Previous chat saved.".into());
        } else {
            self.push_info("New session started (not persisted — API off).".into());
        }
        self.status = "New session".into();
    }

    fn switch_workspace(&mut self, path: String) {
        let path = path.trim();
        if path.is_empty() {
            self.push_info(format!(
                "Current workspace: {}\nUsage: /workspace <path>",
                self.workspace.display()
            ));
            return;
        }
        let candidate = PathBuf::from(path);
        let canonical = match candidate.canonicalize() {
            Ok(p) if p.is_dir() => p,
            Ok(_) => {
                self.push_info(format!("Not a directory: {path}"));
                return;
            }
            Err(e) => {
                self.push_info(format!("Cannot use '{path}': {e}"));
                return;
            }
        };
        self.workspace = canonical;
        // Sidebar state belongs to the previous workspace — reset it so the
        // new workspace gets its own top-level expansion and a fresh scroll
        // position. Without this, the picker would still hold paths from
        // the old tree that no longer exist in the new one.
        self.seed_sidebar_top_level();
        self.push_info(format!("Workspace: {}", self.workspace.display()));
        self.status = format!("Workspace: {}", self.workspace.display());
    }

    fn list_sessions_inline(&mut self, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let Some(client) = self.api.clone() else {
            self.push_info(
                "API is off — set HMANLAB_API_KEY or pass --api-key to enable persistence.".into(),
            );
            return;
        };
        let tx = tx.clone();
        tokio::spawn(async move {
            match client.list_sessions(20).await {
                Ok(rows) => {
                    let _ = tx.send(StreamMsg::SessionList(rows));
                }
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(format!("list sessions: {e}")));
                }
            }
        });
    }

    fn load_session(&mut self, prefix: String, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let Some(client) = self.api.clone() else {
            self.push_info("API is off — cannot load saved sessions.".into());
            return;
        };
        if prefix.trim().is_empty() {
            self.push_info("Usage: /load <id-prefix>  (run /sessions for the list)".into());
            return;
        }
        let tx = tx.clone();
        tokio::spawn(async move {
            let session = match client.find_session_by_prefix(&prefix).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(format!("load: {e}")));
                    return;
                }
            };
            match client.load_recent_messages(&session.id, 10).await {
                Ok(messages) => {
                    let _ = tx.send(StreamMsg::Loaded { session, messages });
                }
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(format!("load messages: {e}")));
                }
            }
        });
    }

    fn load_more(&mut self, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let Some(client) = self.api.clone() else {
            self.push_info("API is off — nothing to page.".into());
            return;
        };
        let Some(session_id) = self.loaded_session_id.clone() else {
            self.push_info(
                "/more works inside a /load'd session. Run /sessions and /load <id> first.".into(),
            );
            return;
        };
        let Some(before_id) = self.oldest_loaded_msg_id else {
            self.push_info("No more messages to load.".into());
            return;
        };
        self.status = "Loading older messages…".into();
        let tx = tx.clone();
        tokio::spawn(async move {
            match client.load_older_messages(&session_id, before_id, 10).await {
                Ok(messages) => {
                    let _ = tx.send(StreamMsg::MoreLoaded { messages });
                }
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(format!("/more: {e}")));
                }
            }
        });
    }

    /// `/settings` — show what the user has set: hmanlab version, active
    /// model, Ollama host, configured BYOK providers (presence only,
    /// never the key), workspace, plus the authenticated user's profile.
    /// The profile + latest-version look-up run in the background — the
    /// prompt returns instantly with the locally-known fields and the
    /// account block fills in when the request resolves.
    ///
    /// Backend URL / "where this came from" is intentionally not shown —
    /// users care about their account and configuration, not plumbing.
    fn show_settings(&mut self, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let current = env!("CARGO_PKG_VERSION");
        let mut byok = Vec::new();
        if self.zai_api_key.is_some() {
            byok.push("z.ai (subscription)");
        }
        if self.zai_usage_api_key.is_some() {
            byok.push("z.ai (usage)");
        }
        if self.ollama_cloud_api_key.is_some() {
            byok.push("Ollama Cloud");
        }
        if self.opencode_api_key.is_some() {
            byok.push("OpenCode");
        }
        let byok_line = if byok.is_empty() {
            "none".to_string()
        } else {
            byok.join(", ")
        };
        let upstream = self.update_available.as_deref();
        let version_line = match upstream {
            Some(latest) if crate::update_check::newer(current, latest) => {
                format!("{current}  (npm has {latest} — run /update)")
            }
            _ => current.to_string(),
        };
        let local = format!(
            "Settings\n\
             \x20 hmanlab version  : {version_line}\n\
             \x20 model            : {model}\n\
             \x20 ollama host      : {host}\n\
             \x20 BYOK providers   : {byok_line}\n\
             \x20 workspace        : {ws}\n\
             \n\
             Account: loading…",
            model = self.model,
            host = self.client.base,
            ws = self.workspace.display(),
        );
        self.push_info(local);
        self.status = "Loading account info…".into();

        let Some(api) = self.api.clone() else {
            // No auth client → nothing to fetch. The local block above
            // already covers everything we can show.
            return;
        };
        let current_owned = current.to_string();
        let tx = tx.clone();
        tokio::spawn(async move {
            let me = api.fetch_me().await;
            let latest = crate::update_check::fetch_latest_npm().await.ok();
            let account = match me {
                Ok(me) => {
                    let name = me.name.as_deref().unwrap_or("(no display name set)");
                    let admin = if me.is_admin { " · admin" } else { "" };
                    let opt = if me.training_opt_in {
                        "opted in"
                    } else {
                        "opted out"
                    };
                    format!(
                        "Account\n\
                         \x20 name             : {name}{admin}\n\
                         \x20 email            : {email}\n\
                         \x20 training data    : {opt}",
                        email = me.email,
                    )
                }
                Err(_) => "Account\n\x20 (could not load — try /settings again later)".to_string(),
            };
            let version_tail = match latest {
                Some(l) if crate::update_check::newer(&current_owned, &l) => {
                    format!("\n\nnpm latest: {l} — run /update to install.")
                }
                Some(l) => format!("\n\nnpm latest: {l} (you're up to date)."),
                None => String::new(),
            };
            let _ = tx.send(StreamMsg::Settings(format!("{account}{version_tail}")));
        });
    }

    /// `/update` — shell out to `npm install -g hmanlab@latest` in the
    /// background and report the outcome inline. The currently running
    /// process keeps serving the chat; npm replaces the on-disk binary,
    /// and the user picks it up on next launch.
    ///
    /// If the binary was installed via cargo (path under `.cargo/bin` or
    /// a `target/` build dir), we don't even try npm — surface the right
    /// `cargo install` command instead so the user upgrades through the
    /// channel they actually used.
    fn start_update(&mut self, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let current = env!("CARGO_PKG_VERSION");

        if let Some(hint) = cargo_install_hint() {
            self.push_info(format!(
                "hmanlab looks like a cargo install ({hint}).\n\
                 Run this in another terminal to upgrade:\n\
                 \x20 cargo install hmanlab --force"
            ));
            self.status = "Cargo install detected — see message".into();
            return;
        }

        self.push_info(format!(
            "Checking npm for a newer hmanlab (current {current})…"
        ));
        self.status = "Checking latest version…".into();

        let tx = tx.clone();
        let current_owned = current.to_string();
        tokio::spawn(async move {
            // Step 1: ask npm what's published. If the lookup fails we still
            // proceed to install — the user explicitly asked, and a flaky
            // registry shouldn't block them. If it succeeds and the current
            // version is already latest, bail out without spawning npm.
            match crate::update_check::fetch_latest_npm().await {
                Ok(latest) if !crate::update_check::newer(&current_owned, &latest) => {
                    let _ = tx.send(StreamMsg::UpdateResult {
                        ok: true,
                        text: format!(
                            "Already up to date — hmanlab {current_owned} matches the latest \
                             on npm ({latest}). No install needed."
                        ),
                    });
                    return;
                }
                Ok(latest) => {
                    let _ = tx.send(StreamMsg::UpdateInfo(format!(
                        "Update available: {current_owned} → {latest}. \
                         Running: npm install -g hmanlab@latest"
                    )));
                }
                Err(e) => {
                    let _ = tx.send(StreamMsg::UpdateInfo(format!(
                        "Couldn't reach npm registry ({e}). Trying install anyway…"
                    )));
                }
            }

            let result = tokio::process::Command::new("npm")
                .args(["install", "-g", "hmanlab@latest"])
                .output()
                .await;
            let msg = match result {
                Ok(out) if out.status.success() => StreamMsg::UpdateResult {
                    ok: true,
                    text: "Update complete. Restart hmanlab to use the new version.".into(),
                },
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let tail = stderr.lines().rev().take(8).collect::<Vec<_>>();
                    let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
                    StreamMsg::UpdateResult {
                        ok: false,
                        text: format!(
                            "npm install failed (exit {}).\n{}",
                            out.status.code().unwrap_or(-1),
                            if tail.is_empty() {
                                "No stderr output.".into()
                            } else {
                                tail
                            }
                        ),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => StreamMsg::UpdateResult {
                    ok: false,
                    text: "Couldn't run `npm` — it's not on PATH.\n\
                           Install Node.js (https://nodejs.org) and try again, or grab a\n\
                           prebuilt binary from https://github.com/rekabytes/hmanlab/releases."
                        .into(),
                },
                Err(e) => StreamMsg::UpdateResult {
                    ok: false,
                    text: format!("Failed to launch npm: {e}"),
                },
            };
            let _ = tx.send(msg);
        });
    }

    fn show_help_inline(&mut self) {
        let help = "Commands:\n\
            \x20 /new                start a fresh session\n\
            \x20 /sessions, /hist    list recent saved sessions\n\
            \x20 /load <id-prefix>   load a saved session (10 most recent messages)\n\
            \x20 /more, /older       load 10 older messages in the current loaded session\n\
            \x20 /model              open model picker\n\
            \x20 /model <name>       switch model (partial match works)\n\
            \x20 /models, /ls        list available models\n\
            \x20 /host <url>         change Ollama host\n\
            \x20 /workspace <path>   change agent workspace\n\
            \x20 /clear              clear visible chat (current session keeps going)\n\
            \x20 /compact            summarise prior turns into a single context briefing\n\
            \x20 /disconnect [name]  drop a BYOK provider key (zai, zai-usage, ollama-cloud, opencode)\n\
            \x20 /settings, /whoami  show account info, version, configured providers\n\
            \x20 /update             update hmanlab to the latest npm release\n\
            \x20 /help, /?           show this help\n\
            \x20 /quit, /exit        quit\n\
            \n\
            Tools (agent uses these on its own — needs a tool-capable model like qwen2.5):\n\
            \x20 read_file, list_dir, find_files, git_status, git_log, git_diff,\n\
            \x20 git_show, run_command (shell — you confirm each call).\n\
            \n\
            Keys:\n\
            \x20 Enter         send  ·  Shift+Enter  newline\n\
            \x20 Ctrl+N        new session  ·  Ctrl+T  fold/unfold all tool + thinking blocks\n\
            \x20 Wheel         scroll chat  ·  PgUp/PgDn  Home/End  also scroll\n\
            \x20 Ctrl+C        cancel/quit  ·  Esc  interrupt generation / clear draft\n\
            \n\
            Drag with your mouse to select text — copy with your terminal's normal\n\
            shortcut (Ctrl+Shift+C / Cmd+C). The wheel scrolls the chat in single-line\n\
            input mode; when composing multi-line input (Shift+Enter), use PgUp/PgDn.";
        self.push_info(help.to_string());
    }

    fn switch_host(&mut self, url: String, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let url = url.trim();
        if url.is_empty() {
            self.push_info(format!(
                "Current host: {}\nUsage: /host <url>",
                self.client.base
            ));
            return;
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.push_info(format!(
                "Host must start with http:// or https://. Got: {url}"
            ));
            self.status = "Invalid host URL".into();
            return;
        }
        self.client = Client::new(url.to_string());
        self.push_info(format!(
            "Host set to {}. Refreshing models…",
            self.client.base
        ));
        self.status = "Refreshing models…".into();
        let client = self.client.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            match client.list_models().await {
                Ok(models) => {
                    let _ = tx.send(StreamMsg::Models {
                        models,
                        base: client.base.clone(),
                    });
                }
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(format!("list models: {e}")));
                }
            }
        });
    }

    pub(super) fn push_info(&mut self, content: String) {
        self.messages.push(ChatMessage {
            role: "info".into(),
            content,
            ..Default::default()
        });
        self.follow = true;
    }

    fn reset_input(&mut self) {
        let mut fresh = fresh_textarea();
        fresh.set_placeholder_text(
            "Type a message, or /help for commands.  (Enter=send, Shift+Enter=newline)",
        );
        self.input = fresh;
    }

    /// True when the most recent assistant turn made zero tool calls.
    /// Walks back to the last user message, counting any tool messages in
    /// between. Used to decide whether the model only announced intent.
    pub(super) fn no_tools_since_last_user(&self) -> bool {
        for m in self.messages.iter().rev() {
            if m.role == "user" {
                return true;
            }
            if m.role == "tool" {
                return false;
            }
        }
        true
    }

    /// Heuristic: did the latest assistant message announce intent rather
    /// than acting? ('I'll look at…', 'Let me check…', etc.)
    pub(super) fn looks_like_intent_announcement(&self) -> bool {
        const PATTERNS: &[&str] = &[
            "i'll",
            "let's",
            "let me",
            "i'm going to",
            "i am going to",
            "going to",
            "i will",
            "i shall",
        ];
        let last = self
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.trim().is_empty());
        let Some(m) = last else { return false };
        let lc = m.content.to_lowercase();
        PATTERNS.iter().any(|p| lc.contains(p))
    }

    /// True when the last assistant turn ended with one of the configured
    /// trigger phrases — at which point Y/N should fire the quick-reply.
    /// Inspects ONLY the last `?`-terminated sentence, and skips when that
    /// sentence is a WH-question (open-ended, not yes/no).
    pub(super) fn last_assistant_invites_yn(&self) -> bool {
        const TRIGGERS: &[&str] = &[
            "would you like",
            "would you want",
            "shall i",
            "shall we",
            "want me to",
            "want more",
            "should i",
            "should we",
            "do you want",
            "do you need",
            "any specific",
            "anything else",
            "anything specific",
            "more detail",
            "interested in",
            "let me know if",
            "let me know which",
            "which one",
            "which would",
        ];
        const WH_WORDS: &[&str] = &["what", "which", "who", "where", "when", "why", "how"];
        let last = self
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.trim().is_empty());
        let Some(m) = last else { return false };
        let trimmed = m.content.trim_end();
        if !trimmed.ends_with('?') {
            return false;
        }
        // Extract the last sentence: walk back from the trailing `?` to the
        // previous sentence terminator (or start of string).
        let bytes = trimmed.as_bytes();
        let mut start = 0usize;
        // Skip the trailing `?` itself, then scan backwards.
        for i in (0..bytes.len().saturating_sub(1)).rev() {
            let b = bytes[i];
            if b == b'.' || b == b'!' || b == b'?' || b == b'\n' {
                start = i + 1;
                break;
            }
        }
        let sentence = trimmed[start..].trim();
        // Open-ended questions ("What…", "Which…", "How…") are not Y/N.
        if let Some(first) = sentence.split_whitespace().next() {
            let first_lc = first
                .trim_matches(|c: char| !c.is_alphabetic())
                .to_lowercase();
            if WH_WORDS.iter().any(|w| *w == first_lc) {
                return false;
            }
        }
        let lc = sentence.to_lowercase();
        TRIGGERS.iter().any(|t| lc.contains(t))
    }

    /// Send a user message that goes to the model but is NOT rendered in the
    /// chat UI. Used by the Y/N quick-reply so accept/deny doesn't pollute the
    /// visible transcript.
    pub(super) fn inject_hidden_user(&mut self, text: &str, tx: &mpsc::UnboundedSender<StreamMsg>) {
        if (self.models.is_empty() && self.extra_models.is_empty()) || self.generating {
            return;
        }
        if let Some(api_tx) = &self.api_tx {
            // Persist the silent reply too — the session record stays coherent.
            let _ = api_tx.send(ApiOp::UserMessage {
                content: text.to_string(),
                model: self.model.clone(),
            });
        }
        self.messages.push(ChatMessage {
            role: "user".into(),
            content: text.into(),
            hidden: true,
            ..Default::default()
        });
        self.messages.push(ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            ..Default::default()
        });
        self.generating = true;
        self.follow = true;
        self.status = format!("Generating with {}…", self.model);
        let history: Vec<ChatMessage> = self.messages[..self.messages.len() - 1]
            .iter()
            .filter(|m| matches!(m.role.as_str(), "user" | "assistant" | "tool"))
            .cloned()
            .collect();
        let Some(backend) = self.make_backend() else {
            self.generating = false;
            self.status = format!("No API key configured for model {}", self.model);
            return;
        };
        let model = self.model.clone();
        let workspace = self.workspace.clone();
        let tx = tx.clone();
        let handle = tokio::spawn(async move {
            crate::agent::agent_loop(backend, model, history, workspace, tx).await;
        });
        self.current_task = Some(handle);
    }

    pub(super) fn send_to_llm(&mut self, text: String, tx: &mpsc::UnboundedSender<StreamMsg>) {
        if self.models.is_empty() && self.extra_models.is_empty() {
            self.push_info(
                "Not connected to a model. Use /host <url> for Ollama, or /model to add a BYOK provider.".into(),
            );
            self.status = "No model".into();
            return;
        }

        // Auto-compaction: if the last assistant turn's prompt was over
        // threshold, fold the visible history into a summary first, then
        // re-issue this user message once compaction completes. Bail out
        // if we're already compacting (avoid re-entry) or generating.
        if !self.compacting
            && !self.generating
            && self.last_prompt_tokens > crate::compact::AUTO_COMPACT_THRESHOLD
            && self
                .messages
                .iter()
                .any(|m| !m.hidden && m.role == "assistant")
        {
            self.push_info(format!(
                "Context at {} tokens — compacting before sending so your next turn has room.",
                self.last_prompt_tokens
            ));
            self.start_compact(tx, Some(text));
            return;
        }

        if let Some(api_tx) = &self.api_tx {
            let _ = api_tx.send(ApiOp::UserMessage {
                content: text.clone(),
                model: self.model.clone(),
            });
        }

        self.messages.push(ChatMessage {
            role: "user".into(),
            content: text,
            ..Default::default()
        });
        self.messages.push(ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            ..Default::default()
        });
        self.generating = true;
        self.follow = true;
        self.status = format!("Generating with {}…", self.model);

        // History sent to the model: prior user/assistant/tool turns plus
        // any compaction summary translated to a `system` role. The
        // trailing empty assistant placeholder is dropped.
        let history: Vec<ChatMessage> = self.messages[..self.messages.len() - 1]
            .iter()
            .filter(|m| matches!(m.role.as_str(), "user" | "assistant" | "tool" | "summary"))
            .map(|m| {
                if m.role == "summary" {
                    ChatMessage {
                        role: "system".into(),
                        content: format!("(Compacted summary of earlier turns:)\n\n{}", m.content),
                        ..m.clone()
                    }
                } else {
                    m.clone()
                }
            })
            .collect();

        let Some(backend) = self.make_backend() else {
            self.generating = false;
            self.status = format!("No API key configured for model {}", self.model);
            return;
        };
        let model = self.model.clone();
        let workspace = self.workspace.clone();
        let tx = tx.clone();
        let handle = tokio::spawn(async move {
            crate::agent::agent_loop(backend, model, history, workspace, tx).await;
        });
        self.current_task = Some(handle);
    }

    fn cancel(&mut self) {
        if let Some(h) = self.current_task.take() {
            h.abort();
        }
        if let Some(h) = self.compact_task.take() {
            h.abort();
            self.compacting = false;
            self.pending_after_compact = None;
        }
        self.persist_assistant_if_any();
        self.generating = false;
        self.active_tool_msg_idx = None;
        self.status = "Cancelled".into();
    }

    /// `/disconnect [provider]` — drop a saved BYOK key and the matching
    /// extra-model rows. With no arg, lists what's currently connected so
    /// the user can pick. With an arg, accepts a few common aliases
    /// (`zai`, `zai-usage`, `ollama-cloud`, `opencode`, plus `go` for
    /// OpenCode). If the currently-active model belonged to the
    /// disconnected provider, the active model falls back to the first
    /// Ollama-discovered model, then the first remaining extra, else empty.
    fn handle_disconnect(&mut self, name: &str) {
        let trimmed = name.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            // No arg → open the arrow-key picker. Mirrors how `/model`
            // with no arg opens the model picker.
            self.open_disconnect_picker();
            return;
        }
        let provider_id: &'static str = match trimmed.as_str() {
            "zai" | "zai-subscription" | "subscription" | "z.ai" => {
                crate::config::ZAI_SUBSCRIPTION_PROVIDER
            }
            "zai-usage" | "usage" | "z.ai-usage" => crate::config::ZAI_USAGE_PROVIDER,
            "ollama-cloud" | "cloud" | "ollama_cloud" => crate::config::OLLAMA_CLOUD_PROVIDER,
            "opencode" | "opencode-go" | "go" | "oc" => crate::config::OPENCODE_PROVIDER,
            other => {
                self.push_info(format!(
                    "Unknown provider '{other}'. Try: zai, zai-usage, ollama-cloud, opencode.\nOr /disconnect (no args) to open the picker."
                ));
                return;
            }
        };
        self.disconnect_by_id(provider_id);
    }

    /// Drop a BYOK provider by its canonical id. Shared between the
    /// string-arg fast-path (`/disconnect zai`) and the picker
    /// (`/disconnect` → Enter on a row). All key-clearing, model-removal,
    /// active-model fallback, and config persistence live here.
    fn disconnect_by_id(&mut self, provider_id: &str) {
        // No-op guard so the user gets a clean message rather than a
        // silent "removed 0 things".
        let key_present = match provider_id {
            p if p == crate::config::ZAI_SUBSCRIPTION_PROVIDER => self.zai_api_key.is_some(),
            p if p == crate::config::ZAI_USAGE_PROVIDER => self.zai_usage_api_key.is_some(),
            p if p == crate::config::OLLAMA_CLOUD_PROVIDER => self.ollama_cloud_api_key.is_some(),
            p if p == crate::config::OPENCODE_PROVIDER => self.opencode_api_key.is_some(),
            _ => false,
        };
        if !key_present {
            self.push_info(format!(
                "'{provider_id}' isn't connected — nothing to disconnect."
            ));
            return;
        }

        // Clear the key on App; persist_config later flushes it to disk.
        match provider_id {
            p if p == crate::config::ZAI_SUBSCRIPTION_PROVIDER => self.zai_api_key = None,
            p if p == crate::config::ZAI_USAGE_PROVIDER => self.zai_usage_api_key = None,
            p if p == crate::config::OLLAMA_CLOUD_PROVIDER => self.ollama_cloud_api_key = None,
            p if p == crate::config::OPENCODE_PROVIDER => self.opencode_api_key = None,
            _ => {}
        }

        // Sweep the matching extra-model rows out of the picker source.
        let removed = self
            .extra_models
            .iter()
            .filter(|m| m.provider == provider_id)
            .count();
        self.extra_models.retain(|m| m.provider != provider_id);

        // Fall back the active model if it belonged to this provider.
        let active_was_provider =
            self.selected_extra.as_ref().map(|e| e.provider.as_str()) == Some(provider_id);
        if active_was_provider {
            self.selected_extra = None;
            if let Some(first) = self.models.first().cloned() {
                self.model = first;
            } else if let Some(em) = self.extra_models.first().cloned() {
                self.model = em.name.clone();
                self.selected_extra = Some(em);
            } else {
                self.model.clear();
            }
        }

        self.persist_config();
        self.push_info(format!(
            "Disconnected {provider_id} — removed {removed} model(s). Use /model to add another provider or pick a different model."
        ));
        self.status = format!("Disconnected {provider_id}");
    }

    /// Build `disconnect_entries` from currently-connected BYOK providers
    /// and switch to `Mode::DisconnectPicker`. If nothing is connected,
    /// push an info row and stay in chat — there's no point opening an
    /// empty popup.
    fn open_disconnect_picker(&mut self) {
        let mut entries: Vec<DisconnectEntry> = Vec::new();
        let preview = |provider: &str, extras: &[ExtraModel]| -> String {
            let names: Vec<&str> = extras
                .iter()
                .filter(|m| m.provider == provider)
                .map(|m| m.name.as_str())
                .collect();
            if names.is_empty() {
                "(no models seeded)".to_string()
            } else if names.len() <= 3 {
                names.join(", ")
            } else {
                format!("{}, +{} more", names[..3].join(", "), names.len() - 3)
            }
        };
        if self.zai_api_key.is_some() {
            entries.push(DisconnectEntry {
                provider: crate::config::ZAI_SUBSCRIPTION_PROVIDER.to_string(),
                label: "z.ai subscription".to_string(),
                preview: preview(crate::config::ZAI_SUBSCRIPTION_PROVIDER, &self.extra_models),
            });
        }
        if self.zai_usage_api_key.is_some() {
            entries.push(DisconnectEntry {
                provider: crate::config::ZAI_USAGE_PROVIDER.to_string(),
                label: "z.ai usage-based".to_string(),
                preview: preview(crate::config::ZAI_USAGE_PROVIDER, &self.extra_models),
            });
        }
        if self.ollama_cloud_api_key.is_some() {
            entries.push(DisconnectEntry {
                provider: crate::config::OLLAMA_CLOUD_PROVIDER.to_string(),
                label: "Ollama Cloud".to_string(),
                preview: preview(crate::config::OLLAMA_CLOUD_PROVIDER, &self.extra_models),
            });
        }
        if self.opencode_api_key.is_some() {
            entries.push(DisconnectEntry {
                provider: crate::config::OPENCODE_PROVIDER.to_string(),
                label: "OpenCode Go".to_string(),
                preview: preview(crate::config::OPENCODE_PROVIDER, &self.extra_models),
            });
        }
        if entries.is_empty() {
            self.push_info(
                "No BYOK providers connected. Use /model and the + Add … rows to add one.".into(),
            );
            return;
        }
        self.disconnect_entries = entries;
        self.disconnect_index = 0;
        self.mode = Mode::DisconnectPicker;
        self.status = "↑↓ to navigate · Enter to disconnect · Esc to cancel".into();
    }

    fn handle_disconnect_picker(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Chat;
                self.status = "Disconnect cancelled".into();
            }
            KeyCode::Up | KeyCode::Char('k') if self.disconnect_index > 0 => {
                self.disconnect_index -= 1;
            }
            KeyCode::Down | KeyCode::Char('j')
                if self.disconnect_index + 1 < self.disconnect_entries.len() =>
            {
                self.disconnect_index += 1;
            }
            KeyCode::Home => self.disconnect_index = 0,
            KeyCode::End => {
                self.disconnect_index = self.disconnect_entries.len().saturating_sub(1);
            }
            KeyCode::Enter => {
                if let Some(entry) = self.disconnect_entries.get(self.disconnect_index).cloned() {
                    self.mode = Mode::Chat;
                    self.disconnect_by_id(&entry.provider);
                }
            }
            _ => {}
        }
        AppAction::Continue
    }

    /// Kick off an asynchronous compaction. Sends the current visible
    /// history (minus hidden / info / system entries) to the active model
    /// with a summarization system prompt. The reply lands as
    /// `StreamMsg::CompactionDone`, where `app::stream` replaces the
    /// visible history with a `summary`-role message and, if a pending
    /// user message was buffered via `pending_after_compact`, re-issues it.
    pub(super) fn start_compact(
        &mut self,
        tx: &mpsc::UnboundedSender<StreamMsg>,
        pending_user_message: Option<String>,
    ) {
        if self.compacting {
            self.push_info("A compaction is already running.".into());
            return;
        }
        if self.generating {
            self.push_info("Wait for the current turn to finish, then /compact.".into());
            return;
        }
        let to_compact_count = self
            .messages
            .iter()
            .filter(|m| !m.hidden && matches!(m.role.as_str(), "user" | "assistant" | "tool"))
            .count();
        if to_compact_count < 2 {
            self.push_info("Nothing meaningful to compact yet.".into());
            return;
        }
        let Some(backend) = self.make_backend() else {
            self.push_info(format!(
                "Can't compact — no backend configured for model {}.",
                self.model
            ));
            return;
        };

        // Snapshot the visible history for the task. Hidden user messages
        // (Y/N injections) are dropped — they're not real conversation
        // turns the summary should preserve.
        let snapshot: Vec<ChatMessage> = self
            .messages
            .iter()
            .filter(|m| !m.hidden)
            .cloned()
            .collect();
        let model = self.model.clone();
        let tx2 = tx.clone();
        self.compacting = true;
        self.pending_after_compact = pending_user_message;
        self.status = "Compacting conversation…".into();
        self.follow = true;
        self.push_info("/compact — summarising prior turns into a single context briefing.".into());

        let handle = tokio::spawn(async move {
            match crate::compact::compact_history(&backend, &model, snapshot).await {
                Ok((summary, prompt_tokens, completion_tokens)) => {
                    let _ = tx2.send(StreamMsg::CompactionDone {
                        summary,
                        prompt_tokens,
                        completion_tokens,
                    });
                }
                Err(e) => {
                    let _ = tx2.send(StreamMsg::CompactionError(e.to_string()));
                }
            }
        });
        self.compact_task = Some(handle);
    }
}
