//! Agent tool surface.
//!
//! The model talks to this module via two entry points:
//!   - `tool_definitions()` — JSON schemas Ollama serialises into its tool API
//!   - `execute_tool(name, args, ctx)` — dispatch into the right `tool_*` impl
//!
//! Each implementation lives in a subfile grouped by intent (read, git, write,
//! shell). Confirmation flow, diff preview, and workspace-path safety are
//! shared utilities under the same tree.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};

mod definitions;
mod diff;
mod git;
mod memory_tools;
mod read;
mod shell;
mod workspace;
mod write;

pub use definitions::{system_prompt, tool_definitions};
pub use diff::{DiffLine, DiffLineKind};

/// Request the user confirm before running something risky.
///
/// `prompt` is a short, plain-text headline (still used by run_command).
/// `diff`, when non-empty, is rendered as a coloured diff under the
/// headline — that's how edit_file / write_file show what they'd change.
pub struct ConfirmRequest {
    pub prompt: String,
    pub diff: Vec<DiffLine>,
    pub responder: oneshot::Sender<bool>,
}

/// Per-invocation context handed to each tool.
pub struct ToolContext {
    pub workspace: PathBuf,
    /// Channel back to the UI to ask the user for confirmation.
    pub confirm_tx: mpsc::UnboundedSender<ConfirmRequest>,
}

/// Send a confirmation request to the UI and wait for the user's y/n.
/// Returns `Ok(false)` on both "deny" and "UI channel dropped".
pub(super) async fn confirm(
    ctx: &ToolContext,
    prompt: String,
    diff: Vec<DiffLine>,
) -> Result<bool> {
    let (tx, rx) = oneshot::channel::<bool>();
    ctx.confirm_tx
        .send(ConfirmRequest { prompt, diff, responder: tx })
        .map_err(|_| anyhow!("UI channel closed before confirmation"))?;
    Ok(rx.await.unwrap_or(false))
}

/// Resolve a model-emitted tool name to a canonical hmanlab tool name.
///
/// Public v0.1 was fine-tuned on agentic traces that use Claude-Code-style
/// TitleCase names (`Read`, `Glob`, `Bash`, ...), not hmanlab's snake_case
/// surface. When such a model emits its trained name, route it to the matching
/// hmanlab handler instead of failing with "unknown tool". Snake_case names
/// pass through unchanged.
fn resolve_tool_alias(name: &str) -> &str {
    match name {
        "Read" => "read_file",
        "LS" | "List" => "list_dir",
        "Glob" => "find_files",
        "Bash" | "Shell" => "run_command",
        "Edit" => "edit_file",
        "Write" => "write_file",
        other => other,
    }
}

pub async fn execute_tool(name: &str, args: &Value, ctx: &ToolContext) -> Result<String> {
    let canonical = resolve_tool_alias(name);
    match canonical {
        "read_file" => read::tool_read_file(args, ctx).await,
        "list_dir" => read::tool_list_dir(args, ctx).await,
        "find_files" => read::tool_find_files(args, ctx).await,
        "git_status" => git::run_git(ctx, &["status", "--porcelain=v1", "-b"]).await,
        "git_log" => {
            let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(10).clamp(1, 100);
            git::run_git_owned(
                ctx,
                vec!["log".into(), "--oneline".into(), "-n".into(), limit.to_string()],
            )
            .await
        }
        "git_diff" => git::tool_git_diff(args, ctx).await,
        "git_show" => {
            let rev = args
                .get("rev")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("git_show requires 'rev'"))?;
            if !rev.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | '~' | '^')
            }) {
                bail!("rev contains disallowed characters");
            }
            git::run_git(ctx, &["show", "--stat", rev]).await
        }
        "edit_file" => write::tool_edit_file(args, ctx).await,
        "write_file" => write::tool_write_file(args, ctx).await,
        "run_command" => shell::tool_run_command(args, ctx).await,
        "save_memory" => memory_tools::tool_save_memory(args, ctx).await,
        "read_memory" => memory_tools::tool_read_memory(args, ctx).await,
        "forget_memory" => memory_tools::tool_forget_memory(args, ctx).await,
        other => bail!("unknown tool: {other}"),
    }
}
