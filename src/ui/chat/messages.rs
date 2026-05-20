//! `render_chat` — assembles every visible row in the chat panel.
//!
//! Walks `app.messages` once, emitting both styled `Line`s (for ratatui)
//! and a parallel plain-text snapshot (for copy-on-drag). Branches on
//! message role + state:
//!   - Read-card grouping (consecutive collapsed read-only tools).
//!   - Standalone tool tiles (write/edit/multi_edit/etc., or expanded reads).
//!   - Thinking-fold for assistant `<think>…</think>` blocks.
//!   - Plain assistant / user / info / summary text.
//!
//! After paragraph render, two buffer-overlay passes paint the hover
//! highlight on the row under the cursor and the drag-select rectangle.

use ratatui::{
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::App;

use super::super::theme;
use super::super::wrap_cache::{wrap_md_paragraph, wrap_plain_paragraph};
use super::helpers::{
    args_for_tool_msg, card_line, compute_read_groups, diff_line_counts, split_thinking,
    thinking_breath, tool_breath, tool_summary,
};

pub(in crate::ui) fn render_chat(f: &mut Frame, area: Rect, app: &mut App) {
    // Chat is the always-focused surface — wear the active border colour.
    let block = theme::panel_block("chat", true).padding(Padding::horizontal(1));
    let inner = block.inner(area);

    // Stash inner geometry so the mouse selection code can hit-test.
    app.render.chat_x = inner.x;
    app.render.chat_y = inner.y;
    app.render.chat_w = inner.width;
    app.render.chat_h = inner.height;

    // 2-col gutter under each speaker label: rendered as a colored `▎` bar
    // in the role's color, but recorded in `text_lines` as two spaces so
    // copy-on-drag doesn't grab the bar glyph and selection cell-widths
    // still line up.
    let indent = "  ";
    let gutter_glyph = "▎ ";
    let content_width = (inner.width as usize).saturating_sub(indent.len()).max(10);

    let mut lines: Vec<Line> = Vec::new();
    // Parallel plain-text copy of `lines` so copy-on-drag can extract the
    // selected substring without re-parsing styled spans.
    let mut text_lines: Vec<String> = Vec::new();
    let mut ranges: Vec<(usize, u16, u16)> = Vec::with_capacity(app.messages.len());
    let last_idx = app.messages.len().saturating_sub(1);
    // Read-card grouping: consecutive collapsed read-only tool messages
    // (read_file, list_dir, git_*, etc.) coalesce into a single tinted
    // tile rather than rendering as N standalone rows. Expanding any tool
    // breaks it out of the group and shows its full output (and diff if
    // applicable) the normal way.
    let read_groups = compute_read_groups(&app.messages, &app.expanded_tools);
    let card_bg = theme::color::BG_CARD;
    let card_width = inner.width as usize;
    // Reset per-frame hover hit-test list — populated below as each card
    // file row is emitted, consumed after the paragraph render to paint
    // the hover overlay on whichever row is under the cursor.
    app.render.card_row_targets.clear();
    for (i, msg) in app.messages.iter().enumerate() {
        if msg.hidden {
            continue;
        }
        // Card-grouped rendering: header row (once) + one dim file row
        // per member. Each file row's line range maps back to its
        // message index so clicking it still toggles that single tool's
        // expansion (and breaks it out of the group on the next frame).
        if let Some(group) = read_groups[i] {
            let line_start = lines.len() as u16;
            if i == group.first {
                let header_text = format!("reading {} files", group.count);
                text_lines.push(format!("  {header_text}"));
                lines.push(card_line(
                    "  ",
                    &header_text,
                    theme::color::FG,
                    card_bg,
                    card_width,
                ));
            }
            let summary = tool_summary(
                msg.name.as_deref().unwrap_or("tool"),
                args_for_tool_msg(&app.messages, i),
            );
            text_lines.push(format!("    {summary}"));
            lines.push(card_line(
                "    ",
                &summary,
                theme::color::FG_DIM,
                card_bg,
                card_width,
            ));
            // Range covers just this msg's row inside the card.
            let line_end_excl = lines.len() as u16;
            let logical_row = line_end_excl.saturating_sub(1);
            ranges.push((i, logical_row, line_end_excl));
            // Record this row as a hover target — the post-render overlay
            // below uses this to know which screen row to repaint with the
            // hover bg when the cursor lands on it.
            app.render.card_row_targets.push((logical_row, i));
            // Spacer goes AFTER the last group member, not between them
            // (that's what makes the card read as one block).
            if i == group.last && i != last_idx {
                text_lines.push(String::new());
                lines.push(Line::from(""));
            }
            // Done with this message — skip the standalone render below.
            let _ = line_start;
            continue;
        }
        let line_start = lines.len() as u16;
        let is_tool = msg.role == "tool";
        let tool_expanded = is_tool && app.expanded_tools.contains(&i);
        let is_active_tool = is_tool && app.active_tool_msg_idx == Some(i);

        // Tool rows are detected as errored by their content's first line —
        // the agent loop in agent.rs wraps tool failures as "error: {e}".
        let tool_errored =
            is_tool && !is_active_tool && msg.content.trim_start().starts_with("error:");

        // Standalone card-styled tool render. Triggered for any tool that
        // didn't get folded into a consolidated read group above. Same
        // BG_CARD fill, same dim-grey body — so expanding a tool feels
        // like the card just grew downward instead of jumping into a
        // different visual style. Keeps "agent tool" vs "agent reply"
        // immediately distinguishable: tools always look like tinted
        // tiles, assistant text always looks like plain prose with a
        // gutter bar.
        if is_tool {
            // Header label — same glyph + summary + suffix logic as the
            // legacy tool header below, but rendered as a full-width card
            // row instead of bare text.
            let summary = tool_summary(
                msg.name.as_deref().unwrap_or("tool"),
                args_for_tool_msg(&app.messages, i),
            );
            // Suffix prefers `+aL -rL` add/remove counts when the tool
            // carries an attached diff (write_file / edit_file /
            // multi_edit / save_memory). Falls back to total-content-line
            // count for tools without a diff (read_file, list_dir,
            // git_*, etc.) — there's no meaningful add/remove for them.
            let suffix = if is_active_tool {
                "  · running…".to_string()
            } else if tool_errored {
                "  · failed".to_string()
            } else if tool_expanded {
                String::new()
            } else if let Some(diff) = msg.diff.as_ref() {
                let (added, removed) = diff_line_counts(diff);
                format!("  (+{added}L -{removed}L)")
            } else {
                let body_lines = msg.content.lines().count().max(1);
                format!("  ({body_lines}L)")
            };
            // No chevron in either collapsed or expanded state — matches
            // the grouped read-card design and your "no chevron" rule.
            // The card-tile bg + hover highlight carries clickability;
            // visible body rows make the expanded state obvious. `◌` is
            // kept only for the actively-running tool because the breath
            // colour alone can be subtle and the spinner is informative
            // (not a click affordance).
            let label = if is_active_tool {
                format!("◌ {summary}{suffix}")
            } else {
                format!("{summary}{suffix}")
            };
            // Match the grouped read-card row colour — dim grey instead of
            // sky-blue. Reserves the saturated palette for genuine state
            // signals (red for failed, peach breath for actively running)
            // so a wall of routine reads doesn't outshine assistant prose.
            let header_fg = if is_active_tool {
                tool_breath(app.anim_tick)
            } else if tool_errored {
                theme::color::TOOL_ERROR
            } else {
                theme::color::FG_DIM
            };

            // Header row — bold tool color over card bg, indent 2 to align
            // with `▎ assistant` body text under the role label.
            let header_prefix = "  ";
            text_lines.push(format!("{header_prefix}{label}"));
            let header_used = header_prefix.chars().count() + label.chars().count();
            let header_pad = card_width.saturating_sub(header_used);
            lines.push(Line::from(vec![
                Span::styled(header_prefix.to_string(), Style::default().bg(card_bg)),
                Span::styled(
                    label,
                    Style::default()
                        .fg(header_fg)
                        .bg(card_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ".repeat(header_pad), Style::default().bg(card_bg)),
            ]));
            // Header is the click target — hover highlight follows it.
            let header_logical_row = (lines.len() as u16).saturating_sub(1);
            app.render.card_row_targets.push((header_logical_row, i));

            if tool_expanded {
                let body_prefix = "    ";
                let body_indent_w = body_prefix.chars().count();
                let body_content_w = card_width.saturating_sub(body_indent_w).max(10);

                // Helper: emit one body row as a full-width card line so
                // the bg stays solid edge-to-edge regardless of text length.
                let push_body_row = |lines: &mut Vec<Line>,
                                     text_lines: &mut Vec<String>,
                                     text: String,
                                     fg: Color| {
                    text_lines.push(format!("{body_prefix}{text}"));
                    let used = body_indent_w + text.chars().count();
                    let pad = card_width.saturating_sub(used);
                    lines.push(Line::from(vec![
                        Span::styled(body_prefix.to_string(), Style::default().bg(card_bg)),
                        Span::styled(text, Style::default().fg(fg).bg(card_bg)),
                        Span::styled(" ".repeat(pad), Style::default().bg(card_bg)),
                    ]));
                };

                if let Some(diff) = msg.diff.as_ref() {
                    // Diff body — keep semantic palette (green/red/dim/yellow)
                    // for readability, but every row sits on the card bg so
                    // the expanded block reads as a single tile.
                    for dl in diff {
                        let fg = match dl.kind {
                            crate::tools::DiffLineKind::Added => theme::color::SUCCESS,
                            crate::tools::DiffLineKind::Removed => theme::color::ERROR,
                            crate::tools::DiffLineKind::Context => theme::color::FG_DIM,
                            crate::tools::DiffLineKind::Summary => theme::color::WARNING,
                        };
                        for spans in
                            wrap_plain_paragraph(&dl.text, Style::default().fg(fg), body_content_w)
                        {
                            let combined: String =
                                spans.iter().map(|s| s.content.as_ref()).collect();
                            push_body_row(&mut lines, &mut text_lines, combined, fg);
                        }
                    }
                } else {
                    // Plain text body — dim grey on card bg. No markdown
                    // parsing (tool output is typically raw JSON/text/log).
                    let trimmed = msg.content.trim_end_matches(['\n', '\r']);
                    let body_fg = if tool_errored {
                        theme::color::TOOL_ERROR
                    } else {
                        theme::color::FG_DIM
                    };
                    for paragraph in trimmed.split('\n') {
                        if paragraph.is_empty() {
                            // Empty card row keeps the bg continuous — without
                            // this, blank lines in the tool output would punch
                            // a hole through the tile.
                            text_lines.push(String::new());
                            lines.push(Line::from(Span::styled(
                                " ".repeat(card_width),
                                Style::default().bg(card_bg),
                            )));
                            continue;
                        }
                        for spans in wrap_plain_paragraph(
                            paragraph,
                            Style::default().fg(body_fg),
                            body_content_w,
                        ) {
                            let combined: String =
                                spans.iter().map(|s| s.content.as_ref()).collect();
                            push_body_row(&mut lines, &mut text_lines, combined, body_fg);
                        }
                    }
                }
            }

            // Click hit-test covers the whole tool tile (header + any body).
            let line_end_excl = lines.len() as u16;
            ranges.push((i, line_start, line_end_excl));

            // Spacer between messages so consecutive tool tiles don't
            // visually fuse into one giant card.
            if i != last_idx {
                text_lines.push(String::new());
                lines.push(Line::from(""));
            }
            continue;
        }

        // Header line. For tool messages we collapse what used to be three
        // separate signals (`● ⏵ tool · name`, the `→ name(json)` echo on the
        // assistant message, and the trailing `(N lines)`) into one row that
        // reads as `verb · primary-arg` with state encoded in glyph + color.
        let (label, color) = match msg.role.as_str() {
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
                let color = if tool_errored {
                    theme::color::TOOL_ERROR
                } else {
                    theme::color::TOOL
                };
                (format!("{glyph} {summary}{suffix}"), color)
            }
            other => {
                let (text, c) = theme::role_label(other);
                (text.to_string(), c)
            }
        };
        let header_text = label.clone();
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
        // "this app is generating" to *every* assistant message made
        // historical turns from non-reasoning models (which never emit
        // `</think>`) get treated as mid-stream and re-render the
        // breathing placeholder. Scope "is generating" to the tail
        // message instead.
        let is_streaming_here = app.turn.is_generating() && i == last_idx;

        // Split assistant content into `<think>…</think>` reasoning + the
        // user-facing answer. Reasoning-tuned models (Qwen3 family, including
        // hmanlab-ai v0.1) emit chain-of-thought in <think> blocks; we render
        // it as a foldable header (collapsed by default) so the chat stays
        // readable while still letting the user expand to inspect reasoning.
        // Other roles render verbatim.
        // Owned body buffer used only by the consult_specialist expansion
        // path — we prepend the query to the message content so an
        // expanded consult shows `query: …` above the specialist's reply.
        // Stays None for every other path so `visible_content` borrows
        // from `msg.content` directly (no allocation in the common case).
        let consult_body: Option<String> =
            if is_tool && tool_expanded && msg.name.as_deref() == Some("consult_specialist") {
                let args = args_for_tool_msg(&app.messages, i);
                let query = args
                    .and_then(|v| v.get("query"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("(query not captured)");
                Some(format!("query: {query}\n\n{}", msg.content))
            } else {
                None
            };
        let (think_text, visible_content): (Option<&str>, &str) = if msg.role == "assistant" {
            split_thinking(&msg.content, is_streaming_here)
        } else if let Some(buf) = consult_body.as_deref() {
            (None, buf)
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
                format!(
                    "  ({body_lines} line{})",
                    if body_lines == 1 { "" } else { "s" }
                )
            };
            let header_text = format!("{indent}{chevron} thinking{suffix}");
            text_lines.push(header_text.clone());
            lines.push(Line::from(Span::styled(
                header_text,
                Style::default()
                    .fg(theme::color::FG_DIM)
                    .add_modifier(Modifier::ITALIC),
            )));
            if thought_expanded {
                for paragraph in think.split('\n') {
                    if paragraph.is_empty() {
                        text_lines.push(String::new());
                        lines.push(Line::from(""));
                        continue;
                    }
                    let body_style = Style::default()
                        .fg(theme::color::FG_DIM)
                        .add_modifier(Modifier::ITALIC);
                    let wrapped = wrap_md_paragraph(paragraph, body_style, content_width);
                    for spans in wrapped {
                        let mut plain = String::with_capacity(content_width);
                        plain.push_str(indent);
                        for span in &spans {
                            plain.push_str(span.content.as_ref());
                        }
                        text_lines.push(plain);
                        let mut line_spans: Vec<Span<'static>> =
                            Vec::with_capacity(spans.len() + 1);
                        line_spans.push(Span::styled(
                            gutter_glyph.to_string(),
                            Style::default().fg(theme::color::FG_DIMMER),
                        ));
                        line_spans.extend(spans);
                        lines.push(Line::from(line_spans));
                    }
                }
            }
        }

        let trimmed = visible_content.trim_end_matches(['\n', '\r']);

        // Render body, unless this is a collapsed tool.
        let show_body = !is_tool || tool_expanded;
        // Tools that went through y/n approval (write_file, edit_file,
        // save_memory) carry the authorised diff. When the tool row is
        // expanded, we render that diff colourised instead of the raw
        // text result — re-using the same green/red/dim scheme as the
        // confirm popup. The text fallback below still runs for tools
        // without a diff (read_file, run_command, etc.).
        let render_diff = is_tool && tool_expanded && msg.diff.is_some();
        if render_diff {
            if let Some(diff) = msg.diff.as_ref() {
                let gutter_style = Style::default().fg(theme::color::FG_DIMMER);
                for dl in diff {
                    let style = match dl.kind {
                        crate::tools::DiffLineKind::Added => {
                            Style::default().fg(theme::color::SUCCESS)
                        }
                        crate::tools::DiffLineKind::Removed => {
                            Style::default().fg(theme::color::ERROR)
                        }
                        crate::tools::DiffLineKind::Context => {
                            Style::default().fg(theme::color::FG_DIM)
                        }
                        crate::tools::DiffLineKind::Summary => Style::default()
                            .fg(theme::color::WARNING)
                            .add_modifier(Modifier::BOLD),
                    };
                    for spans in wrap_plain_paragraph(&dl.text, style, content_width) {
                        let mut plain = String::with_capacity(content_width);
                        plain.push_str(indent);
                        for span in &spans {
                            plain.push_str(span.content.as_ref());
                        }
                        text_lines.push(plain);
                        let mut line_spans: Vec<Span<'static>> =
                            Vec::with_capacity(spans.len() + 1);
                        line_spans.push(Span::styled(gutter_glyph.to_string(), gutter_style));
                        line_spans.extend(spans);
                        lines.push(Line::from(line_spans));
                    }
                }
            }
        } else if show_body {
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
                    let plain_text = format!("{indent}● thinking");
                    text_lines.push(plain_text);
                    let breath = thinking_breath(app.anim_tick);
                    lines.push(Line::from(vec![
                        Span::styled(gutter_glyph.to_string(), Style::default().fg(breath)),
                        Span::styled(
                            "● thinking",
                            Style::default().fg(breath).add_modifier(Modifier::BOLD),
                        ),
                    ]));
                }
            } else {
                let base_style = match msg.role.as_str() {
                    "info" => Style::default().fg(theme::color::SYSTEM),
                    "summary" => Style::default().fg(theme::color::SYSTEM),
                    "tool" if tool_errored => Style::default().fg(theme::color::TOOL_ERROR),
                    "tool" => Style::default().fg(theme::color::TOOL),
                    _ => Style::default().fg(theme::color::FG),
                };
                // Dim version of the role color for the body gutter — full
                // saturation reads as too loud when it runs down every line.
                let gutter_style = Style::default().fg(theme::color::FG_DIMMER);
                for paragraph in trimmed.split('\n') {
                    if paragraph.is_empty() {
                        text_lines.push(String::new());
                        lines.push(Line::from(""));
                        continue;
                    }
                    let wrapped = wrap_md_paragraph(paragraph, base_style, content_width);
                    for spans in wrapped {
                        let mut plain = String::with_capacity(content_width);
                        plain.push_str(indent);
                        for span in &spans {
                            plain.push_str(span.content.as_ref());
                        }
                        text_lines.push(plain);
                        let mut line_spans: Vec<Span<'static>> =
                            Vec::with_capacity(spans.len() + 1);
                        line_spans.push(Span::styled(gutter_glyph.to_string(), gutter_style));
                        line_spans.extend(spans);
                        lines.push(Line::from(line_spans));
                    }
                }
                // Blinking caret at the tail of the in-flight assistant
                // reply. Toggles every 4 ticks (~480 ms at the 120 ms
                // ticker) — reads as a real terminal cursor. Appended
                // only to `lines` so copy-on-drag won't pick up the glyph
                // from `text_lines`. Off-state is a space so the visual
                // width doesn't shift between blinks.
                if is_streaming_here && msg.role == "assistant" {
                    let caret = if (app.anim_tick / 4) % 2 == 0 {
                        "▌"
                    } else {
                        " "
                    };
                    if let Some(line) = lines.last_mut() {
                        line.spans
                            .push(Span::styled(caret, Style::default().fg(theme::color::FG)));
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
    app.render.rendered_text_lines = text_lines;
    app.render.message_line_ranges = ranges;

    // Scroll math has to use VISUAL row count, not `lines.len()`. With
    // `Wrap { trim: false }`, one logical Line can render as multiple
    // rows when it's wider than the viewport. The earlier version used
    // `lines.len() as u16` here, which under-counted long messages — a
    // 3-paragraph reply where each paragraph wraps to ~3 visual rows
    // would only let scroll reach the middle of the last paragraph,
    // cutting off the bottom even with follow=true. The copy buffer
    // (`rendered_text_lines`) intentionally stays one-per-logical-line
    // so click hit-testing keeps working; only the scroll bound changes.
    let mut para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    let total = para.line_count(inner.width).min(u16::MAX as usize) as u16;
    let visible = inner.height;
    let max_scroll = total.saturating_sub(visible);

    if app.follow {
        app.scroll = max_scroll;
    } else {
        app.scroll = app.scroll.min(max_scroll);
    }

    para = para.scroll((app.scroll, 0));
    f.render_widget(para, area);

    // Hover overlay for card rows. Repaint the cell bg for whichever
    // card file row sits under the cursor — gives a "this is clickable"
    // affordance without adding a chevron / arrow. Done by mutating the
    // buffer post-render so we don't have to know the hover row at
    // line-build time (which is awkward because the scroll offset is
    // computed *after* lines are assembled).
    if app.render.hover_x >= inner.x
        && app.render.hover_x < inner.x.saturating_add(inner.width)
        && app.render.hover_y >= inner.y
        && app.render.hover_y < inner.y.saturating_add(inner.height)
    {
        // Translate hover screen Y back to a logical line index using the
        // same scroll offset the paragraph rendered with.
        let hovered_logical = (app.render.hover_y as u32)
            .saturating_sub(inner.y as u32)
            .saturating_add(app.scroll as u32);
        // O(N) over visible card rows — N is usually 2–10, never enough
        // to matter. Bail on the first match because each logical row
        // belongs to one card entry.
        let hit = app
            .render
            .card_row_targets
            .iter()
            .any(|(row, _)| *row as u32 == hovered_logical);
        if hit {
            let y = app.render.hover_y;
            let x_start = inner.x;
            let x_end = inner.x.saturating_add(inner.width).saturating_sub(1);
            let bg = theme::color::BG_CARD_HOVER;
            let buf = f.buffer_mut();
            for x in x_start..=x_end {
                if let Some(cell) = buf.cell_mut(Position::new(x, y)) {
                    let s = cell.style().bg(bg);
                    cell.set_style(s);
                }
            }
        }
    }

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
