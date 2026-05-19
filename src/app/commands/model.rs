//! Model selection + BYOK key-entry flows.
//!
//! Covers:
//!   - `/model <name>` — fuzzy-pick from the Ollama-discovered list.
//!   - `/models` — print the discovered list inline.
//!   - The Ctrl+M picker (rebuild entries + open in picker mode).
//!   - The "+ Add … key" flow for BYOK providers (z.ai, Ollama Cloud,
//!     OpenCode) and the modal that collects the key.
//!   - `persist_last_model` — best-effort write so the next launch boots
//!     into the user's last choice instead of the default.
//!
//! Picker entries are rebuilt every time the modal opens, not cached, so
//! "+ Add … key" rows disappear the moment the key is saved.
//!
//! For BYOK providers, saving a key also auto-switches the active model to
//! that provider's `*_DEFAULT_MODEL` so the user can start chatting
//! immediately — no need to re-open the picker.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;
use tui_textarea::Input;

use super::super::{fresh_textarea, App, AppAction, Mode, PickerEntry, StreamMsg};

impl App {
    pub(in crate::app) fn rebuild_picker_entries(&mut self) {
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
        for provider in crate::config::BYOK_PROVIDERS {
            if !self.has_byok_key(provider) {
                entries.push(PickerEntry::AddProvider((*provider).to_string()));
            }
        }
        self.model_picker.set_items(entries);
    }

    pub(in crate::app) fn open_picker(&mut self) {
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
        self.model_picker.index = self
            .model_picker
            .items
            .iter()
            .position(|e| match e {
                PickerEntry::Ollama(n) => active_provider.is_none() && n == &self.model,
                PickerEntry::Extra(m) => {
                    active_provider.is_some_and(|p| p == m.provider) && m.name == self.model
                }
                PickerEntry::AddProvider(_) => false,
            })
            .unwrap_or(0);
    }

    /// Begin the "+ Add … key" flow for the given provider. Single step:
    /// collect the API key. After save, all known models for that provider
    /// become available in the picker.
    pub(in crate::app) fn begin_add_model(&mut self, provider: &str) {
        self.add_model_provider = provider.to_string();
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
            p if p == crate::config::OPENROUTER_PROVIDER => (
                "Paste your OpenRouter API key (from https://openrouter.ai/settings/keys)",
                "OpenRouter",
            ),
            _ => ("Paste your z.ai coding-plan API key", "z.ai subscription"),
        };
        self.add_model_input.set_placeholder_text(placeholder);
        self.mode = Mode::AddModel;
        self.status = format!("Adding {label} key — Esc to cancel");
    }

    pub(in crate::app) fn handle_add_model(
        &mut self,
        key: KeyEvent,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) -> AppAction {
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
                // Store the key first; per-provider "seed the model list"
                // dispatch is independent and only differs in what list to
                // seed and which default model to switch to.
                self.set_byok_key(&provider, val);
                // `default_model` is the model the active selection switches
                // to after saving — chosen per-provider so the user can chat
                // immediately without re-opening the picker.
                let (label, default_model) = match provider.as_str() {
                    p if p == crate::config::ZAI_USAGE_PROVIDER => {
                        self.ensure_zai_models_for(&provider);
                        ("z.ai usage-based", crate::config::ZAI_DEFAULT_MODEL)
                    }
                    p if p == crate::config::OLLAMA_CLOUD_PROVIDER => {
                        self.ensure_ollama_cloud_models();
                        ("Ollama Cloud", crate::config::OLLAMA_CLOUD_DEFAULT_MODEL)
                    }
                    p if p == crate::config::OPENCODE_PROVIDER => {
                        self.ensure_opencode_models();
                        ("OpenCode", crate::config::OPENCODE_DEFAULT_MODEL)
                    }
                    p if p == crate::config::OPENROUTER_PROVIDER => {
                        self.ensure_openrouter_models();
                        ("OpenRouter", crate::config::OPENROUTER_DEFAULT_MODEL)
                    }
                    _ => {
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
                // For OpenRouter, immediately try to pull the live model
                // catalog — the static seed in OPENROUTER_MODELS is just a
                // first-launch fallback. Silent failure is fine (network
                // blip, no_proxy, etc.); the user keeps the seeded set.
                if provider == crate::config::OPENROUTER_PROVIDER {
                    self.refresh_openrouter_models(tx);
                }
                return AppAction::Continue;
            }
            _ => {}
        }
        let input: Input = key.into();
        self.add_model_input.input(input);
        AppAction::Continue
    }

    pub(in crate::app) fn switch_model(&mut self, name: &str) {
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
            let _ = persist_last_model(&self.model, None);
        }
    }

    pub(in crate::app) fn list_models_inline(&mut self) {
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
}

/// Persist the user's currently-selected model so the next launch boots
/// straight into it. `provider = None` means Ollama; otherwise it's the
/// `ExtraModel::provider` tag. Best-effort — a write failure is logged to
/// the chat as info but never aborts the model switch itself.
pub(in crate::app) fn persist_last_model(
    model: &str,
    provider: Option<&str>,
) -> anyhow::Result<()> {
    let mut cfg = crate::config::load()?.unwrap_or_default();
    cfg.last_model = Some(model.to_string());
    cfg.last_provider = provider.map(|s| s.to_string());
    crate::config::save(&cfg)
}
