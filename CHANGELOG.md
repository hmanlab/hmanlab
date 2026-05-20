# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2026-05-20

### What's new
- **Live shell monitor.** When the agent runs a `run_command`, the status bar shows a pulsing `● 1 shell running` indicator. Click it to open a monitor overlay that streams stdout and stderr in real time, with a blinking cursor at the tail. `Esc` hides the monitor and the shell keeps running; `Ctrl+C` inside the monitor kills the child cleanly and the badge flips to `✗ killed`. Hard timeout lifted from 30 s to 10 min so real workflows (test suites, builds) finish without the agent giving up — and you can always interrupt earlier.
- **Typing-cursor on streaming replies.** A `▌` caret blinks at the end of the assistant's in-flight message while it streams. Disappears the instant the turn ends and stays out of copy-on-drag selections.
- **`apply_patch` tool with V4A envelope.** New atomic primitive for multi-region content changes — the model emits a single `*** Begin Patch` / `*** End Patch` envelope with `Add File` / `Update File` / `Delete File` blocks and `@@` hunks, hmanlab parses it, validates every hunk against the matcher cascade, and applies the whole patch in one confirm popup. Models trained on Codex / GPT-5 traces already know this format. Replaces the old "panic-rewrite via write_file" failure path on multi-region edits.
- **Coordinate-based line ops: `move_lines`, `delete_lines`, `insert_at`.** The model passes line numbers (1-indexed, end-inclusive) instead of file content — so it can't get tripped up by whitespace drift, dropped list markers, or output-token truncation when doing structural changes. `move_lines(path, from_start, from_end, to_before)` is now the headline primitive for section reorders and block moves; `insert_at` lets the model splice in new content without ever reproducing existing lines. The tool descriptions explicitly steer the model toward these for rearrangements and away from `write_file`.

### Changed
- **`read_file` output now carries line numbers.** Every line is returned as `<n>\t<content>` — the model gets a coordinate system it can use directly when forming `edit_file` snippets or calling `move_lines`. Stripped before quoting back into `old_string` (the tool description spells this out).
- **`edit_file` and `multi_edit` accept `replace_all: true`.** Default behavior unchanged (strict single-match), but the model can opt in for genuine bulk renames. The ambiguity error now points the model at both recovery paths (expand context OR set `replace_all`) so it doesn't fall back to wholesale `write_file` rewrites.
- **Fuzzy-match cascade in `edit_file` / `multi_edit`.** When the exact `old_string` doesn't land, the matcher tries line-trimmed (catches trailing-whitespace drift) and block-anchor (catches middle-line drift while boundaries are right) strategies. Each still demands exactly one match — fuzziness lives in *how* we find the span, not in *what* we substitute.
- **All modal popups now split-pane instead of floating.** Model picker, sessions, confirm dialog, add-model, telegram setup, agents wizard, shell monitor — they all render in the bottom half of the chat column. The conversation stays visible in the top half so you can still see context while you pick / approve.
- **Memory tool rows are now one line.** `save_memory` / `read_memory` / `forget_memory` render as `memory · save <slug>` in the chat tile instead of dumping the full JSON arg blob (which could inline several KB of memory body into the header). The redundant `✓ Allowed: …` system line is also suppressed for memory operations — the tile itself already tells you what happened.

### Fixed
- **README-style section moves no longer panic-rewrite the whole file.** The old failure was structural: `edit_file` / `multi_edit` / `apply_patch` all required the model to reproduce file content byte-perfectly as a tool-call argument, and long reproductions hit token-budget truncation or whitespace-drift errors. `move_lines` retires that entire failure class by taking integer coordinates — the model never reproduces content, so reproduction errors are impossible. Section-swap requests that previously cycled through three failed `apply_patch` calls before giving up now land in one `move_lines` call.
- **Cancelling an agent turn while a shell was running no longer leaves the footer indicator stuck.** The `cancel()` path now finalizes the active `ShellRuntime` so the `● 1 shell running` indicator disappears when the underlying child process is killed.

[0.2.1]: https://github.com/hmanlab/hmanlab/compare/0.2.0...0.2.1

## [0.2.0] - 2026-05-19

