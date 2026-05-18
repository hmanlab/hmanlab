use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, EventStream},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

mod agent;
mod api;
mod app;
mod compact;
mod config;
mod memory;
mod ollama;
mod openai_compat;
mod tools;
mod trust;
mod ui;
mod update_check;

use app::{App, AppAction, StreamMsg};

#[derive(Parser, Debug)]
#[command(
    name = "hmanlab",
    version,
    about = "hmanlab — terminal UI for Ollama, backed by hmanlab-api"
)]
struct Cli {
    /// Ollama URL. Overrides config; falls back to http://localhost:11434.
    #[arg(long, env = "OLLAMA_HOST")]
    host: Option<String>,

    #[arg(long, env = "OLLAMA_MODEL")]
    model: Option<String>,

    /// hmanlab-api URL for session persistence. Overrides config.
    #[arg(long, env = "HMANLAB_API_URL")]
    api_url: Option<String>,

    /// hmanlab API key. Overrides config; runs the first-run wizard if absent.
    #[arg(long, env = "HMANLAB_API_KEY")]
    api_key: Option<String>,

    #[arg(long, value_name = "PATH")]
    workspace: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let saved = config::load().ok().flatten().unwrap_or_default();
    let api_url = cli
        .api_url
        .or(saved.api_url.clone())
        .unwrap_or_else(|| config::DEFAULT_API_URL.to_string());

    // Resolve api_key: flag/env > config > prompt
    let api_key = match cli.api_key.or(saved.api_key.clone()) {
        Some(k) => k,
        None => {
            let cfg = config::run_setup_wizard(&api_url, saved.ollama_host.as_deref()).await?;
            cfg.api_key.expect("wizard guarantees api_key")
        }
    };

    // Re-load config so ollama_host reflects anything the wizard just wrote.
    let mut saved2 = config::load().ok().flatten().unwrap_or_default();
    // Has the user explicitly told us about Ollama — either via --host flag or
    // a saved config entry from onboarding? If neither, they skipped the
    // wizard's local-LLM step; don't auto-probe localhost behind their back.
    // They can add it later with /host <url>.
    let user_supplied_host = cli.host.clone().or(saved2.ollama_host.clone());
    let ollama_host = user_supplied_host
        .clone()
        .unwrap_or_else(|| config::DEFAULT_OLLAMA_HOST.to_string());

    let client = ollama::Client::new(ollama_host.clone());

