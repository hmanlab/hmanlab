//! Session-related stream handlers: `/sessions` results, `/load` (replace
//! visible history), `/more` (prepend older messages).

use crate::api::{ApiOp, Message, Session};
use crate::ollama::ChatMessage;

use super::super::{App, Mode, PageState};
use super::api_message_to_chat;

impl App {
    pub(super) fn on_session_list(&mut self, rows: Vec<Session>) {
        if rows.is_empty() {
            self.push_info("No saved sessions yet. Send a message to start one.".into());
        } else {
            self.session_picker.set_items(rows);
            self.mode = Mode::SessionPicker;
            self.status = "↑↓ to navigate · Enter to load · Esc to cancel".into();
        }
    }

    pub(super) fn on_loaded(&mut self, session: Session, messages: Vec<Message>) {
        self.messages.clear();
        for m in &messages {
            self.messages.push(api_message_to_chat(m));
        }
        // Fresh session — reset pagination. The previous session's
        // "no more older" verdict doesn't apply to this one.
        self.page_state = PageState::Idle;
        // The loaded session has its own recorded model, but the
        // user's current selection wins — switching sessions
        // shouldn't silently bounce them off the model they just
        // picked. Surface the difference inline so they can
        // /model back to the session's original if they want.
        if let Some(recorded) = session.model.as_deref() {
            if recorded != self.model {
                self.push_info(format!(
                    "Loaded session previously used {recorded}; continuing with your current model ({}). Run /model to switch.",
                    self.model
                ));
            }
        }
        let session_id = session.id.clone();
        if let Some(api_tx) = &self.api_tx {
            let _ = api_tx.send(ApiOp::SetSession(session_id.clone()));
        }
        self.loaded_session_id = Some(session_id.clone());
        self.oldest_loaded_msg_id = messages.iter().map(|m| m.id).min();
        self.follow = true;
        self.scroll = 0;
        let count = messages.len();
        self.status = format!("Loaded — {count} message(s) — scroll up for older");
        let id_str = session_id.replace('-', "");
        let short = &id_str[..id_str.len().min(8)];
        // 30 matches `commands::session::PAGE_SIZE` — if the load returned
        // a full page, there's almost certainly older history on the server
        // and the auto-loader will fetch it when the user scrolls up.
        let hint = if count >= 30 {
            "\n(Showing 30 most recent — scroll up to load older messages automatically.)"
        } else {
            ""
        };
        self.push_info(format!(
            "Loaded session {short} — \"{}\"  ·  {count} message(s){hint}",
            session.title
        ));
    }

    pub(super) fn on_more_loaded(&mut self, messages: Vec<Message>) {
        if messages.is_empty() {
            // Tell the user once, then never auto-fire again for this
            // session — `Exhausted` short-circuits future
            // `maybe_auto_load_more` calls.
            self.page_state = PageState::Exhausted;
            self.push_info("No older messages.".into());
            self.status = "No older messages".into();
            return;
        }
        // Server returned content; transition back to Idle so the next
        // scroll-to-top can trigger another /more.
        self.page_state = PageState::Idle;
        let count = messages.len();
        if let Some(min_id) = messages.iter().map(|m| m.id).min() {
            self.oldest_loaded_msg_id = Some(min_id);
        }
        // Shift any expanded-tool indices since everything moves down by `count`.
        self.expanded_tools = self.expanded_tools.iter().map(|&i| i + count).collect();

        let mut prepend: Vec<ChatMessage> = messages.iter().map(api_message_to_chat).collect();
        prepend.append(&mut self.messages);
        self.messages = prepend;
        // Keep the user pinned to the same logical row that was on top
        // before the prepend, so scroll-triggered auto-loads don't yank
        // the viewport. Without this, scroll snaps to 0 (effectively
        // "follow the new top") and a single scroll-up gesture triggers
        // an immediate cascade of more loads.
        self.follow = false;
        self.scroll = count as u16;
        self.status = format!("Loaded {count} older message(s)");
    }
}
