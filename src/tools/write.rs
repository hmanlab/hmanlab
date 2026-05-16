//! Mutating filesystem tools: `edit_file` (surgical) + `write_file` (whole-file).
//!
//! Both require user approval. The approval popup carries a coloured diff
//! preview built by `super::diff::{diff_edit, diff_write}`.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use super::diff::{diff_edit, diff_write};
use super::workspace::resolve_in_workspace;
use super::{confirm, ToolContext};

/// Hard cap on `write_file` content + `edit_file` strings. Above this the
/// prompt and history get unwieldy and the model is almost certainly going
/// wrong anyway. 500 KB is enough for any single source file we'd realistically
/// edit; bigger pastes belong in a separate file-management workflow.
const MAX_WRITE_BYTES: usize = 500_000;

pub(super) async fn tool_edit_file(args: &Value, ctx: &ToolContext) -> Result<String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("edit_file requires 'path'"))?;
    let old_string = args
        .get("old_string")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("edit_file requires 'old_string'"))?;
    let new_string = args
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("edit_file requires 'new_string'"))?;

    if old_string.is_empty() {
        bail!("edit_file: 'old_string' cannot be empty — use write_file to create a file");
    }
    if old_string == new_string {
        bail!("edit_file: 'old_string' and 'new_string' are identical");
    }
    if old_string.len() > MAX_WRITE_BYTES || new_string.len() > MAX_WRITE_BYTES {
        bail!("edit_file: strings exceed {MAX_WRITE_BYTES} byte cap");
    }

    let resolved = resolve_in_workspace(&ctx.workspace, path)?;
    let bytes = tokio::fs::read(&resolved).await?;
    let content = String::from_utf8(bytes)
        .map_err(|_| anyhow!("edit_file: {} is not valid UTF-8", path))?;

    let matches = content.matches(old_string).count();
    if matches == 0 {
        bail!(
            "edit_file: 'old_string' not found in {}. Read the file first to confirm exact text \
             (whitespace, tabs vs spaces, trailing newlines all count).",
            path
        );
    }
    if matches > 1 {
        bail!(
            "edit_file: 'old_string' appears {matches} times in {}. Expand the snippet with \
             surrounding context until it's unique.",
            path
        );
    }

    let prompt = format!(
        "Edit file {} ({} → {} bytes)",
        resolved.display(),
        content.len(),
        content.len() - old_string.len() + new_string.len(),
    );
    let diff = diff_edit(old_string, new_string);
    if !confirm(ctx, prompt, diff).await? {
        return Ok("(user denied this edit)".into());
    }

    let updated = content.replacen(old_string, new_string, 1);
    tokio::fs::write(&resolved, updated.as_bytes()).await?;
    Ok(format!("edited {} (1 replacement)", path))
}

pub(super) async fn tool_write_file(args: &Value, ctx: &ToolContext) -> Result<String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("write_file requires 'path'"))?;
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("write_file requires 'content'"))?;

    if content.len() > MAX_WRITE_BYTES {
        bail!(
            "write_file: content is {} bytes; cap is {}. Split into multiple files or \
             reduce.",
            content.len(),
            MAX_WRITE_BYTES
        );
    }

    let resolved = resolve_in_workspace(&ctx.workspace, path)?;
    // resolve_in_workspace returns a path even if the file doesn't exist yet
    // (parent must exist). exists() lets us tell create vs overwrite in the
    // approval prompt — useful when the model thinks it's making a new file
    // but is actually about to clobber one.
    let prev_contents: Option<String> = if resolved.exists() {
        match tokio::fs::read(&resolved).await {
            Ok(bytes) => String::from_utf8(bytes).ok(),
            Err(_) => None,
        }
    } else {
        None
    };
    let (action, byte_summary) = match &prev_contents {
        Some(prev) => (
            "OVERWRITE",
            format!("{} → {} bytes", prev.len(), content.len()),
        ),
        None => ("CREATE", format!("{} bytes", content.len())),
    };
    let prompt = format!("{} {} ({})", action, resolved.display(), byte_summary);
    let diff = diff_write(prev_contents.as_deref(), content);
    if !confirm(ctx, prompt, diff).await? {
        return Ok("(user denied this write)".into());
    }

    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&resolved, content.as_bytes()).await?;
    Ok(format!("wrote {} ({} bytes)", path, content.len()))
}
