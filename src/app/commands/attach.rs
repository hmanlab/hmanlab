//! `/attach <path>` and `/detach [name|all]` — manage image / media
//! attachments queued for the next user message.
//!
//! Attachments live in `App.pending_attachments` between commands. The
//! actual wire encoding (base64 data URL → `image_url` content part)
//! happens in `ollama::ChatMessage::to_api_content` once `start_turn`
//! drains the queue onto the user message.

use std::path::PathBuf;

use crate::ollama::Attachment;

use super::super::App;

/// Hard cap per attachment. Above this, providers either reject the
/// request outright or drop the image silently — better to fail fast
/// with a clear error than ship a payload that gets stripped server-
/// side. 20 MB matches OpenAI / Anthropic's documented per-image limit.
const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;

/// Max attachments queued at once. Token budget on the next turn
/// degrades fast past 4-5 images on most multimodal models; cap here
/// so the user gets feedback instead of a context-overflow error.
const MAX_PENDING: usize = 8;

impl App {
    pub(in crate::app) fn handle_attach(&mut self, raw: String) {
        let raw = raw.trim();
        if raw.is_empty() {
            self.push_info(
                "Usage: /attach <path>   — queue an image or file for the next message.".into(),
            );
            return;
        }

        if self.pending_attachments.len() >= MAX_PENDING {
            self.push_info(format!(
                "Already at the {MAX_PENDING}-attachment cap. Send the current message or /detach all to reset."
            ));
            return;
        }

        let path = resolve_path(raw, &self.workspace);
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                self.push_info(format!("/attach: can't read {} — {e}", path.display()));
                return;
            }
        };
        if !meta.is_file() {
            self.push_info(format!("/attach: {} is not a file.", path.display()));
            return;
        }
        if meta.len() > MAX_ATTACHMENT_BYTES {
            self.push_info(format!(
                "/attach: {} is {:.1} MB — over the 20 MB per-attachment cap.",
                path.display(),
                meta.len() as f64 / (1024.0 * 1024.0)
            ));
            return;
        }

        match Attachment::from_path(&path) {
            Ok(att) => {
                let summary = format!(
                    "✓ Attached {} ({}, {}). Send your next message to include it.",
                    att.filename,
                    att.media_type,
                    att.size_display()
                );
                self.pending_attachments.push(att);
                self.push_info(summary);
            }
            Err(e) => {
                self.push_info(format!("/attach: {e}"));
            }
        }
    }

    pub(in crate::app) fn handle_detach(&mut self, arg: String) {
        let arg = arg.trim();
        if self.pending_attachments.is_empty() {
            self.push_info("Nothing to detach — no attachments queued.".into());
            return;
        }
        if arg.is_empty() {
            let dropped = self.pending_attachments.pop().expect("non-empty");
            self.push_info(format!("✓ Detached {} (most recent).", dropped.filename));
            return;
        }
        if arg.eq_ignore_ascii_case("all") || arg == "*" {
            let n = self.pending_attachments.len();
            self.pending_attachments.clear();
            self.push_info(format!("✓ Cleared {n} queued attachment(s)."));
            return;
        }
        let lo = arg.to_lowercase();
        let pos = self
            .pending_attachments
            .iter()
            .position(|a| a.filename.to_lowercase().contains(&lo));
        match pos {
            Some(i) => {
                let dropped = self.pending_attachments.remove(i);
                self.push_info(format!("✓ Detached {}.", dropped.filename));
            }
            None => {
                self.push_info(format!(
                    "No queued attachment matches '{arg}'. Use /detach all to clear everything."
                ));
            }
        }
    }
}

/// Resolve an attach target: `~`-expansion, then absolute as-is, else
/// relative to the workspace. We do NOT canonicalize — symlinks should
/// be followed by `std::fs::read` later, and pre-canonicalizing here
/// would hide the user's original path in the chat log.
fn resolve_path(raw: &str, workspace: &std::path::Path) -> PathBuf {
    let expanded: PathBuf = if let Some(rest) = raw.strip_prefix("~/") {
        match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(rest),
            None => PathBuf::from(raw),
        }
    } else if raw == "~" {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(raw))
    } else {
        PathBuf::from(raw)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        workspace.join(expanded)
    }
}
