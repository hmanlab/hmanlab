//! Stream-message handler. The agent loop talks to the UI via `StreamMsg`
//! events; this is the dispatch that turns each one into UI state.

use tokio::sync::mpsc;

use crate::api::ApiOp;
use crate::ollama::ChatMessage;

use super::{App, Mode, StreamMsg};

/// Pull a one-line description out of a compaction summary for use as the
/// memory's index entry. Tries to find the first non-empty bullet,
/// strips the marker (`- `, `* `, `• `), and caps at 200 chars so the
/// MEMORY.md index stays scannable.
fn compact_description(summary: &str) -> String {
    let mut picked = String::new();
    for line in summary.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cleaned = trimmed.trim_start_matches(['-', '*', '•']).trim();
        if !cleaned.is_empty() {
            picked = cleaned.replace(['\r', '\n'], " ");
            break;
        }
    }
    if picked.is_empty() {
        picked = "Compacted session summary".to_string();
    }
    if picked.chars().count() > 200 {
        let mut t: String = picked.chars().take(199).collect();
        t.push('…');
        picked = t;
    }
    picked
}

impl App {
    pub fn handle_stream(&mut self, msg: StreamMsg, tx: &mpsc::UnboundedSender<StreamMsg>) {
        match msg {
            StreamMsg::Chunk(text) => {
                if let Some(last) = self.messages.last_mut() {
                    if last.role == "assistant" {
                        last.content.push_str(&text);
                    }
                }
            }
            StreamMsg::AssistantTurnEnded { tool_calls } => {
                // Snapshot the assistant content + tool_calls before mutation,
                // then persist this intermediate turn so future fine-tunes can
                // see the model's tool-calling behavior, not just the final
                // text. Without this we'd only ever capture the closing reply.
                let snapshot: Option<(String, serde_json::Value)> =
                    if let Some(last) = self.messages.last_mut() {
                        if last.role == "assistant" && !tool_calls.is_empty() {
                            last.tool_calls = Some(tool_calls.clone());
                            let tc_value = serde_json::to_value(&tool_calls)
                                .unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));
                            Some((last.content.clone(), tc_value))
                        } else {
                            if last.role == "assistant" {
                                last.tool_calls = Some(tool_calls);
                            }
                            None
                        }
                    } else {
                        None
                    };
                if let (Some((content, tc_value)), Some(api_tx)) = (snapshot, self.api_tx.as_ref())
                {
                    let _ = api_tx.send(ApiOp::AssistantToolCalls {
                        content,
                        tool_calls: tc_value,
                        model: self.model.clone(),
                    });
                }
            }
            StreamMsg::ToolStart { name, args } => {
                let args_str = serde_json::to_string(&args).unwrap_or_else(|_| "{}".into());
                self.messages.push(ChatMessage {
                    role: "tool".into(),
                    name: Some(name),
                    content: format!("(running… args: {args_str})"),
                    ..Default::default()
                });
                self.active_tool_msg_idx = Some(self.messages.len() - 1);
                self.follow = true;
            }
            StreamMsg::ToolResult { output } => {
                // Walk backwards to find the most recent tool placeholder.
                // We can't just look at `last()` because confirmed tools
                // (run_command / edit_file / write_file) sit through the
                // user's y/n decision — the handler for that decision calls
                // `push_info(...)` which appends a system message between the
                // tool placeholder and the eventual ToolResult. Trusting
                // `last_mut()` silently drops the tool result on the floor
                // (and from the DB, which breaks training data for any tool
                // that requires confirmation).
                let mut to_persist: Option<(String, String)> = None;
                for msg in self.messages.iter_mut().rev() {
                    if msg.role == "tool" {
                        msg.content = output.clone();
                        if let Some(n) = msg.name.clone() {
                            to_persist = Some((n, output));
                        }
                        break;
                    }
                }
                if let (Some((name, output)), Some(api_tx)) = (to_persist, self.api_tx.as_ref()) {
                    let _ = api_tx.send(ApiOp::ToolResult { name, output });
                }
                self.active_tool_msg_idx = None;
            }
            StreamMsg::NewAssistantTurn => {
                self.messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    ..Default::default()
                });
                self.follow = true;
            }
            StreamMsg::ConfirmRequest(req) => {
                self.pending_confirm = Some(req);
                self.mode = Mode::Confirm;
                self.status = "Confirmation needed — y/n".into();
            }
            StreamMsg::Done {
                prompt_tokens,
                completion_tokens,
            } => {
                self.persist_assistant_if_any();
                self.total_prompt_tokens = self
                    .total_prompt_tokens
                    .saturating_add(prompt_tokens as u64);
                self.total_completion_tokens = self
                    .total_completion_tokens
                    .saturating_add(completion_tokens as u64);
                // Track this turn's prompt size for the next auto-compact
                // check in `send_to_llm`.
                self.last_prompt_tokens = prompt_tokens;
                self.generating = false;
                self.current_task = None;
                self.active_tool_msg_idx = None;
                self.status = format!(
                    "Ready  ·  this turn: {} in / {} out",
                    prompt_tokens, completion_tokens
                );

                // Auto-continue: if this turn was the reply to a Y-injection
                // and the model only announced intent (no tool calls + 'I'll
                // do X' text), nudge it to actually do the thing. One retry.
                if self.awaiting_yn_followup {
                    self.awaiting_yn_followup = false;
                    if self.no_tools_since_last_user() && self.looks_like_intent_announcement() {
                        self.inject_hidden_user(
                            "You announced intent but didn't act. Call the necessary tools and \
                             actually do the work now — don't restate the plan.",
                            tx,
                        );
                        return;
                    }
                }

                self.yn_pending = self.last_assistant_invites_yn();
            }
            StreamMsg::Error(e) => {
                self.persist_assistant_if_any();
                self.generating = false;
                self.current_task = None;
                self.active_tool_msg_idx = None;
                self.status = format!("Error: {e}");
            }
            StreamMsg::CompactionDone {
                summary,
                prompt_tokens,
                completion_tokens,
            } => {
                self.compacting = false;
                self.compact_task = None;
                self.total_prompt_tokens = self
                    .total_prompt_tokens
                    .saturating_add(prompt_tokens as u64);
                self.total_completion_tokens = self
                    .total_completion_tokens
                    .saturating_add(completion_tokens as u64);

                // Persist the summary as a project-scope memory so it
                // survives /clear, restart, and cross-session use. Rolling
                // single file (`compact-current.md`) — each new compaction
                // overwrites it. The model picks it up on next launch via
                // the always-loaded MEMORY.md index.
                let description = compact_description(&summary);
                let persisted = crate::memory::save_memory(
                    crate::memory::MemoryScope::Project,
                    "compact-current",
                    "project",
                    &description,
                    &summary,
                    &self.workspace,
                );

                // Replace the visible history with a single `summary`-role
                // entry. The `send_to_llm` filter translates summary →
                // system before each next turn, so the model still sees
                // the compacted context as load-bearing background.
                let summary_msg = ChatMessage {
                    role: "summary".into(),
                    content: summary,
                    ..Default::default()
                };
                self.messages.clear();
                self.messages.push(summary_msg);
                // Tool expansion state pointed at indices that no longer
                // exist; clear it so no stale highlights remain.
                self.expanded_tools.clear();
                self.expanded_thoughts.clear();
                // Reset the auto-compact trigger — the next turn starts
                // small again.
                self.last_prompt_tokens = 0;
                self.follow = true;
                self.status = format!(
                    "Compacted  ·  summary: {} in / {} out",
                    prompt_tokens, completion_tokens
                );
                match persisted {
                    Ok(path) => self.push_info(format!(
                        "Saved to {} (project memory `compact-current`).",
                        path.display()
                    )),
                    Err(e) => {
                        self.push_info(format!("Compaction succeeded but persistence failed: {e}"))
                    }
                }
                // Replay the user message that was buffered while
                // compaction ran, if any. Done last so all state is
                // consistent before the new turn starts.
                if let Some(pending) = self.pending_after_compact.take() {
                    self.send_to_llm(pending, tx);
                }
            }
            StreamMsg::CompactionError(e) => {
                self.compacting = false;
                self.compact_task = None;
                // The original history is intact (we didn't touch it
                // before the result came back). Drop any buffered message
                // — re-running it would just re-trigger the auto-compact.
                self.pending_after_compact = None;
                self.push_info(format!("Compaction failed: {e}. History unchanged."));
                self.status = format!("Compact error: {e}");
            }
            StreamMsg::UpdateAvailable(latest) => {
                self.update_available = Some(latest);
            }
            StreamMsg::UpdateInfo(text) => {
                self.push_info(text);
            }
            StreamMsg::Settings(text) => {
                self.push_info(text);
                self.status = "Settings loaded".into();
            }
            StreamMsg::UpdateResult { ok, text } => {
                self.push_info(text);
                self.status = if ok {
                    "Update finished — restart hmanlab to use the new version".into()
                } else {
                    "Update failed".into()
                };
                if ok {
                    // Clear the header notice: whatever version was advertised
                    // has now been installed, even if we can't detect the new
                    // version until the user restarts.
                    self.update_available = None;
                }
            }
            StreamMsg::Models { models, base } => {
                let n = models.len();
                self.models = models;
                if !self.models.iter().any(|m| m == &self.model) {
                    if let Some(first) = self.models.first() {
                        self.model = first.clone();
                    }
                }
                self.status = format!("Connected to {base} — {n} model(s)");
                self.push_info(format!(
                    "Connected to {base}\nModels available: {n}\nCurrent: {}",
                    self.model
                ));
                // /host succeeded — remember the URL across restarts so the
                // user doesn't have to re-add Ollama every session.
                let mut cfg = crate::config::load().ok().flatten().unwrap_or_default();
                cfg.ollama_host = Some(base);
                let _ = crate::config::save(&cfg);
            }
            StreamMsg::SessionList(rows) => {
                if rows.is_empty() {
                    self.push_info("No saved sessions yet. Send a message to start one.".into());
                } else {
                    self.session_picker_items = rows;
                    self.session_picker_index = 0;
                    self.mode = Mode::SessionPicker;
                    self.status = "↑↓ to navigate · Enter to load · Esc to cancel".into();
                }
            }
            StreamMsg::Loaded { session, messages } => {
                self.messages.clear();
                for m in &messages {
                    self.messages.push(ChatMessage {
                        role: m.role.clone(),
                        content: m.content.clone(),
                        ..Default::default()
                    });
                }
                if let Some(model) = &session.model {
                    if self.models.iter().any(|m| m == model) {
                        // Ollama-discovered model: clear any active extra so
                        // routing goes to Ollama.
                        self.model = model.clone();
                        self.selected_extra = None;
                    } else if let Some(em) =
                        self.extra_models.iter().find(|m| &m.name == model).cloned()
                    {
                        // Extra-provider model. If the same name exists for
                        // multiple providers (e.g. glm-4.7 on both z.ai
                        // plans), this picks the first one in extra_models —
                        // user can /model via picker to switch plans.
                        self.model = em.name.clone();
                        self.selected_extra = Some(em);
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
                self.status = format!("Loaded — {count} message(s) (use /more for older)");
                let id_str = session_id.replace('-', "");
                let short = &id_str[..id_str.len().min(8)];
                let hint = if count == 10 {
                    "\n(Showing 10 most recent — type /more for older messages.)"
                } else {
                    ""
                };
                self.push_info(format!(
                    "Loaded session {short} — \"{}\"  ·  {count} message(s){hint}",
                    session.title
                ));
            }
            StreamMsg::MoreLoaded { messages } => {
                if messages.is_empty() {
                    self.push_info("No older messages.".into());
                    self.status = "No older messages".into();
                    return;
                }
                let count = messages.len();
                if let Some(min_id) = messages.iter().map(|m| m.id).min() {
                    self.oldest_loaded_msg_id = Some(min_id);
                }
                // Shift any expanded-tool indices since everything moves down by `count`.
                self.expanded_tools = self.expanded_tools.iter().map(|&i| i + count).collect();

                let mut prepend: Vec<ChatMessage> = messages
                    .into_iter()
                    .map(|m| ChatMessage {
                        role: m.role,
                        content: m.content,
                        ..Default::default()
                    })
                    .collect();
                prepend.append(&mut self.messages);
                self.messages = prepend;
                self.follow = false;
                self.scroll = 0;
                self.status = format!("Loaded {count} older message(s)");
            }
        }
    }

    /// Persist the trailing assistant message if it's the final reply
    /// (no tool_calls) and non-empty. Otherwise drop empties.
    pub(super) fn persist_assistant_if_any(&mut self) {
        if let Some(last) = self.messages.last() {
            if last.role != "assistant" {
                return;
            }
            let has_tool_calls = last
                .tool_calls
                .as_ref()
                .map(|tc| !tc.is_empty())
                .unwrap_or(false);
            if last.content.trim().is_empty() && !has_tool_calls {
                self.messages.pop();
            } else if !has_tool_calls && !last.content.trim().is_empty() {
                // Strip the `<think>…</think>` reasoning block before persisting.
                // It's useful in-session as a foldable block but is in-flight
                // scratch — durable storage should hold only the visible answer.
                let raw = &last.content;
                let content = match raw.find("</think>") {
                    Some(idx) => raw[idx + "</think>".len()..]
                        .trim_start_matches(['\n', '\r'])
                        .to_string(),
                    None => raw.clone(),
                };
                if content.trim().is_empty() {
                    return;
                }
                let model = self.model.clone();
                if let Some(api_tx) = &self.api_tx {
                    let _ = api_tx.send(ApiOp::AssistantMessage { content, model });
                }
            }
        }
    }
}
