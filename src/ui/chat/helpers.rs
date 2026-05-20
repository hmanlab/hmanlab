//! Pure helpers shared by `messages` and `input` — no `App` mutation, no
//! `Frame` access. Keeping them here means the render functions in the
//! sibling modules stay focused on layout, and these can be unit-tested
//! (or repurposed) in isolation.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

/// Period of one full breath, in animation ticks. The ticker fires every
/// 120 ms (see `main::run`), so 30 ticks ≈ 3.6 s — slow enough to read as
/// breathing rather than blinking.
const BREATH_PERIOD: u64 = 30;

/// Sine-interpolate between two RGB colors using `tick` as phase. Returns
/// `lo` at the trough and `hi` at the peak of each breath cycle.
fn breath_color(tick: u64, lo: (u8, u8, u8), hi: (u8, u8, u8)) -> Color {
    let phase = (tick % BREATH_PERIOD) as f32 / BREATH_PERIOD as f32 * std::f32::consts::TAU;
    let t = (phase.sin() * 0.5) + 0.5;
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    Color::Rgb(lerp(lo.0, hi.0), lerp(lo.1, hi.1), lerp(lo.2, hi.2))
}

/// Sky-tinted breath used for the "thinking" indicator. Pulses between a
/// muted version of the sky-blue role color and the full sky color.
pub(super) fn thinking_breath(tick: u64) -> Color {
    breath_color(tick, (55, 90, 105), (137, 220, 235))
}

/// Peach-tinted breath used for the active tool row — peach is the
/// theme's primary accent, so an in-flight tool reads as the focal point.
pub(super) fn tool_breath(tick: u64) -> Color {
    breath_color(tick, (115, 80, 60), (250, 179, 135))
}

