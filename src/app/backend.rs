//! Backend selection — Ollama for local models, OpenAI-compat for BYOK
//! providers. Routing is driven by `App.selected_extra`: when it's `Some`,
//! the provider field decides which URL + key combo to use; when it's
//! `None`, we're on Ollama.

use crate::config::{
    ExtraModel, OLLAMA_CLOUD_BASE, OLLAMA_CLOUD_MODELS, OLLAMA_CLOUD_PROVIDER, OPENCODE_BASE,
    OPENCODE_MODELS, OPENCODE_PROVIDER, ZAI_MODELS, ZAI_SUBSCRIPTION_BASE,
    ZAI_SUBSCRIPTION_PROVIDER, ZAI_USAGE_BASE, ZAI_USAGE_PROVIDER,
};
use crate::ollama::Client;
use crate::openai_compat;

use super::App;

/// Backend that fulfills a chat turn. Ollama for local models, OpenAI-compat
/// for any BYOK provider (z.ai subscription, z.ai usage-based, etc).
#[derive(Clone)]
pub enum LlmBackend {
    Ollama(Client),
    OpenAi(openai_compat::Client),
}

impl App {
    /// Host URL that the header should show — reflects the actual backend
    /// for the current model, not just the Ollama URL.
    pub fn current_host(&self) -> &str {
        match self.selected_extra.as_ref().map(|e| e.provider.as_str()) {
            Some(ZAI_SUBSCRIPTION_PROVIDER) => ZAI_SUBSCRIPTION_BASE,
            Some(ZAI_USAGE_PROVIDER) => ZAI_USAGE_BASE,
            Some(OLLAMA_CLOUD_PROVIDER) => OLLAMA_CLOUD_BASE,
            Some(OPENCODE_PROVIDER) => OPENCODE_BASE,
            _ => &self.client.base,
        }
    }

    /// Build the LLM backend to use for the current `self.model`.
    pub fn make_backend(&self) -> Option<LlmBackend> {
        let Some(em) = self.selected_extra.as_ref() else {
            return Some(LlmBackend::Ollama(self.client.clone()));
        };
        match em.provider.as_str() {
            ZAI_SUBSCRIPTION_PROVIDER => {
                let key = self.zai_api_key.clone()?;
                Some(LlmBackend::OpenAi(openai_compat::Client::new(
                    ZAI_SUBSCRIPTION_BASE.to_string(),
                    key,
                )))
            }
            ZAI_USAGE_PROVIDER => {
                let key = self.zai_usage_api_key.clone()?;
                Some(LlmBackend::OpenAi(openai_compat::Client::new(
                    ZAI_USAGE_BASE.to_string(),
                    key,
                )))
            }
            OLLAMA_CLOUD_PROVIDER => {
                let key = self.ollama_cloud_api_key.clone()?;
                // Cloud Ollama speaks the same native protocol as local
                // Ollama; only auth + host differ. Reuse the Ollama backend
                // variant with a Bearer-authed client.
                Some(LlmBackend::Ollama(Client::with_api_key(
                    OLLAMA_CLOUD_BASE.to_string(),
                    key,
                )))
            }
            OPENCODE_PROVIDER => {
                let key = self.opencode_api_key.clone()?;
                Some(LlmBackend::OpenAi(openai_compat::Client::new(
                    OPENCODE_BASE.to_string(),
                    key,
                )))
            }
            _ => None,
        }
    }

    /// Public bridge for main.rs to migrate older configs at startup.
    /// Idempotent: re-applies known models to extra_models for every
    /// configured provider key, and rewrites the legacy `"zai"` provider
    /// string to `"zai-subscription"` so old configs keep working.
    pub fn ensure_zai_models_pub(&mut self) {
        self.migrate_legacy_zai_provider();
        if self.zai_api_key.is_some() {
            self.ensure_zai_models_for(ZAI_SUBSCRIPTION_PROVIDER);
        }
        if self.zai_usage_api_key.is_some() {
            self.ensure_zai_models_for(ZAI_USAGE_PROVIDER);
        }
        if self.ollama_cloud_api_key.is_some() {
            self.ensure_ollama_cloud_models();
        }
        if self.opencode_api_key.is_some() {
            self.ensure_opencode_models();
        }
        self.persist_config();
    }

    /// Make sure all hardcoded z.ai models exist in `extra_models` for the
    /// given provider. Idempotent — no duplicates per provider.
    pub(super) fn ensure_zai_models_for(&mut self, provider: &str) {
        for name in ZAI_MODELS {
            let exists = self
                .extra_models
                .iter()
                .any(|m| m.provider == provider && m.name == *name);
            if !exists {
                self.extra_models.push(ExtraModel {
                    provider: provider.to_string(),
                    name: (*name).to_string(),
                });
            }
        }
    }

    /// Populate the canonical `OLLAMA_CLOUD_MODELS` lineup under the
    /// `ollama-cloud` provider. Unlike `ensure_zai_models_for`, this REPLACES
    /// any existing `ollama-cloud` entries — the cloud catalog has shipped
    /// paid-only models under names we previously seeded as free, and stale
    /// entries from earlier releases would otherwise leave broken picker
    /// rows that 403 on click. Treating `OLLAMA_CLOUD_MODELS` as the source
    /// of truth keeps the persisted config in sync with what the API
    /// actually serves on the free tier.
    pub(super) fn ensure_ollama_cloud_models(&mut self) {
        self.extra_models
            .retain(|m| m.provider != OLLAMA_CLOUD_PROVIDER);
        for name in OLLAMA_CLOUD_MODELS {
            self.extra_models.push(ExtraModel {
                provider: OLLAMA_CLOUD_PROVIDER.to_string(),
                name: (*name).to_string(),
            });
        }
    }

    /// Same canonical-replacement pattern as `ensure_ollama_cloud_models`:
    /// drop any existing `opencode` entries, then push the curated list
    /// from `OPENCODE_MODELS`. Keeps the picker honest if the catalog
    /// shifts between releases.
    pub(super) fn ensure_opencode_models(&mut self) {
        self.extra_models
            .retain(|m| m.provider != OPENCODE_PROVIDER);
        for name in OPENCODE_MODELS {
            self.extra_models.push(ExtraModel {
                provider: OPENCODE_PROVIDER.to_string(),
                name: (*name).to_string(),
            });
        }
    }

    /// Rewrite any `provider: "zai"` entries (pre-split config) to
    /// `"zai-subscription"`. Safe to call repeatedly.
    pub(super) fn migrate_legacy_zai_provider(&mut self) {
        for em in self.extra_models.iter_mut() {
            if em.provider == "zai" {
                em.provider = ZAI_SUBSCRIPTION_PROVIDER.to_string();
            }
        }
    }

    /// Write the current BYOK settings back to ~/.config/hmanlab/config.json.
    /// Silent on error — the running state is still consistent.
    pub(super) fn persist_config(&self) {
        let mut cfg = crate::config::load().ok().flatten().unwrap_or_default();
        cfg.zai_api_key = self.zai_api_key.clone();
        cfg.zai_usage_api_key = self.zai_usage_api_key.clone();
        cfg.ollama_cloud_api_key = self.ollama_cloud_api_key.clone();
        cfg.opencode_api_key = self.opencode_api_key.clone();
        cfg.extra_models = self.extra_models.clone();
        let _ = crate::config::save(&cfg);
    }
}
