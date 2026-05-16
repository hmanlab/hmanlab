//! `run_command` — shell escape hatch. User-approval required per call.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command as TokioCmd;

use super::workspace::{truncate_utf8, MAX_CMD_BYTES};
use super::{confirm, ToolContext};

const CMD_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) async fn tool_run_command(args: &Value, ctx: &ToolContext) -> Result<String> {
    let cmd = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("run_command requires 'command'"))?
        .to_string();

    let prompt = format!(
        "Run shell command in {}:\n  {}",
        ctx.workspace.display(),
        cmd
    );
    if !confirm(ctx, prompt, Vec::new()).await? {
        return Ok("(user denied this command)".into());
    }

    let fut = TokioCmd::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(&ctx.workspace)
        .output();
    let output = match tokio::time::timeout(CMD_TIMEOUT, fut).await {
        Ok(r) => r?,
        Err(_) => bail!("command timed out after {} seconds", CMD_TIMEOUT.as_secs()),
    };

    let mut text = String::new();
    if !output.stdout.is_empty() {
        text.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("[stderr]\n");
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    text.push_str(&format!(
        "\n[exit {}]",
        output.status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".into())
    ));
    Ok(truncate_utf8(text, MAX_CMD_BYTES))
}