/// Boil a tool call down to a `verb · primary-arg` summary the user can scan
/// at a glance. Tool-specific so the most-informative argument bubbles up:
/// `read_file({"path":"src/main.rs"})` → `read · src/main.rs`,
/// `run_command({"command":"cargo build"})` → `$ cargo build`. Unknown
/// tools fall back to `name(json)` so nothing is lost. Accepts the model's
/// TitleCase aliases (`Read`, `Bash`, …) the same way `tools::resolve_tool_alias` does.
pub(super) fn tool_summary(name: &str, args: Option<&serde_json::Value>) -> String {
    let get_str = |key: &str| -> Option<String> {
        args.and_then(|v| v.get(key))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    match name {
        "read_file" | "Read" => format!("read · {}", get_str("path").unwrap_or_else(|| "?".into())),
        "list_dir" | "LS" | "List" => {
            format!("ls · {}", get_str("path").unwrap_or_else(|| ".".into()))
        }
        "find_files" | "Glob" => {
            format!(
                "find · {}",
                get_str("pattern").unwrap_or_else(|| "?".into())
            )
        }
        "git_status" => "git status".into(),
        "git_log" => {
            let n = args
                .and_then(|v| v.get("limit"))
                .and_then(|v| v.as_i64())
                .map(|n| format!(" -n {n}"))
                .unwrap_or_default();
            format!("git log{n}")
        }
        "git_diff" => match get_str("path") {
            Some(p) if !p.is_empty() => format!("git diff · {p}"),
            _ => "git diff".into(),
        },
        "git_show" => format!(
            "git show · {}",
            get_str("rev").unwrap_or_else(|| "?".into())
        ),
        "edit_file" | "Edit" => format!("edit · {}", get_str("path").unwrap_or_else(|| "?".into())),
        "multi_edit" | "MultiEdit" => {
            let path = get_str("path").unwrap_or_else(|| "?".into());
            let count = args
                .and_then(|v| v.get("edits"))
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            if count > 0 {
                format!("multi-edit · {path} ({count} edits)")
            } else {
                format!("multi-edit · {path}")
            }
        }
        "write_file" | "Write" => {
            format!("write · {}", get_str("path").unwrap_or_else(|| "?".into()))
        }
        "run_command" | "Bash" | "Shell" => {
            format!("$ {}", get_str("command").unwrap_or_else(|| "?".into()))
        }
        // Phase 2 specialist delegation. The args carry a multi-line
        // `query` that's noisy when inlined into the header — collapse
        // to just the specialist name. The full query is rendered into
        // the expanded body by the chat renderer so nothing is lost;
        // it's just folded by default.
        "consult_specialist" => {
            format!(
                "consult · {}",
                get_str("name").unwrap_or_else(|| "?".into())
            )
        }
        // Memory ops — collapse to `memory · verb {slug}` instead of the
        // default JSON dump. `save_memory`'s args carry the full body
        // text, which would otherwise inline several KB of prose into
        // the tile header. The slug alone is the signal worth showing.
        "save_memory" => format!(
            "memory · save {}",
            get_str("name").unwrap_or_else(|| "?".into())
        ),
        "read_memory" => format!(
            "memory · read {}",
            get_str("name").unwrap_or_else(|| "?".into())
        ),
        "forget_memory" => format!(
            "memory · forget {}",
            get_str("name").unwrap_or_else(|| "?".into())
        ),
        other => {
            let json = args
                .and_then(|v| serde_json::to_string(v).ok())
                .unwrap_or_else(|| "{}".into());
            format!("{other}({json})")
        }
    }
}

/// Membership info for the "reading N files" consolidation card. Set on
/// every message that's part of a run of consecutive collapsed read-only
/// tool calls (see `compute_read_groups`); `None` for everything else.
#[derive(Clone, Copy)]
pub(super) struct ReadGroup {
    /// Index of the first visible message in the run — anchor for the
    /// `reading N files` header.
    pub first: usize,
    /// Index of the last visible message in the run — used to know when
    /// to emit the trailing spacer.
    pub last: usize,
    /// Total number of visible messages in the run. Drives the header
    /// count and decides whether to consolidate at all (requires ≥ 2).
    pub count: usize,
}

/// Compute consolidation groups: runs of ≥ 2 consecutive **collapsed**
/// read-only tool messages, skipping any hidden messages between them.
/// Returns a vec parallel to `messages` — `Some(g)` means msg is part of
/// the consolidation card, `None` means it renders standalone.
pub(super) fn compute_read_groups(
    messages: &[crate::ollama::ChatMessage],
    expanded: &std::collections::HashSet<usize>,
) -> Vec<Option<ReadGroup>> {
    let mut out = vec![None; messages.len()];
    let visible: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.hidden)
        .map(|(i, _)| i)
        .collect();

    let groupable = |i: usize| -> bool {
        let m = &messages[i];
        m.role == "tool"
            && crate::tools::is_readonly_tool(m.name.as_deref().unwrap_or(""))
            && !expanded.contains(&i)
    };

    let mut run: Vec<usize> = Vec::new();
    let flush = |run: &mut Vec<usize>, out: &mut Vec<Option<ReadGroup>>| {
        if run.len() >= 2 {
            let info = ReadGroup {
                first: run[0],
                last: *run.last().unwrap(),
                count: run.len(),
            };
            for &k in run.iter() {
                out[k] = Some(info);
            }
        }
        run.clear();
    };
    for &idx in &visible {
        if groupable(idx) {
            run.push(idx);
        } else {
            flush(&mut run, &mut out);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Tally `(added, removed)` lines in an attached tool diff so the tool
/// header can read e.g. `(+12L -7L)` instead of the less-useful total
/// content line count. Mirrors `tools::diff::diff_stats` but the UI
/// crate doesn't depend on that one — the diff slice arrives via the
/// `ChatMessage::diff` field.
pub(super) fn diff_line_counts(diff: &[crate::tools::DiffLine]) -> (usize, usize) {
    let (mut added, mut removed) = (0usize, 0usize);
    for d in diff {
        match d.kind {
            crate::tools::DiffLineKind::Added => added += 1,
            crate::tools::DiffLineKind::Removed => removed += 1,
            crate::tools::DiffLineKind::Context | crate::tools::DiffLineKind::Summary => {}
        }
    }
    (added, removed)
}

/// Build one full-width line for a "reading N files" card. The bg color
/// fills from column 0 all the way to `width`, so consecutive card lines
/// stack into a single visual block without a border.
pub(super) fn card_line(
    prefix: &str,
    text: &str,
    fg: Color,
    bg: Color,
    width: usize,
) -> Line<'static> {
    let used = prefix.chars().count() + text.chars().count();
    let pad = width.saturating_sub(used);
    let pad_str = if pad > 0 {
        " ".repeat(pad)
    } else {
        String::new()
    };
    Line::from(vec![
        Span::styled(prefix.to_string(), Style::default().bg(bg)),
        Span::styled(text.to_string(), Style::default().fg(fg).bg(bg)),
        Span::styled(pad_str, Style::default().bg(bg)),
    ])
}

/// Find the arguments the model passed to the tool call that produced
/// `messages[i]`. The chat-completion convention pairs each `tool` message
/// positionally with one entry of the preceding assistant's `tool_calls`,
/// so we walk back to the nearest assistant message and index by how many
/// tool messages sit between it and `i`.
pub(super) fn args_for_tool_msg(
    messages: &[crate::ollama::ChatMessage],
    i: usize,
) -> Option<&serde_json::Value> {
    if messages.get(i)?.role != "tool" {
        return None;
    }
    let mut prior_tools: usize = 0;
    let mut asst_idx: Option<usize> = None;
    for j in (0..i).rev() {
        match messages[j].role.as_str() {
            "tool" => prior_tools += 1,
            "assistant" => {
                asst_idx = Some(j);
                break;
            }
            // user / info — no preceding assistant tool_calls relate to this tool
            _ => return None,
        }
    }
    let tcs = messages[asst_idx?].tool_calls.as_ref()?;
    tcs.get(prior_tools).map(|tc| &tc.function.arguments)
}

/// Split an assistant message into its `<think>` reasoning block and the
/// visible answer. Qwen3's chat template *prepends* `<think>\n` to the
/// assistant prefix, so streamed output starts directly with reasoning text
/// and emits `</think>` once the model is ready to answer.
///
/// Returns `(thinking, visible)` where:
///   - `thinking` is `Some(text)` if the model produced any reasoning content,
///     `None` if the message has no thinking (or thinking is empty).
///   - `visible` is the post-`</think>` answer.
///
/// While still streaming and `</think>` hasn't arrived yet, everything so far
/// is reasoning — we report `visible = ""` so the existing "generating dots"
/// branch renders progress without leaking raw thoughts. Once generation
/// finishes without ever emitting `</think>`, we fall back to treating the
/// whole content as visible (legacy / non-reasoning models).
pub(super) fn split_thinking(s: &str, generating: bool) -> (Option<&str>, &str) {
    const CLOSE: &str = "</think>";
    const OPEN: &str = "<think>";
    if let Some(idx) = s.find(CLOSE) {
        let raw_think = &s[..idx];
        // Strip a leading "<think>" if present (some templates include it in
        // the streamed content rather than the prompt) plus surrounding
        // whitespace.
        let trimmed_think = raw_think
            .trim_start_matches(OPEN)
            .trim_matches(|c: char| c == '\n' || c == '\r' || c == ' ');
        let after = &s[idx + CLOSE.len()..];
        let visible = after.trim_start_matches(['\n', '\r']);
        if trimmed_think.is_empty() {
            (None, visible)
        } else {
            (Some(trimmed_think), visible)
        }
    } else if generating {
        // Mid-stream: thinking in progress, no answer yet. Hide content;
        // the generating-spinner branch will show a "…" placeholder.
        (None, "")
    } else {
        // Finished without a closing </think>: legacy / non-thinking model.
        // Render content as-is.
        (None, s)
    }
}
