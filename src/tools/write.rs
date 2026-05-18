//! Mutating filesystem tools: `edit_file` (surgical) + `write_file` (whole-file).
//!
//! Both require user approval. The approval popup carries a coloured diff
//! preview built by `super::diff::{diff_edit, diff_write}`.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use super::diff::{diff_edit, diff_stats, diff_write};
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
    let content =
        String::from_utf8(bytes).map_err(|_| anyhow!("edit_file: {} is not valid UTF-8", path))?;

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

    // Compute the diff first so the prompt can use `+NL -NL` line totals
    // instead of raw byte counts — much easier for the user to size up.
    let diff = diff_edit(old_string, new_string);
    let (added, removed) = diff_stats(&diff);
    let prompt = format!("Edit file {} (+{added}L -{removed}L)", resolved.display(),);
    if !confirm(ctx, prompt, diff).await? {
        return Ok("(user denied this edit)".into());
    }

    let updated = content.replacen(old_string, new_string, 1);
    tokio::fs::write(&resolved, updated.as_bytes()).await?;
    Ok(format!("edited {} (1 replacement)", path))
}

/// Batched surgical edit: apply N `{old_string, new_string}` pairs to the
/// same file in one call, one approval, one unified diff. Mirrors Claude
/// Code's `MultiEdit` so models trained on those traces reach for it
/// naturally instead of firing N separate `edit_file` calls.
///
/// Apply order matters — each `old_string` is matched against the file
/// state *after* the previous edits in the batch. All-or-nothing: if any
/// edit fails validation (snippet missing, ambiguous, empty, or a no-op),
/// nothing is written and the model gets a clear error pointing at the
/// failing index.
pub(super) async fn tool_multi_edit(args: &Value, ctx: &ToolContext) -> Result<String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("multi_edit requires 'path'"))?;
    let edits = args
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("multi_edit requires 'edits' array"))?;
    if edits.is_empty() {
        bail!("multi_edit: 'edits' must contain at least one {{old_string, new_string}} pair");
    }

    let resolved = resolve_in_workspace(&ctx.workspace, path)?;
    let bytes = tokio::fs::read(&resolved).await?;
    let original =
        String::from_utf8(bytes).map_err(|_| anyhow!("multi_edit: {} is not valid UTF-8", path))?;

    // Apply in-memory first so a mid-batch failure leaves the file on disk
    // untouched. Each edit's `old_string` is re-matched against the running
    // `current` buffer, not the original — that's the contract that lets
    // later edits target text that earlier edits produced.
    let mut current = original.clone();
    for (i, edit) in edits.iter().enumerate() {
        let old = edit
            .get("old_string")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("multi_edit: edit #{i} missing 'old_string'"))?;
        let new = edit
            .get("new_string")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("multi_edit: edit #{i} missing 'new_string'"))?;
        if old.is_empty() {
            bail!("multi_edit: edit #{i} has empty 'old_string' — use write_file to create a file");
        }
        if old == new {
            bail!("multi_edit: edit #{i} 'old_string' and 'new_string' are identical (no-op)");
        }
        if old.len() > MAX_WRITE_BYTES || new.len() > MAX_WRITE_BYTES {
            bail!("multi_edit: edit #{i} strings exceed {MAX_WRITE_BYTES} byte cap");
        }
        let matches = current.matches(old).count();
        if matches == 0 {
            bail!(
                "multi_edit: edit #{i} 'old_string' not found in {} (after prior edits). \
                 Either the snippet doesn't exist or a previous edit in this batch already \
                 rewrote that region — read the file and rebuild the batch.",
                path
            );
        }
        if matches > 1 {
            bail!(
                "multi_edit: edit #{i} 'old_string' appears {matches} times in {}. Expand the \
                 snippet with surrounding context until it's unique.",
                path
            );
        }
        current = current.replacen(old, new, 1);
    }

    // One confirm popup, one cumulative diff against the on-disk original.
    let diff = diff_write(Some(&original), &current);
    let (added, removed) = diff_stats(&diff);
    let prompt = format!(
        "Multi-edit {} (+{added}L -{removed}L · {} edits)",
        resolved.display(),
        edits.len()
    );
    if !confirm(ctx, prompt, diff).await? {
        return Ok("(user denied this multi-edit)".into());
    }

    tokio::fs::write(&resolved, current.as_bytes()).await?;
    Ok(format!(
        "edited {} ({} replacements applied atomically)",
        path,
        edits.len()
    ))
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
    // Compute the diff first so the prompt summary uses line counts — same
    // `+NL -NL` shape as edit_file. For a CREATE the removed count is 0.
    let diff = diff_write(prev_contents.as_deref(), content);
    let (added, removed) = diff_stats(&diff);
    let action = if prev_contents.is_some() {
        "OVERWRITE"
    } else {
        "CREATE"
    };
    let prompt = format!("{} {} (+{added}L -{removed}L)", action, resolved.display());
    if !confirm(ctx, prompt, diff).await? {
        return Ok("(user denied this write)".into());
    }

    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&resolved, content.as_bytes()).await?;
    Ok(format!("wrote {} ({} bytes)", path, content.len()))
}
