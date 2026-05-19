//! UI mode + picker-row types. Kept out of `app/mod.rs` so that file
//! stays focused on the `App` struct itself; everything in here is a
//! small, mostly-enum value the input handlers and renderers reach for.

use std::path::PathBuf;

use crate::config::ExtraModel;

/// Renderer-produced, input-consumed scratch state. The chat and sidebar
/// renderers write geometry and hit-test tables here every frame; the
/// mouse handler reads them on the next event to translate screen
/// coordinates back into clicks on specific messages/files.
///
/// Used to live as 15 loose fields on `App`. Grouping them makes the
/// "renderer is the producer, input is the consumer" relationship
/// explicit and means a future contributor can't forget to reset one of
/// the Vecs at the top of a render pass without it being visibly grouped.
#[derive(Default)]
pub struct RenderState {
    /// Chat panel inner geometry from the last render.
    pub chat_x: u16,
    pub chat_y: u16,
    pub chat_w: u16,
    pub chat_h: u16,
    /// Sidebar panel inner geometry from the last render.
    pub sidebar_x: u16,
    pub sidebar_y: u16,
    pub sidebar_w: u16,
    pub sidebar_h: u16,
    /// One row per visible sidebar entry — `(logical_line_idx, abs_path,
    /// is_dir)`. The click handler converts `screen_y` to a logical
    /// line, looks up the matching entry, and toggles or opens it.
    pub sidebar_targets: Vec<(u16, PathBuf, bool)>,
    /// One row per visible card-style tool tile — `(logical_line_idx,
    /// message_idx)`. The hover overlay uses this to repaint the cell
    /// bg under the cursor; clicking a card row toggles `expanded_tools`.
    pub card_row_targets: Vec<(u16, usize)>,
    /// `(message_idx, logical_line_start_excl, logical_line_end_excl)`
    /// per visible message. The mouse handler converts a click row into
    /// a logical line, then finds the message that owns it.
    pub message_line_ranges: Vec<(usize, u16, u16)>,
    /// Plain-text projection of each rendered chat line. Copy-on-drag
    /// extracts the selection from this without re-parsing styled spans.
    pub rendered_text_lines: Vec<String>,
    /// Last observed mouse cursor position from `MouseEventKind::Moved`.
    pub hover_x: u16,
    pub hover_y: u16,
    /// Inner content width (cols) of the input box from the last render.
    /// Soft-wrap on typed chars uses this to know when to break.
    pub input_inner_w: u16,
}

/// Returned by event handlers to tell the main loop whether to keep
/// running or shut down cleanly.
#[derive(PartialEq)]
pub enum AppAction {
    Continue,
    Quit,
}

/// Which keymap is active. The dispatcher in `event.rs::handle_event`
/// reads this to route key events to the right handler; the popup
/// renderers in `ui::popups` use it to decide what to draw on top of
/// the chat.
#[derive(Clone, PartialEq)]
pub enum Mode {
    Chat,
    ModelPicker,
    Confirm,
    /// Asking for a BYOK API key (e.g. z.ai). Use `add_step` to track which
    /// step of the add-model flow we're on.
    AddModel,
    /// Listing saved chat sessions; Up/Down navigate, Enter loads the
    /// highlighted session.
    SessionPicker,
    /// Listing currently-connected BYOK providers for removal; Up/Down
    /// navigate, Enter disconnects the highlighted provider, Esc cancels.
    DisconnectPicker,
    /// Two-step Telegram pairing wizard: paste token, then paste the
    /// code the bot DM's you. Step is tracked via
    /// [`TelegramSetupStep`].
    TelegramSetup,
    /// `/agents add` / `/agents edit` wizard — four steps walking the
    /// user through name → model → task description → system prompt.
    /// Step tracked via [`AgentsSetupStep`].
    AgentsSetup,
}

/// Which step of the `/telegram` wizard is showing. Token first, then
/// Pair once getMe succeeds. Modal dismisses itself on a successful
/// pair; failure paths set an error on App and stay on the same step.
#[derive(Clone, Copy, PartialEq)]
pub enum TelegramSetupStep {
    /// Step 1 — paste the `@BotFather` token. Submit triggers async
    /// `getMe`; success advances to `Pair`.
    Token,
    /// Step 2 — wait for the user to DM the bot, then paste the 6-char
    /// code it replied with. Submit redeems via `try_telegram_pair`.
    Pair,
}

/// Generic "list + cursor" pair used by every navigable picker in the
/// UI: model picker, session picker, disconnect picker, …
///
/// Replaces the previous habit of carrying a parallel `<thing>_items:
/// Vec<T>` and `<thing>_index: usize` pair on `App` for each picker.
/// Methods clamp + bounds-check, so callers don't reimplement the
/// "don't crash on empty list" guard every time.
pub struct Picker<T> {
    pub items: Vec<T>,
    pub index: usize,
}

impl<T> Default for Picker<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            index: 0,
        }
    }
}

