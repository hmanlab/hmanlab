# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
