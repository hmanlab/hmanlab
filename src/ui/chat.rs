//! The main chat surface (message history + input box).

use ratatui::{
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::App;

use super::markdown::{parse_inline_md, wrap_styled_segments};

/// Period of one full breath, in animation ticks. The ticker fires every
/// 120 ms (see `main::run`), so 30 ticks ≈ 3.6 s — slow enough to read as
/// breathing rather than blinking.
const BREATH_PERIOD: u64 = 30;

/// Sine-interpolate between two RGB colors using `tick` as phase. Returns
/// `lo` at the trough and `hi` at the peak of each breath cycle.
fn breath_color(tick: u64, lo: (u8, u8, u8), hi: (u8, u8, u8)) -> Color {
    let phase =
        (tick % BREATH_PERIOD) as f32 / BREATH_PERIOD as f32 * std::f32::consts::TAU;
    let t = (phase.sin() * 0.5) + 0.5;
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    Color::Rgb(lerp(lo.0, hi.0), lerp(lo.1, hi.1), lerp(lo.2, hi.2))
}

/// Cyan-tinted breath used for the "thinking" indicator.
fn thinking_breath(tick: u64) -> Color {
    breath_color(tick, (40, 95, 115), (130, 230, 255))
}

/// Blue-tinted breath used for the active tool row.
fn tool_breath(tick: u64) -> Color {
    breath_color(tick, (45, 90, 150), (160, 200, 255))
}

/// Boil a tool call down to a `verb · primary-arg` summary the user can scan
/// at a glance. Tool-specific so the most-informative argument bubbles up:
/// `read_file({"path":"src/main.rs"})` → `read · src/main.rs`,
/// `run_command({"command":"cargo build"})` → `$ cargo build`. Unknown
/// tools fall back to `name(json)` so nothing is lost. Accepts the model's
/// TitleCase aliases (`Read`, `Bash`, …) the same way `tools::resolve_tool_alias` does.
fn tool_summary(name: &str, args: Option<&serde_json::Value>) -> String {
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
            format!("find · {}", get_str("pattern").unwrap_or_else(|| "?".into()))
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
        "git_show" => format!("git show · {}", get_str("rev").unwrap_or_else(|| "?".into())),
        "edit_file" | "Edit" => format!("edit · {}", get_str("path").unwrap_or_else(|| "?".into())),
        "write_file" | "Write" => {
            format!("write · {}", get_str("path").unwrap_or_else(|| "?".into()))
        }
        "run_command" | "Bash" | "Shell" => {
            format!("$ {}", get_str("command").unwrap_or_else(|| "?".into()))
        }
        other => {
            let json = args
                .and_then(|v| serde_json::to_string(v).ok())
                .unwrap_or_else(|| "{}".into());
            format!("{other}({json})")
        }
    }
}

