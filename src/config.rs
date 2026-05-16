use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

pub const DEFAULT_API_URL: &str = "https://be-ai.senireka.my";
pub const DEFAULT_OLLAMA_HOST: &str = "http://localhost:11434";

/// z.ai has two billing plans, each with its own base URL. Treated as
/// independent providers throughout the codebase so a user can hold a key
/// for one or both at the same time.
pub const ZAI_SUBSCRIPTION_BASE: &str = "https://api.z.ai/api/coding/paas/v4";
pub const ZAI_USAGE_BASE: &str = "https://api.z.ai/api/paas/v4";

/// Provider identifiers stored in `ExtraModel.provider`. Old configs that
/// say `"zai"` are migrated to `"zai-subscription"` at App init.
pub const ZAI_SUBSCRIPTION_PROVIDER: &str = "zai-subscription";
pub const ZAI_USAGE_PROVIDER: &str = "zai-usage";

/// Models exposed for each z.ai plan. Same lineup today — if z.ai diverges
/// the two lists, split this constant.
pub const ZAI_MODELS: &[&str] = &["glm-4.7", "glm-4.6", "glm-5.1"];
pub const ZAI_DEFAULT_MODEL: &str = "glm-4.7";

/// OpenCode Go — the opencode.ai subscription tier. Hits
/// `https://opencode.ai/zen/go/v1/chat/completions` with Bearer auth; the
/// same API key generated at https://opencode.ai/zen works for both Zen
/// (pay-per-credit) and Go (subscription-billed), but routing is by URL:
/// requests to `/zen/v1` bill against pay-per-credit; requests to
/// `/zen/go/v1` bill against the Go subscription. We point at the Go URL
/// because Go is what hmanlab users buying this provider are subscribed
/// to. Free-tier-only access (big-pickle, *-free) is intentionally NOT
/// served from this endpoint — those models 401 ModelError on Go and
/// require the Zen URL instead. That's a separate future provider if
/// anyone asks.
///
/// Slug convention: the wire protocol expects **bare model IDs**, not
/// `opencode-go/<id>`. The `opencode-go/` prefix in opencode.ai's docs is
/// only used by their own client's `opencode.json` config file — the
/// HTTPS API rejects it as "Model opencode-go/X not supported". Verified
/// 2026-05-16 against a real Go key.
///
/// Catalog: every Go model that responds 200 to a probe through
/// `/chat/completions`. The docs claim MiniMax requires the `/messages`
/// shape, but empirically it works through `/chat/completions` on Go, so
/// it's included. Closed-weight models (claude-*, gpt-*, gemini-*) still
/// use their own non-OpenAI endpoints (`/messages`, `/responses`,
/// `/models/...`) and are excluded.
pub const OPENCODE_PROVIDER: &str = "opencode";
pub const OPENCODE_BASE: &str = "https://opencode.ai/zen/go/v1";
pub const OPENCODE_MODELS: &[&str] = &[
    "glm-5.1",
    "glm-5",
    "qwen3.6-plus",
    "qwen3.5-plus",
    "kimi-k2.6",
    "kimi-k2.5",
    "minimax-m2.7",
    "minimax-m2.5",
];
pub const OPENCODE_DEFAULT_MODEL: &str = "glm-5.1";

/// Ollama Cloud — separate provider from local Ollama. Same wire protocol
/// (native /api/chat), but reached over HTTPS with a Bearer-auth API key
/// generated at https://ollama.com/settings/keys.
///
/// Free-tier catalog as of 2026-05-16, verified by direct API probe: most
/// Ollama Cloud models (glm-5.1, glm-5, deepseek-*, kimi, minimax) return
/// `403: this model requires a subscription`. The three below are the only
/// chat-capable ones a fresh key can actually call without upgrading. We
/// keep the list narrow so the picker never lies about what works — paid
/// models can be added manually by editing `~/.config/hmanlab/config.json`
/// once a subscription is in place.
///
/// Note: the API accepts both `glm-4.7` and `glm-4.7:cloud` — the `:cloud`
/// suffix is optional. We use bare slugs so the picker display
/// (`[ollama-cloud] glm-4.7`) reads cleanly.
pub const OLLAMA_CLOUD_PROVIDER: &str = "ollama-cloud";
pub const OLLAMA_CLOUD_BASE: &str = "https://ollama.com";
pub const OLLAMA_CLOUD_MODELS: &[&str] = &[
    "glm-4.7",
    "gpt-oss:120b-cloud",
    "qwen3-coder-next",
];
pub const OLLAMA_CLOUD_DEFAULT_MODEL: &str = "glm-4.7";