### What's new
- **Multi-agent specialists.** Configure up to 5 named specialists, each with its own model and a one-line "use this when …" description. The main agent can `consult_specialist` automatically when its delegation rules match, or you can route by hand with `/ask <name> <query>`. Specialists run with a read-only tool surface (file reads, git, memory recall) — writes / shell / memory mutation stay with the main agent so cost stays bounded and loops can't form. Set them up via `/agents add` (5-step wizard with 7 opinionated templates: code-reviewer, planner, file-explorer, researcher, triage, test-advisor, doc-reviewer). Per-session opt-in with `/agents on` so you don't surprise-bill yourself across restarts. Header tally splits tokens per agent so the cost of consults stays legible.
- **Telegram bot integration.** Pair your own Telegram bot to chat with hmanlab from your phone. Run `/telegram setup` and paste a `@BotFather` token, then DM the bot to receive a 6-character pairing code and redeem it in the terminal with `/telegram pair <code>`. Only allowlisted contacts can interact; codes expire after 10 minutes. DMs from paired users land in the local TUI as user turns and the assistant's reply DMs back. Destructive tool actions get inline ✅ Allow / 🔏 Always / ❌ Deny buttons (with a `y`/`n` text fallback). Slash commands (`/help`, `/models`, `/model`, `/new`, `/sessions`, `/settings`, `/agents`, `/ask`) work from Telegram too with the same aliases as the local terminal. Optional `/telegram notify on` DMs paired users when a long local turn finishes after the terminal goes idle.
- **"Did you mean?" suggestions for typo'd subcommands.** Type `/agents lst` and hmanlab now suggests `/agents list` instead of silently falling through to the help screen.

### Changed
- **Streaming feels smoother.** Token-by-token replies coalesce multiple chunks into one redraw, and inline markdown is cached so long transcripts don't re-parse every paragraph on every frame. Long replies stay responsive even when the model is firing tokens fast.
- **Sidebar stays responsive on big repos.** The workspace tree is walked only when something actually changes (you toggle a directory or switch workspaces), not on every frame. Clicking around a large monorepo no longer stalls the chat stream.
- **Opening large files no longer freezes the UI.** Clicking a file in the sidebar shows `loading…` immediately while the read happens on a background thread. The viewer fills in when the read completes.
- **Config writes don't block the UI.** Adding a BYOK key, editing a specialist, or trusting a workspace now persists to disk on a background worker — no more pauses when finishing a wizard.
- **`Ctrl+C` during a multi-agent consult stops the specialist too.** Previously a runaway specialist could keep iterating (and billing your BYOK provider) after you cancelled the main agent. Now one `Ctrl+C` stops everything in flight.
- **Slash command aliases work everywhere.** `/m` for `/model`, `/n` for `/new`, `/ls` for `/models`, `/tg` for `/telegram` and the rest now work the same way from Telegram DMs as they do in the local terminal.

### Fixed
- **Long replies no longer cut off mid-paragraph.** Scroll math now uses visual rows (after word-wrap) instead of logical lines, so the bottom of a long streaming reply is always reachable with `End` or follow-mode.
- **`/help` always matches what works.** The commands cheatsheet now generates from the same table the parser uses, so help text can't drift from what's actually accepted. Aliases and argument shapes appear automatically next to each command.
- **Tool-result correlation can't silently break.** Tool result messages are built via a dedicated constructor that requires the `tool_call_id` matching the originating assistant turn — strict providers like MiniMax 400 without it, and there's no longer a struct literal to forget the field in.

[0.2.0]: https://github.com/hmanlab/hmanlab/compare/0.1.11...0.2.0

## [0.1.11] - 2026-05-19

### Added
- **OpenRouter model list stays current automatically.** After you paste your OpenRouter key, hmanlab fetches the live model catalog from openrouter.ai and refreshes the picker with everything available today — Claude Opus 4.7, Sonnet 4.6, Haiku 4.5; GPT-5.5 / 5.5 Pro / 5.4 Mini; Gemini 3.1 Flash Lite, Gemini 2.5 Pro; DeepSeek v4, Qwen 3.6, Grok 4.3, Kimi K2.6, and more. The refresh also runs on startup if your key is already configured, so new models show up the next time you launch.
- **Curated to popular vendors.** OpenRouter exposes 400+ models; the picker filters down to a usable set from Anthropic, OpenAI, Google, DeepSeek, Qwen, Meta, Mistral, xAI, Moonshot, and z.ai — and drops preview / image / TTS / embedding rows so you only see chat-capable models.

### Changed
- **Default OpenRouter model is now Claude Sonnet 4.6** (was GPT-4o Mini), reflecting the current best balance of capability and price for coding work.

[0.1.11]: https://github.com/hmanlab/hmanlab/compare/0.1.10...0.1.11

## [0.1.10] - 2026-05-19