impl<T> Picker<T> {
    pub fn set_items(&mut self, items: Vec<T>) {
        self.items = items;
        self.index = 0;
    }
    pub fn select_next(&mut self) {
        if self.index + 1 < self.items.len() {
            self.index += 1;
        }
    }
    pub fn select_prev(&mut self) {
        if self.index > 0 {
            self.index -= 1;
        }
    }
    pub fn select_first(&mut self) {
        self.index = 0;
    }
    pub fn select_last(&mut self) {
        self.index = self.items.len().saturating_sub(1);
    }
    pub fn selected(&self) -> Option<&T> {
        self.items.get(self.index)
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// Single-source-of-truth for "what's the agent doing right now?".
/// Replaces the old `generating` / `compacting` bool pair plus the two
/// `JoinHandle` Options and the stranded `pending_after_compact` field
/// — those used to be five fields that callers had to keep in sync
/// manually, and a panicked task could leave the bools out of sync
/// with the handles.
///
/// The three variants are mutually exclusive: an agent turn can't run
/// while a compaction is in flight, and vice versa (each entry point
/// already gated on that via two-bool checks). This enum makes the
/// invariant a type-level fact.
pub enum TurnState {
    /// Nothing in flight. The chat is accepting keypresses and the
    /// renderer skips the streaming-only paths.
    Idle,
    /// An assistant turn is streaming. The contained `JoinHandle` is
    /// the abort target for `cancel()`. Cleared back to `Idle` on
    /// `on_done` / `on_error` / `cancel`.
    Generating { task: tokio::task::JoinHandle<()> },
    /// A `/compact` call (manual or auto-triggered) is running. The
    /// optional `pending_user` is a user message that was buffered
    /// because the compaction was triggered by it — replayed via
    /// `send_to_llm` when the summary arrives.
    Compacting {
        task: tokio::task::JoinHandle<()>,
        pending_user: Option<String>,
    },
}

impl TurnState {
    pub fn is_idle(&self) -> bool {
        matches!(self, TurnState::Idle)
    }
    pub fn is_generating(&self) -> bool {
        matches!(self, TurnState::Generating { .. })
    }
    pub fn is_compacting(&self) -> bool {
        matches!(self, TurnState::Compacting { .. })
    }
    pub fn is_busy(&self) -> bool {
        !self.is_idle()
    }
}

/// Pagination state for `/load`-ed session history. Replaces the
/// `loading_more` + `no_more_history` bool pair so callers can't get
/// into the impossible "loading and exhausted at the same time" state.
///
/// State transitions:
///   - `Idle`     — nothing in flight, more history might exist
///   - `Loading`  — a `/more` request is in flight (manual or auto)
///   - `Exhausted` — server told us no older messages exist
///
/// Resets to `Idle` on `/clear`, `/new`, and `/load` (each starts a
/// fresh pagination cursor).
#[derive(Clone, Copy, PartialEq)]
pub enum PageState {
    Idle,
    Loading,
    Exhausted,
}

impl PageState {
    pub fn is_loading(self) -> bool {
        matches!(self, PageState::Loading)
    }
    pub fn is_exhausted(self) -> bool {
        matches!(self, PageState::Exhausted)
    }
}

/// Steps in the `/agents add` (and `/agents edit`) wizard. Linear: each
/// Enter advances to the next step; Esc cancels the whole flow. The
/// add path starts at `Template` (pre-fills name/task/prompt from a
/// canned recipe); the edit path skips `Template` and starts at `Name`
/// since you don't re-template-ize an existing specialist.
#[derive(Clone, Copy, PartialEq)]
pub enum AgentsSetupStep {
    /// Step 1 (add only) — pick a canned recipe ("code-reviewer",
    /// "planner", etc.) that pre-fills name/task/prompt. Pick "blank"
    /// to opt out and fill everything by hand.
    Template,
    /// Pick a short slug (`coder`, `reviewer`, etc.). Used by
    /// `/ask <name>` and as the `consult_specialist` argument.
    Name,
    /// Pick a model. Reuses the existing picker entries (Ollama + BYOK)
    /// via a list rendered inline in the wizard.
    Model,
    /// One-line "use this when …" description. Shown in `/agents list`
    /// + (phase 2) fed into the consult tool description.
    Task,
    /// Full system prompt (multi-line textarea allowed).
    Prompt,
}

/// One row in the `/disconnect` picker — a currently-connected BYOK
/// provider plus a short preview of the models that will be removed
/// alongside its API key.
#[derive(Clone)]
pub struct DisconnectEntry {
    /// Provider identifier (e.g. `"zai-subscription"`).
    pub provider: String,
    /// Pretty label shown in the popup (e.g. `"z.ai subscription"`).
    pub label: String,
    /// Three-or-fewer model names + a "+N more" suffix when the provider
    /// seeds a longer catalog. Lets the user see what they're about to
    /// drop before pressing Enter.
    pub preview: String,
}

/// What the `/model` picker can display. The picker mixes Ollama-discovered
/// models with BYOK extras and trailing "Add …" action rows (one per
/// unconfigured provider).
#[derive(Clone)]
pub enum PickerEntry {
    Ollama(String),
    Extra(ExtraModel),
    /// "+ Add <provider> key" row — appears only when the matching
    /// `byok_keys` entry isn't set. Provider id matches the
    /// `*_PROVIDER` constants in `crate::config`. Replaces the old
    /// AddZaiSubscription/AddZaiUsage/AddOllamaCloud/AddOpenCode/AddOpenRouter
    /// per-provider variants — new providers can be added to
    /// `crate::config::BYOK_PROVIDERS` and they'll surface here
    /// automatically.
    AddProvider(String),
}

impl PickerEntry {
    pub fn display(&self) -> String {
        match self {
            PickerEntry::Ollama(name) => name.clone(),
            PickerEntry::Extra(m) => format!("[{}] {}", m.provider, m.name),
            PickerEntry::AddProvider(p) => {
                format!("+ Add {} key", crate::config::provider_label(p))
            }
        }
    }
}