/// One user-added model from a BYOK provider. Lives in extra_models so the
/// `/model` picker can list it alongside Ollama-discovered models.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExtraModel {
    /// "zai", later "openrouter", etc. Selects which client to use.
    pub provider: String,
    /// Model name as the provider expects it (e.g. "glm-4-plus").
    pub name: String,
}

/// What's persisted in ~/.config/hmanlab/config.json.
/// BYOK keys live here too — they go to the provider directly, never to the
/// hmanlab backend.
///
/// Legacy `zai_base` from earlier versions is silently ignored (serde drops
/// unknown fields by default). The URL is now derived per-plan from the
/// `ZAI_*_BASE` constants.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Config {
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    pub ollama_host: Option<String>,
    /// z.ai SUBSCRIPTION (coding plan) key — historically called just `zai_api_key`.
    #[serde(default)]
    pub zai_api_key: Option<String>,
    /// z.ai USAGE-BASED key — pay-per-token endpoint.
    #[serde(default)]
    pub zai_usage_api_key: Option<String>,
    /// Ollama Cloud API key (Bearer auth against https://ollama.com).
    /// Independent of any local Ollama host configured via `ollama_host`.
    #[serde(default)]
    pub ollama_cloud_api_key: Option<String>,
    /// OpenCode Zen API key (Bearer auth against opencode.ai/zen/v1).
    /// Same key works for the free tier and the OpenCode Go subscription.
    #[serde(default)]
    pub opencode_api_key: Option<String>,
    #[serde(default)]
    pub extra_models: Vec<ExtraModel>,
}

pub fn path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".config/hmanlab/config.json"))
}

pub fn load() -> Result<Option<Config>> {
    let p = path()?;
    if !p.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    let c: Config = serde_json::from_str(&s).context("parse config json")?;
    Ok(Some(c))
}

pub fn save(c: &Config) -> Result<()> {
    let p = path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let s = serde_json::to_string_pretty(c)?;
    std::fs::write(&p, s).with_context(|| format!("write {}", p.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&p)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&p, perms)?;
    }
    Ok(())
}