    let workspace = match cli.workspace.clone() {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    let workspace = workspace.canonicalize().unwrap_or(workspace);

    // Pre-TUI trust prompt. Fires only when this workspace isn't already
    // on the persisted trusted list, so repeat launches in the same folder
    // don't re-ask. We mutate `saved2` and re-save so the new entry sticks.
    let workspace_str = workspace.display().to_string();
    let already_trusted = saved2
        .trusted_workspaces
        .iter()
        .any(|p| p == &workspace_str);
    if !already_trusted {
        match trust::prompt_workspace_trust(&workspace) {
            Ok(true) => {
                saved2.trusted_workspaces.push(workspace_str.clone());
                // Best-effort persist — if save fails, the trust still
                // applies for this session via the in-memory list below.
                if let Err(e) = config::save(&saved2) {
                    eprintln!("warn: failed to persist trust decision: {e}");
                }
            }
            Ok(false) => {}
            Err(e) => {
                // Don't block startup on a prompt error — just leave
                // the workspace untrusted and continue into the TUI.
                eprintln!("warn: trust prompt failed: {e}");
            }
        }
    }

    let models = if user_supplied_host.is_some() {
        client.list_models().await.unwrap_or_default()
    } else {
        Vec::new()
    };

    let api_client_built = api::Client::new(api_url.clone(), api_key.clone());
    let (api_client, api_tx, api_warning) = match api_client_built.check_auth().await {
        Ok(()) => {
            let (tx, rx) = mpsc::unbounded_channel();
            tokio::spawn(api::run_writer(api_client_built.clone(), rx));
            (Some(api_client_built), Some(tx), None)
        }
        Err(e) => (None, None, Some(format!("hmanlab-api unreachable: {e}"))),
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Boot App with a placeholder model; we'll resolve the real default
    // below once we've loaded BYOK extras (otherwise a z.ai-only user gets
    // stuck on a bogus "llama3.2" default that Ollama can't serve).
    let mut app = App::new(client, String::new(), models, workspace, api_client, api_tx);
    // Pre-seed the sidebar's expanded set so the first paint shows one level
    // of contents under the workspace root, not just the root entry alone.
    app.seed_sidebar_top_level();

    // Carry BYOK state into the app so /model can show + use extra models.
    app.zai_api_key = saved2.zai_api_key.clone();
    app.zai_usage_api_key = saved2.zai_usage_api_key.clone();
    app.ollama_cloud_api_key = saved2.ollama_cloud_api_key.clone();
    app.opencode_api_key = saved2.opencode_api_key.clone();
    app.extra_models = saved2.extra_models.clone();
    // Workspace trust list — paths stored as canonical strings. Recompute
    // whether the current workspace sits in that list so the confirm
    // interceptor in `app::stream` can short-circuit destructive tools.
    app.trusted_workspaces = saved2
        .trusted_workspaces
        .iter()
        .map(std::path::PathBuf::from)
        .collect();
    app.workspace_trusted = app.trusted_workspaces.iter().any(|p| p == &app.workspace);
    // Re-seed now that we know the trust state — the first call above
    // ran with the default `workspace_trusted = false`, so trusted
    // workspaces wouldn't have shown their dotfile dirs at root.
    app.seed_sidebar_top_level();
    // Migrate older configs: a saved BYOK key means the matching provider's
    // models should be available, even if the user previously added only one.
    // Also rewrites legacy provider="zai" → "zai-subscription".
    if app.zai_api_key.is_some()
        || app.zai_usage_api_key.is_some()
        || app.ollama_cloud_api_key.is_some()
        || app.opencode_api_key.is_some()
    {
        app.ensure_zai_models_pub();
    }

    // Initial model resolution order:
    //   1. --model flag (explicit user intent on this launch)
    //   2. last-used model from config (persistence across restarts)
    //   3. first Ollama-discovered model
    //   4. first BYOK extra
    // We pick AFTER loading extras so a z.ai-only user lands on glm-4.7
    // instead of a doomed "llama3.2" → Ollama route.
    let last_model = saved2.last_model.as_deref();
    let last_provider = saved2.last_provider.as_deref();
    let last_extra = last_model.and_then(|name| {
        let want_provider = last_provider; // None → Ollama; Some → BYOK
        match want_provider {
            Some(prov) => app
                .extra_models
                .iter()
                .find(|m| m.name == name && m.provider == prov)
                .cloned(),
            None => None,
        }
    });
    let last_ollama = last_model.filter(|name| {
        // Saved model points at Ollama (no provider) AND the host still
        // serves it. If it was renamed/removed we fall through.
        last_provider.is_none() && app.models.iter().any(|m| m == name)
    });
    if let Some(name) = cli.model.clone() {
        app.model = name.clone();
        app.selected_extra = app.extra_models.iter().find(|m| m.name == name).cloned();
    } else if let Some(em) = last_extra {
        app.model = em.name.clone();
        app.selected_extra = Some(em);
    } else if let Some(name) = last_ollama {
        app.model = name.to_string();
        app.selected_extra = None;
    } else if let Some(name) = app.models.first().cloned() {
        app.model = name;
        app.selected_extra = None;
    } else if let Some(em) = app.extra_models.first().cloned() {
        app.model = em.name.clone();
        app.selected_extra = Some(em);
    }

    // Status reflects the COMBINED count of Ollama + BYOK models, not just
    // Ollama. A user with z.ai but no Ollama running should see "Ready",
    // not "No models".
    let total = app.models.len() + app.extra_models.len();
    let db_state = if app.api.is_some() {
        "API on"
    } else {
        "API off"
    };
    app.status = if total == 0 {
        format!(
            "No models — try /host <url> for Ollama, or /model to add a BYOK provider  ·  {db_state}"
        )
    } else {
        format!("Ready — {total} model(s)  ·  {db_state}  ·  /help for commands")
    };
    if let Some(w) = api_warning {
        app.status = format!("{w}  ·  running without persistence");
    }
    let res = run(&mut terminal, app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        eprintln!("error: {:#}", e);
        std::process::exit(1);
    }
    Ok(())
}

async fn run<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, mut app: App) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamMsg>();
    let mut events = EventStream::new();

    // Background update check. Sends a single message if (and only if) npm
    // has a version newer than the compiled-in one. Skipped on debug builds
    // and cached for 24 h, so this is at most one HTTP hit per release-day
    // per user — never blocks the TUI from starting.
    let update_tx = tx.clone();
    tokio::spawn(async move {
        if let Some(latest) = update_check::check().await {
            let _ = update_tx.send(StreamMsg::UpdateAvailable(latest));
        }
    });

    // Animation ticker: fires every 120 ms but is only polled while the agent
    // is generating or a tool is running (see the `if` guard on its select!
    // arm). Drives `app.anim_tick`, which the renderer uses to pulse the
    // breathing color on the thinking / tool-running indicators.
    let mut ticker = interval(Duration::from_millis(120));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        terminal.draw(|f| ui::render(f, &mut app))?;

        tokio::select! {
            _ = ticker.tick(), if app.generating || app.active_tool_msg_idx.is_some() => {
                app.anim_tick = app.anim_tick.wrapping_add(1);
            }
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        if matches!(app.handle_event(event, &tx).await?, AppAction::Quit) {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        app.status = format!("input error: {}", e);
                    }
                    None => break,
                }
            }
            Some(msg) = rx.recv() => {
                app.handle_stream(msg, &tx);
            }
        }
    }
    Ok(())
}