### Added
- **OpenRouter as a new provider.** Add one OpenRouter API key and a curated set of popular models becomes available in `/model` — OpenAI's GPT-4o, Anthropic's Claude 3.5 Sonnet/Haiku, Google's Gemini 2.0 Flash, Meta's Llama 3.3 70B, Qwen 2.5 72B, DeepSeek Chat, and gpt-4o-mini as the default. One key, one bill, many vendors. Get a key at [openrouter.ai/settings/keys](https://openrouter.ai/settings/keys). Disconnect with `/disconnect openrouter` (or `or`).
- **Auto-load older messages when you scroll up.** In a loaded session, scrolling to the top of the chat automatically pages in older history — no more typing `/more`. Works with the mouse wheel, Page Up, the Up arrow on a single-line input, and Home. After fetching, the viewport stays anchored on the same content so you don't lose your place.

### Changed
- **Saved sessions now load 30 most recent messages** (was 10), so most chats fit on one screen without paging.
- **First-run wizard and READMEs now spell out that hmanlab is free and you bring your own LLM** (local Ollama or a BYOK provider like z.ai, Ollama Cloud, OpenCode Go, OpenRouter). The `bai_…` key registered at [hmanlab.senireka.my](https://hmanlab.senireka.my) only powers session storage — it doesn't grant access to any model.

### Fixed
- **`/update` recommends the right upgrade command for curl installs.** If you installed via the `install.sh` one-liner, `/update` now points you at that one-liner instead of telling you to run `npm install`. Cargo installs already worked; npm installs still upgrade in place automatically.

[0.1.10]: https://github.com/hmanlab/hmanlab/compare/0.1.9...0.1.10

## [0.1.9] - 2026-05-18

### Added
- **Workspace trust gate.** When you launch hmanlab in a new directory for the first time, it asks whether you trust that folder before opening the TUI. If you decline, read-only tools (reading files, listing directories, searching, git commands) still work, but file edits, shell commands, and memory changes are blocked. Your choice is remembered so you won't be asked again. Use `/trust` or `/untrust` to change your mind later. Dotfiles like `.env` are only visible in the sidebar and autocomplete when the workspace is trusted.
- **Multi-file editing in one step.** The AI can now apply multiple edits to the same file in a single request — you see one approval popup with the full diff instead of being asked to confirm each edit one by one.
- **Your chosen model is remembered.** After you switch models with `/model`, that choice persists across restarts. Loading a saved session no longer overrides your current model — you'll see which model the session used and can switch back with `/model` if you want.
- **Scrollable confirmation popups.** When reviewing a long diff before approving, you can now scroll through it with arrow keys, Page Up/Down, and Home/End. The position indicator (e.g. `35/120 lines`) is shown in the footer.
- **Click a tool message to see its diff again.** After you approve a file edit, clicking the tool row in the chat re-shows the diff inline with the same color highlighting.
- **Grouped read-only results.** When the AI reads several files in a row, the results are grouped into a single compact tile showing `reading N files` with the file paths listed, instead of one row per file.
- **Hover highlighting on file rows.** Moving your cursor over a clickable file row highlights it so you can tell it's interactive.
- **Visual polish.** Message lines now show a colored sidebar bar matching the speaker (user, assistant, tool, system). Role labels have been cleaned up to `▎ user`, `▎ assistant`, etc. Copying text by dragging still works cleanly without picking up the decorative bar.

### Changed
- **`git_show` shows full commit details.** Viewing a commit now shows the full message and diff instead of just a one-line summary.
- **`/workspace` works repeatedly.** Switching workspaces with relative paths now chains properly — `/workspace ../sibling` resolves from your current workspace, not from where you originally launched hmanlab. `~` and `~/path` shortcuts are supported too.
- **`/settings` refreshes in place.** The settings card updates itself when your account info loads instead of stacking a second card.
- **Diff summaries show lines, not bytes.** Confirmation prompts now display `+5 -3 lines` instead of byte counts, which is more intuitive. Bytes still appear in the tool result details where they're useful.
- **Longer agent sessions.** The AI can now chain up to 50 tool calls in a single turn (previously 10), which supports realistic multi-file refactors. If the limit is hit, the error message clearly says the model may be stuck in a loop.

### Fixed
- **`/workspace` only worked once.** Relative paths now resolve correctly against the current workspace instead of the original launch directory.

[0.1.9]: https://github.com/hmanlab/hmanlab/compare/0.1.8...0.1.9

## [0.1.8] - 2026-05-18

### Fixed
- **npm package links updated.** After the project moved to a new GitHub organization, the npm package pages still pointed at the old repository URL. All links have been updated so npm, docs, and install scripts reference the correct location.

### Security
- Suppressed an advisory for an unmaintained dependency (`paste`) that has no known vulnerability and no fix available. It's pulled in by the TUI framework and will be removed when the framework updates.
- An advisory about an unsafe iterator in the `lru` dependency remains open — the fix requires an upstream library update that hasn't been released yet.

## [0.1.7] - 2026-05-18

### Fixed
- **npm publishing restored.** The publish configuration still referenced the old GitHub organization after the repository transfer, preventing new versions from being published to npm. The configuration has been updated. Same features as v0.1.5.

## [0.1.6] - 2026-05-18

### Fixed
- **npm publishing was skipped after the repository moved.** The automated release workflow was still checking for the old GitHub organization name, so it silently skipped publishing to npm. This has been fixed. No behavior changes from v0.1.5 — this release just gets the npm package caught up.

## [0.1.5] - 2026-05-17

### Added
- **`/update` command** — checks the npm registry for the latest version and installs it in the background so you can keep chatting. Detects if you installed via cargo and shows the right command instead.
- **`/settings` command** (aliases: `/whoami`, `/account`, `/me`) — shows your running version, active model, configured providers, workspace, and account info.

### Changed
- **`Esc` no longer quits.** It now interrupts an in-flight generation, clears the input, or dismisses a popup. To quit, use `Ctrl+C` (when idle), `Ctrl+Q`, `/quit`, or `/exit`.
- Updated install instructions in the README with exact binary locations per install method and a new Updating section with a `which hmanlab` lookup table.

## [0.1.4] - 2026-05-17

### Added
- License and readme pages for each platform-specific npm package so they display correctly on npmjs.com.
- Automated supply-chain security scoring via OpenSSF Scorecard.

### Changed
- **npm publishing now uses trusted publishing** — no stored tokens; npm authenticates each publish directly through GitHub, reducing the risk of credential leaks.
- All GitHub Actions pinned to specific commit hashes instead of mutable tags to prevent supply-chain attacks on the build pipeline.

## [0.1.3] - 2026-05-16

### Added
- **Slash-command autocomplete** — type `/` to see available commands, `@` to see files and folders. Navigate with arrow keys and insert with Tab or Enter.

### Changed
- **Visual redesign** with the Catppuccin Mocha color palette across the entire TUI.
- README restructured with clearer sections and collapsible details.

## [0.1.2] - 2026-05-16

### Added
- **One-line curl installer** — `curl -fsSL …/install.sh | sh` and per-platform binaries attached to GitHub Releases.

### Fixed
- Release publishing is now idempotent — a partial failure can be re-run without errors.

## [0.1.1] - 2026-05-16

### Fixed
- Releases now trigger when you publish a GitHub Release, not when you push a tag — so you can draft release notes before the build starts.

## [0.1.0] - 2026-05-16

### Added
- First public release.
- Streaming chat against local Ollama or any OpenAI-compatible endpoint.
- Multi-provider support: local Ollama, Ollama Cloud, z.ai, and OpenCode Go.
- Agentic tool calls — the AI reads files, explores directories, runs git commands, edits files, and executes shell commands. Every destructive action requires your confirmation.
- Persistent memory store with user-wide and project-local scopes.
- Auto-compaction when the context window fills up.
- Session persistence via the hmanlab-api backend.
- Sidebar workspace tree with click-to-expand directories and click-to-open files.
- Inline markdown rendering and clipboard copy on drag-select.
- First-run wizard for API key and provider setup.
- Background update check on startup.
- `/compact`, `/disconnect`, and other slash commands.
- npm packaging with per-platform binaries for Linux, macOS, and Windows.

[0.1.8]: https://github.com/hmanlab/hmanlab/compare/0.1.7...0.1.8
[0.1.7]: https://github.com/hmanlab/hmanlab/compare/0.1.6...0.1.7
[0.1.6]: https://github.com/hmanlab/hmanlab/compare/0.1.5...0.1.6
[0.1.5]: https://github.com/rekabytes/hmanlab/compare/0.1.4...0.1.5
[0.1.4]: https://github.com/rekabytes/hmanlab/compare/0.1.3...0.1.4
[0.1.3]: https://github.com/rekabytes/hmanlab/compare/0.1.2...0.1.3
[0.1.2]: https://github.com/rekabytes/hmanlab/compare/0.1.1...0.1.2
[0.1.1]: https://github.com/rekabytes/hmanlab/compare/v0.1.0...0.1.1
[0.1.0]: https://github.com/rekabytes/hmanlab/releases/tag/v0.1.0