/// Pre-TUI prompt run when no API key is available from flag/env/config.
///
/// Flow:
///   1. hmanlab API key (validated against `/v1/auth/me`).
///   2. Provider menu loop — add any combination of z.ai subscription,
///      z.ai usage-based, or a local Ollama URL. All optional.
///
/// Ollama is now offered alongside the BYOK providers rather than asked for
/// up front: most users don't want to deal with localhost defaults during a
/// fresh install, and the TUI's `/host` command can configure it later
/// anyway. If the user skips this step entirely, `ollama_host` stays unset
/// and the TUI falls back to `DEFAULT_OLLAMA_HOST` on startup (which is a
/// no-op when Ollama isn't running).
pub async fn run_setup_wizard(
    api_url: &str,
    existing_ollama: Option<&str>,
) -> Result<Config> {
    println!();
    println!("\x1b[1mWelcome to hmanlab.\x1b[0m");
    println!();
    println!(
        "Your hmanlab key authenticates the TUI to the backend and powers session storage."
    );
    println!("Get one (or sign in) at \x1b[36mhttps://hmanlab.senireka.my\x1b[0m → API keys.");
    println!();

    let api_key = loop {
        print!("Paste your hmanlab API key (bai_…): ");
        io::stdout().flush()?;
        let mut key = String::new();
        io::stdin().lock().read_line(&mut key)?;
        let key = key.trim().to_string();
        if key.is_empty() {
            println!("  (empty — try again, or Ctrl-C to quit)");
            continue;
        }
        if !key.starts_with("bai_") {
            println!("  \x1b[33m!\x1b[0m hmanlab keys start with 'bai_' — that doesn't look right. Try again or Ctrl-C to quit.");
            continue;
        }
        print!("  validating… ");
        io::stdout().flush()?;
        let c = crate::api::Client::new(api_url.to_string(), key.clone());
        match c.check_auth().await {
            Ok(()) => {
                println!("\x1b[32mok\x1b[0m");
                break key;
            }
            Err(e) => {
                println!("\x1b[31mfailed\x1b[0m ({e})");
                println!("  Double-check the key, or generate a new one at https://hmanlab.senireka.my/keys");
            }
        }
    };

    let mut cfg = Config {
        api_url: Some(api_url.to_string()),
        api_key: Some(api_key),
        ..Default::default()
    };

    println!();
    println!("Connect a provider? (optional — skip with Enter)");
    loop {
        let sub_state = if cfg.zai_api_key.is_some() { " (configured)" } else { "" };
        let usage_state = if cfg.zai_usage_api_key.is_some() { " (configured)" } else { "" };
        let ollama_state = match &cfg.ollama_host {
            Some(h) => format!(" (configured: {h})"),
            None => String::new(),
        };
        println!("  1) z.ai subscription   — {ZAI_SUBSCRIPTION_BASE}{sub_state}");
        println!("  2) z.ai usage-based    — {ZAI_USAGE_BASE}{usage_state}");
        println!("  3) local LLM (Ollama){ollama_state}");
        print!("  > ");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().lock().read_line(&mut choice)?;
        let choice = choice.trim();
        if choice.is_empty() || choice.eq_ignore_ascii_case("skip") || choice == "0" {
            break;
        }
        match choice {
            "1" => {
                cfg.zai_api_key = prompt_zai_key("subscription")?;
                if cfg.zai_api_key.is_some() {
                    println!("  \x1b[32m✓\x1b[0m z.ai subscription key saved.");
                }
            }
            "2" => {
                cfg.zai_usage_api_key = prompt_zai_key("usage-based")?;
                if cfg.zai_usage_api_key.is_some() {
                    println!("  \x1b[32m✓\x1b[0m z.ai usage-based key saved.");
                }
            }
            "3" => {
                cfg.ollama_host = prompt_ollama_host(existing_ollama)?;
                if cfg.ollama_host.is_some() {
                    println!("  \x1b[32m✓\x1b[0m Ollama URL saved.");
                }
            }
            _ => {
                println!("  (unknown — type 1, 2, 3, or Enter to skip)");
            }
        }
        println!();
    }

    save(&cfg)?;
    let p = path()?;
    println!();
    println!("\x1b[32m✓\x1b[0m Saved to {} (mode 600)", p.display());
    println!();
    Ok(cfg)
}

/// Prompt for a z.ai API key. Returns `None` if the user submits an empty
/// line (they decided to skip this provider after all).
fn prompt_zai_key(label: &str) -> Result<Option<String>> {
    print!("    Paste your z.ai {label} API key (or Enter to cancel): ");
    io::stdout().flush()?;
    let mut key = String::new();
    io::stdin().lock().read_line(&mut key)?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return Ok(None);
    }
    Ok(Some(key))
}

/// Prompt for the Ollama URL. Returns `None` if the user submits an empty
/// line with no existing default — they decided to skip Ollama after all.
/// If they have a previous setting OR they press Enter when the localhost
/// default is shown, we keep that value.
fn prompt_ollama_host(existing: Option<&str>) -> Result<Option<String>> {
    let default = existing.unwrap_or(DEFAULT_OLLAMA_HOST);
    print!("    Ollama URL [{default}] (or 'skip' to cancel): ");
    io::stdout().flush()?;
    let mut h = String::new();
    io::stdin().lock().read_line(&mut h)?;
    let h = h.trim();
    if h.eq_ignore_ascii_case("skip") {
        return Ok(None);
    }
    if h.is_empty() {
        return Ok(Some(default.to_string()));
    }
    Ok(Some(h.to_string()))
}