/// Find the arguments the model passed to the tool call that produced
/// `messages[i]`. The chat-completion convention pairs each `tool` message
/// positionally with one entry of the preceding assistant's `tool_calls`,
/// so we walk back to the nearest assistant message and index by how many
/// tool messages sit between it and `i`.
fn args_for_tool_msg(
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

pub(super) fn render_chat(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" chat ")
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);

    // Stash inner geometry so the mouse selection code can hit-test.
    app.chat_x = inner.x;
    app.chat_y = inner.y;
    app.chat_w = inner.width;
    app.chat_h = inner.height;

    // 2-char indent under each speaker label; wrap to the remaining width.
    let indent = "  ";
    let content_width = (inner.width as usize).saturating_sub(indent.len()).max(10);

    let mut lines: Vec<Line> = Vec::new();
    // Parallel plain-text copy of `lines` so copy-on-drag can extract the
    // selected substring without re-parsing styled spans.
    let mut text_lines: Vec<String> = Vec::new();
    let mut ranges: Vec<(usize, u16, u16)> = Vec::with_capacity(app.messages.len());
    let last_idx = app.messages.len().saturating_sub(1);
    for (i, msg) in app.messages.iter().enumerate() {
        if msg.hidden {
            continue;
        }
        let line_start = lines.len() as u16;
        let is_tool = msg.role == "tool";
        let tool_expanded = is_tool && app.expanded_tools.contains(&i);
        let is_active_tool = is_tool && app.active_tool_msg_idx == Some(i);

        // Tool rows are detected as errored by their content's first line —
        // the agent loop in agent.rs wraps tool failures as "error: {e}".
        let tool_errored = is_tool
            && !is_active_tool
            && msg.content.trim_start().starts_with("error:");

        // Header line. For tool messages we collapse what used to be three
        // separate signals (`● ⏵ tool · name`, the `→ name(json)` echo on the
        // assistant message, and the trailing `(N lines)`) into one row that
        // reads as `verb · primary-arg` with state encoded in glyph + color.
        let (label, color) = match msg.role.as_str() {
            "user" => ("You".to_string(), Color::Green),
            "assistant" => ("AI".to_string(), Color::Cyan),
            "info" => ("system".to_string(), Color::Magenta),
            "summary" => ("compacted summary".to_string(), Color::Magenta),
            "tool" => {
                let summary = tool_summary(
                    msg.name.as_deref().unwrap_or("tool"),
                    args_for_tool_msg(&app.messages, i),
                );
                // Glyph carries fold state when settled; an open circle marks
                // the actively running tool (also picks up the breath color).
                let glyph = if is_active_tool {
                    "◌"
                } else if tool_expanded {
                    "⏷"
                } else {
                    "⏵"
                };
                let suffix = if is_active_tool {
                    "  · running…".to_string()
                } else if tool_errored {
                    "  · failed".to_string()
                } else if tool_expanded {
                    String::new()
                } else {
                    let body_lines = msg.content.lines().count().max(1);
                    format!("  ({body_lines}L)")
                };
                let color = if tool_errored { Color::Red } else { Color::Blue };
                (format!("{glyph} {summary}{suffix}"), color)
            }
            _ => ("?".to_string(), Color::Gray),
        };
        let header_text = format!("● {label}");
        text_lines.push(header_text.clone());
        let header_style = if is_active_tool {
            Style::default()
                .fg(tool_breath(app.anim_tick))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        };
        lines.push(Line::from(Span::styled(header_text, header_style)));

        // Only the most-recent message is actively streaming. Passing
        // `app.generating` to *every* assistant message made historical
        // turns from non-reasoning models (which never emit `</think>`) get
        // treated as mid-stream and re-render the breathing placeholder.
        // Scope "is generating" to the tail message instead.
        let is_streaming_here = app.generating && i == last_idx;

        // Split assistant content into `<think>…</think>` reasoning + the
        // user-facing answer. Reasoning-tuned models (Qwen3 family, including
        // hmanlab-ai v0.1) emit chain-of-thought in <think> blocks; we render
        // it as a foldable header (collapsed by default) so the chat stays
        // readable while still letting the user expand to inspect reasoning.
        // Other roles render verbatim.
        let (think_text, visible_content): (Option<&str>, &str) = if msg.role == "assistant" {
            split_thinking(&msg.content, is_streaming_here)
        } else {
            (None, msg.content.as_str())
        };

        // Render the thinking header (and body, if expanded) for this assistant turn.
        if let Some(think) = think_text {
            let thought_expanded = app.expanded_thoughts.contains(&i);
            let body_lines = think.lines().count().max(1);
            let chevron = if thought_expanded { "⏷" } else { "⏵" };
            let suffix = if thought_expanded {
                String::new()
            } else {
                format!("  ({body_lines} line{})", if body_lines == 1 { "" } else { "s" })
            };
            let header_text = format!("{indent}{chevron} thinking{suffix}");
            text_lines.push(header_text.clone());
            lines.push(Line::from(Span::styled(
                header_text,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )));
            if thought_expanded {
                for paragraph in think.split('\n') {
                    if paragraph.is_empty() {
                        text_lines.push(String::new());
                        lines.push(Line::from(""));
                        continue;
                    }
                    let body_style = Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM);
                    let segments = parse_inline_md(paragraph, body_style);
                    let wrapped = wrap_styled_segments(segments, content_width);
                    for spans in wrapped {
                        let mut plain = String::with_capacity(content_width);
                        plain.push_str(indent);
                        for span in &spans {
                            plain.push_str(span.content.as_ref());
                        }
                        text_lines.push(plain);
                        let mut line_spans: Vec<Span<'static>> = Vec::with_capacity(spans.len() + 1);
                        line_spans.push(Span::raw(indent.to_string()));
                        line_spans.extend(spans);
                        lines.push(Line::from(line_spans));
                    }
                }
            }
        }

        let trimmed = visible_content.trim_end_matches(|c: char| c == '\n' || c == '\r');

        // Render body, unless this is a collapsed tool.
        let show_body = !(is_tool && !tool_expanded);
        if show_body {
            if trimmed.trim().is_empty() {
                if msg.role == "assistant"
                    && is_streaming_here
                    && msg.tool_calls.as_ref().map_or(true, |t| t.is_empty())
                {
                    // Breathing "thinking" line — the assistant has nothing to
                    // show yet (either still inside <think>…</think> or a
                    // non-reasoning model not having streamed any tokens).
                    // Color pulses on `app.anim_tick`; the line text is plain
                    // so copy-on-drag still captures something sensible.
                    let breath_text = format!("{indent}● thinking");
                    text_lines.push(breath_text.clone());
                    lines.push(Line::from(Span::styled(
                        breath_text,
                        Style::default()
                            .fg(thinking_breath(app.anim_tick))
                            .add_modifier(Modifier::BOLD),
                    )));
                }
            } else {
                let base_style = match msg.role.as_str() {
                    "info" => Style::default().fg(Color::Magenta),
                    "summary" => Style::default().fg(Color::Magenta),
                    "tool" if tool_errored => Style::default().fg(Color::Red),
                    "tool" => Style::default().fg(Color::Blue),
                    _ => Style::default(),
                };
                for paragraph in trimmed.split('\n') {
                    if paragraph.is_empty() {
                        text_lines.push(String::new());
                        lines.push(Line::from(""));
                        continue;
                    }
                    let segments = parse_inline_md(paragraph, base_style);
                    let wrapped = wrap_styled_segments(segments, content_width);
                    for spans in wrapped {
                        let mut plain = String::with_capacity(content_width);
                        plain.push_str(indent);
                        for span in &spans {
                            plain.push_str(span.content.as_ref());
                        }
                        text_lines.push(plain);
                        let mut line_spans: Vec<Span<'static>> = Vec::with_capacity(spans.len() + 1);
                        line_spans.push(Span::raw(indent.to_string()));
                        line_spans.extend(spans);
                        lines.push(Line::from(line_spans));
                    }
                }
            }
        }

        // (Previously: an echo of `→ tool_name(json-args)` for each call on
        // the assistant message. That row duplicated the consolidated tool
        // header rendered when the matching `tool` message arrives, so it's
        // omitted — the model's text still renders above, and each tool call
        // gets one clean status row below.)

        let line_end_excl = lines.len() as u16;
        ranges.push((i, line_start, line_end_excl));

        // Spacer between messages, but not after the very last one
        if i != last_idx {
            text_lines.push(String::new());
            lines.push(Line::from(""));
        }
    }
    app.rendered_text_lines = text_lines;
    app.message_line_ranges = ranges;

    let total = lines.len() as u16;
    let visible = inner.height;
    let max_scroll = total.saturating_sub(visible);

    if app.follow {
        app.scroll = max_scroll;
    } else {
        app.scroll = app.scroll.min(max_scroll);
    }

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));
    f.render_widget(para, area);

    // Paint the selection overlay on top of the chat. Cells inside the
    // (sel_start, sel_end) rectangle, clamped to the chat inner area, get the
    // REVERSED modifier so they look highlighted.
    if let (Some(start), Some(end)) = (app.sel_start, app.sel_end) {
        let ((sx, sy), (ex, ey)) = if (start.1, start.0) <= (end.1, end.0) {
            (start, end)
        } else {
            (end, start)
        };
        let cx_min = inner.x;
        let cx_max = inner.x.saturating_add(inner.width).saturating_sub(1);
        let cy_min = inner.y;
        let cy_max = inner.y.saturating_add(inner.height).saturating_sub(1);
        let row_lo = sy.max(cy_min);
        let row_hi = ey.min(cy_max);
        if row_lo <= row_hi {
            let buf = f.buffer_mut();
            for y in row_lo..=row_hi {
                let row_start = if y == sy { sx.max(cx_min) } else { cx_min };
                let row_end = if y == ey { ex.min(cx_max) } else { cx_max };
                if row_start > row_end {
                    continue;
                }
                for x in row_start..=row_end {
                    if let Some(cell) = buf.cell_mut(Position::new(x, y)) {
                        let s = cell.style().add_modifier(Modifier::REVERSED);
                        cell.set_style(s);
                    }
                }
            }
        }
    }
}

pub(super) fn render_input(f: &mut Frame, area: Rect, app: &mut App) {
    let first_line = app.input.lines().first().cloned().unwrap_or_default();
    let is_cmd = first_line.trim_start().starts_with('/');

    let (title, border_color) = if app.generating {
        (
            " input — generating, Ctrl+C to cancel ".to_string(),
            Color::Yellow,
        )
    } else if is_cmd {
        (" command — Enter to run ".to_string(), Color::Magenta)
    } else if app.yn_pending {
        (
            " input — [Y] yes  ·  [N] no  ·  type to override ".to_string(),
            Color::Cyan,
        )
    } else {
        (" input ".to_string(), Color::White)
    };

    app.input.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(title),
    );
    f.render_widget(&app.input, area);
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
fn split_thinking<'a>(s: &'a str, generating: bool) -> (Option<&'a str>, &'a str) {
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
        let visible = after.trim_start_matches(|c: char| c == '\n' || c == '\r');
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
