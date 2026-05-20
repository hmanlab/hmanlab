<h1 align="center">hmanlab</h1>

<p align="center">
  <b>The agentic terminal client for any LLM you have a key for.</b><br>
  Local Ollama · Cloud Ollama · z.ai · OpenCode Go · Telegram · One TUI
</p>

<p align="center">
  <a href="https://www.npmjs.com/package/hmanlab"><img alt="npm" src="https://img.shields.io/npm/v/hmanlab?label=npm&color=cb3837"></a>
  <a href="https://github.com/hmanlab/hmanlab/actions/workflows/ci.yml"><img alt="ci" src="https://github.com/hmanlab/hmanlab/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <a href="https://github.com/hmanlab/hmanlab/releases"><img alt="downloads" src="https://img.shields.io/github/downloads/hmanlab/hmanlab/total?label=downloads&color=green"></a>
  <img alt="status" src="https://img.shields.io/badge/status-alpha-orange">
</p>

<!-- Drop a demo recording here once you have one (asciinema cast or VHS-generated GIF).
     Suggested: 15–30 s clip showing launch → chat with markdown reply → tool call →
     file viewer. Save as docs/demo.gif then uncomment:
<p align="center">
  <img src="docs/demo.gif" alt="hmanlab demo" width="800">
</p>
-->

Built in [Rust](https://www.rust-lang.org) with [ratatui](https://ratatui.rs). Streams from any OpenAI-compatible endpoint or native Ollama. Sessions persist via [hmanlab-api](https://be-ai.senireka.my).

---

## Install

| Method | Command | Binary location |
|---|---|---|
| **Curl** | `curl -fsSL https://github.com/hmanlab/hmanlab/releases/latest/download/install.sh \| sh` | `~/.local/bin/hmanlab` |
| **npm (global)** | `npm i -g hmanlab` | `$(npm root -g)/../bin/hmanlab` |
| **npm (one-off)** | `npx hmanlab` | (no install) |
| **From source** | `cargo install --git https://github.com/hmanlab/hmanlab` | `~/.cargo/bin/hmanlab` |

Prebuilt binaries cover `linux-x64`, `linux-arm64`, `darwin-x64`, `darwin-arm64`, and `win32-x64`. Windows users: use the npm path — the curl installer is POSIX-only.

> **Pick one install method per machine and stick with it.** Each channel drops the binary in its own directory; if you install via curl and later run `npm i -g hmanlab`, both binaries exist and your `PATH` decides which one runs. Mixing channels is the most common reason "updates don't take effect." See [Updating](#updating) below.

---

## Features

### Chat

- **Streaming replies** — tokens render as they arrive from Ollama or any OpenAI-compatible endpoint.
- **Multi-provider** — local Ollama, Ollama Cloud, z.ai (subscription + usage-based), and OpenCode Go from one TUI; switch with `/model` or `Ctrl+M`.
- **Inline markdown** — `**bold**` and `` `code` `` render styled in the chat panel.
- **Thinking block folding** — `<think>…</think>` reasoning blocks collapse by default; click or `Ctrl+T` to expand.
- **Y/N quick-reply** — when the model asks a yes/no question, just press `Y` or `N`.
- **Inline autocomplete** — type `/` for slash-command autocomplete, `@` for file/folder mention autocomplete. ↑↓ to navigate, Tab/Enter to insert, Esc to dismiss.

### Tools & memory

- **Agentic tool calls** — the model reads files, explores directories, runs git commands, edits/writes files, executes shell commands, and recalls persistent memories. Every destructive action requires your confirmation in a scrollable popup with a diff preview.
- **Workspace trust** — on first launch in a new directory, hmanlab asks whether to trust the workspace. Untrusted workspaces allow read-only tools but block destructive ones. Use `/trust` or `/untrust` to change later. Dotfiles like `.env` are only visible when trusted.
- **Model persistence** — your last chosen model is remembered across restarts. Loading a saved session won't override it.
- **Persistent memory** — save and recall durable facts about you, your project, or how to behave, across sessions. Two scopes: user-wide and project-local.
- **Auto-compaction** — when the context window fills up, old turns are summarised into a single message so the conversation keeps going without losing the thread.

### Sessions & UX

- **Session persistence** — chats save to the hmanlab-api backend over HTTPS so future clients (mobile, web) can share your history. Falls back to local-only when the API is unreachable.
- **Session browsing** — `/sessions` to list, `/load <id>` to resume, `/more` to page through older messages.
- **Sidebar + file viewer** — browse your workspace tree and open files inline without leaving the TUI.
- **Mouse support** — drag to select text (copies via OSC 52), wheel to scroll, click on tool blocks to expand/collapse or re-view an approved diff.
- **Catppuccin Mocha theme** — coherent palette across header, sidebar, chat, popups, and viewer. Centralised in `src/ui/theme.rs` so every renderer pulls from one place.
- **First-run wizard** — guided setup for API key and provider selection on first launch; skip-everything-and-configure-later is fine.
- **Telegram connect** — pair your own Telegram bot to chat with hmanlab from your phone. Create a bot via @BotFather (paste the token with `/telegram setup`), then DM the bot to receive a 6-char pairing code — redeem it in the terminal with `/telegram pair <code>`. Only allowlisted contacts can interact; the code expires after 10 minutes. DMs from paired users are forwarded as user turns; the assistant's reply is sent back as a DM. Confirm destructive tool actions with inline ✅ Allow / 🔏 Always / ❌ Deny buttons (or a `y`/`n` text fallback). Slash commands (`/help`, `/models`, `/new`, `/sessions`, `/settings`) work from Telegram too. Idle notifications can DM paired users when a long local turn finishes.
- **Token tracking** — running prompt + completion token count shown in the header.

---

## Updating

The right update command depends on **how you installed**. Check first:

```bash
which hmanlab
```

| If `which` points to… | You installed via… | Update with |
|---|---|---|
| `~/.local/bin/hmanlab` | curl installer | re-run the curl command above |
| `<npm prefix>/bin/hmanlab` | npm | `npm install -g hmanlab@latest` |
| `~/.cargo/bin/hmanlab` | cargo | `cargo install hmanlab --force` |

From inside the running TUI you can also type `/update` — it detects cargo installs and prints the right `cargo install --force` hint, otherwise it shells out to `npm install -g hmanlab@latest`. (If you originally installed via curl, `/update` will install a *second* binary via npm; the curl one stays put. Re-running the curl script is the cleanest fix.)

`/update` checks the npm registry first and tells you whether a newer version actually exists before doing anything.

---

## First run

Launch `hmanlab` without a configured API key and an interactive wizard walks you through:

1. **hmanlab API key** — register a free account at [hmanlab.senireka.my](https://hmanlab.senireka.my) → **API keys**, paste the `bai_…` key when prompted. Validates against the backend and saves to `~/.config/hmanlab/config.json` (mode `0600`). The key only authenticates the TUI to the session-storage backend; it doesn't grant access to any LLM — you still bring your own model (local Ollama or a BYOK provider).
2. **Provider selection** — optionally add a z.ai subscription key, z.ai usage-based key, or local Ollama URL. Skip everything and configure later from inside the TUI.

After that, every flag is also settable via env var or CLI argument.

---

## Configuration

<details>
<summary><b>CLI flags & environment variables</b></summary>

| Flag | Default | Env |
|---|---|---|
| `--host` | `http://localhost:11434` | `OLLAMA_HOST` |
| `--model` | first available | `OLLAMA_MODEL` |
| `--api-url` | `https://be-ai.senireka.my` | `HMANLAB_API_URL` |
| `--api-key` | none (runs wizard) | `HMANLAB_API_KEY` |
| `--workspace` | current directory | — |

Examples:

```bash
# Basic — connect to a LAN Ollama with a specific model
hmanlab --host http://192.168.3.3:11434 --model qwen3:8b

# With persistence — prefer the env var so the key doesn't land in shell history
HMANLAB_API_KEY=bai_yourkeyhere hmanlab \
  --host http://192.168.3.3:11434 \
  --model qwen3:8b
```

</details>

<details>
<summary><b>Slash commands</b></summary>

| Command | Action |
|---|---|
| `/help`, `/?` | Show inline help |
| `/new`, `/n` | Start a fresh session (`Ctrl+N`) |
| `/sessions`, `/hist` | List recent saved sessions |
| `/load <id-prefix>` | Load a session (10 most recent messages) |
| `/more`, `/older` | Load 10 older messages in current loaded session |
| `/model` | Open model picker (`Ctrl+M`) |
| `/model <name>` | Switch model (partial match works) |
| `/models`, `/ls` | List available models |
| `/host <url>` | Change Ollama host |
| `/workspace <path>` | Change agent workspace |
| `/trust` | Authorise this workspace for file edits & shell |
| `/untrust` | Remove this workspace from the trusted list |
| `/compact` | Manually compact conversation history |
| `/disconnect` | Remove a BYOK provider and its models |
| `/settings`, `/whoami` | Show your account, version, and configured providers |
| `/telegram setup [token]` | Set up or replace the Telegram bot (opens wizard if no token given) |
| `/telegram pair [code]` | Redeem a pairing code from a Telegram DM |
| `/telegram status` | Show bot status, paired users, and last event |
| `/telegram unpair` | Clear all paired Telegram users (bot keeps running) |
| `/telegram off` | Stop the bot and clear token + allowlist |
| `/telegram notify [on\|off]` | Toggle idle notifications (DM when a local turn finishes) |
| `/agents [sub]` | Manage specialist agents — see [docs/multiagents.md](docs/multiagents.md) |
| `/ask <name> <query>` | Manually invoke a specialist (run `/agents on` first) |
| `/update` | Check the npm registry and update to the latest release |
| `/clear` | Clear visible chat (session keeps going) |
| `/quit`, `/exit` | Quit (also `Ctrl+Q` or `Ctrl+C` when idle) |

</details>

<details>
<summary><b>Key bindings</b></summary>

| Key | Action |
|---|---|
| `Enter` | Send message |
| `Alt+Enter` / `Ctrl+J` | Newline in input (most terminals collapse `Shift+Enter` to plain Enter) |
| `Ctrl+N` | New session |
| `Ctrl+M` | Open model picker |
| `Ctrl+T` | Fold/unfold all tool blocks and thinking blocks |
| `Ctrl+C` | Cancel generation (or quit when idle) |
| `Ctrl+Q` | Quit |
| `Ctrl+L` | Clear chat history |
| `Esc` | Interrupt generation, clear draft input, or close viewer/popup |
| `/` at start of input | Slash-command autocomplete |
| `@` after whitespace | File/folder mention autocomplete |
| `↑` / `↓` | Navigate autocomplete popup |
| `Tab` / `Enter` in popup | Insert selected completion |
| `Mouse wheel` | Scroll chat |
| `Drag` | Select text; release copies to clipboard (OSC 52) |
| `Click` on tool block | Toggle fold |
| `Click` on thinking block | Toggle fold |
| `PgUp/PgDn`, `Home/End` | Scroll |
| `Y` / `N` | Quick-reply when AI asks a yes/no question |

</details>

---

## Detailed docs

- **[docs/multiagents.md](docs/multiagents.md)** — multi-agent specialists: configure up to 5 named experts, each on their own model, and let the main agent delegate via `consult_specialist` or invoke manually with `/ask`.
- **[docs/telegram.md](docs/telegram.md)** — Telegram bot: pair your own bot to chat with hmanlab from your phone, with inline approval buttons for destructive tools.

---

## How it works

### Agent tools

When using a tool-capable model (`qwen2.5`, `qwen3`, `glm-4.7`, etc.), the AI can autonomously call tools to explore and edit your codebase. Read operations chain freely; anything destructive (`edit_file`, `write_file`, `run_command`, `save_memory`, `forget_memory`) pops a confirmation dialog with a diff preview.

<details>
<summary><b>Available tools</b></summary>

| Tool | What it does |
|---|---|
| `read_file` | Read file contents (~50 KB cap) |
| `list_dir` | List directory entries |
| `find_files` | Glob search (auto-filters build/cache dirs) |
| `git_status` | Working tree status |
| `git_log` | Recent commits |
| `git_diff` | Line-level diffs |
| `git_show` | Show a specific commit |
| `edit_file` | Surgical string replacement (user confirms) |
| `multi_edit` | Batch multiple edits to the same file (one confirm) |
| `write_file` | Create or overwrite a file (user confirms) |
| `run_command` | Shell command in workspace (user confirms, 30 s timeout) |
| `save_memory` | Save durable facts to persistent memory store (user confirms) |
| `read_memory` | Fetch saved memory by slug |
| `forget_memory` | Delete a saved memory (user confirms) |

</details>

### Persistent memory

Memory lets the AI remember facts across sessions. There are two scopes:

- **User scope** (`~/.hmanlab/memory/`) — facts about you that apply across every project: your role, preferences, habits.
- **Project scope** (`<workspace>/.hmanlab/memory/`) — facts specific to this workspace: decisions, architecture choices, external references.

When the agent calls `save_memory` it provides a `scope`, kebab-case `name`, `type` (`user` / `project` / `feedback` / `reference`), one-line `description`, and full `body` in markdown (up to 16 KB).

A `MEMORY.md` index is auto-maintained in each scope. The index is loaded into the system prompt at startup so the AI knows what memories exist and can fetch the full body on demand via `read_memory`.

### Providers

Add a BYOK provider with `Ctrl+M` (or `/model`) → pick one of the `+ Add` entries.

| Provider | Endpoint | How keys are auth'd |
|---|---|---|
| **Ollama** (local or LAN) | `--host` or `/host <url>` | None (local), Bearer (Ollama Cloud) |
| **z.ai subscription** | `https://api.z.ai/api/coding/paas/v4` | Bearer |
| **z.ai usage-based** | `https://api.z.ai/api/paas/v4` | Bearer |
| **Ollama Cloud** | `https://ollama.com` | Bearer (key from <https://ollama.com/settings/keys>) |
| **OpenCode Go** | `https://opencode.ai/zen/go/v1` | Bearer |
| **Telegram** | `api.telegram.org` (bot long-poll) | Bot token from @BotFather |

Keys live in `~/.config/hmanlab/config.json` (mode `0600`) and are sent **only** to the matching provider — never to the hmanlab-api session backend.

### Architecture

```
hmanlab (Rust TUI binary)
   │
   ├── Ollama API (--host, local or LAN)
   │   └── streaming chat + tool calls
   │
   ├── OpenAI-compat clients (z.ai, OpenCode Go, Ollama Cloud)
   │   └── streaming chat + tool calls via /chat/completions
   │
   ├── Telegram bot (api.telegram.org, long-poll)
   │   └── DMs → user turns, replies → DMs back, y/n confirm bridge
   │
   ├── Memory store (~/.hmanlab/memory/ + <workspace>/.hmanlab/memory/)
   │   └── markdown files + auto-maintained MEMORY.md index
   │
   └── HTTPS → hmanlab-api (--api-url)
                  └── Postgres (session + message persistence)
```

The TUI is a pure client. All persistence lives in hmanlab-api so future mobile and web clients can share your conversation history. When the API is unreachable, hmanlab still works fully — just without session saving.

<details>
<summary><b>Source layout</b></summary>

| File | Purpose |
|---|---|
| `src/main.rs` | CLI parsing, terminal setup, event loop |
| `src/agent.rs` | Agent loop — streams from LLM, dispatches tool calls, loops until final answer |
| `src/app/mod.rs` | `App` struct, state enums (`Mode`, `StreamMsg`, `PickerEntry`), constructor |
| `src/app/event.rs` | Keyboard/mouse handling per mode, slash-command parser, model picker, confirm dialog |
| `src/app/stream.rs` | `StreamMsg` handler — token chunks, tool results, session loading, confirm requests |
| `src/app/backend.rs` | `LlmBackend` enum, Ollama vs OpenAI-compat routing, provider key management |
| `src/ui/mod.rs` | Top-level render dispatch, header bar, status bar |
| `src/ui/chat.rs` | Message history rendering, input box, mouse selection + clipboard |
| `src/ui/popups.rs` | Model picker, session picker, add-key dialog, confirm popup with diff preview |
| `src/ui/sidebar.rs` | Workspace tree sidebar — expand/collapse + click handling |
| `src/ui/viewer.rs` | Inline file viewer overlay |
| `src/ui/markdown.rs` | Inline markdown parser (`**bold**`, `` `code` ``) + styled word-wrap |
| `src/ollama.rs` | Ollama `/api/chat` streaming client with tool-call support |
| `src/openai_compat.rs` | OpenAI-compatible `/chat/completions` SSE streaming client |
| `src/compact.rs` | Conversation compaction (manual + auto) |
| `src/update_check.rs` | Background npm-registry version check on startup |
| `src/tools/mod.rs` | Tool dispatch, `ConfirmRequest` / `ToolContext` types |
| `src/tools/definitions.rs` | Tool JSON schemas + system prompt (the model-facing surface) |
| `src/tools/read.rs` | `read_file`, `list_dir`, `find_files` |
| `src/tools/write.rs` | `edit_file`, `write_file` (user confirmation with diff preview) |
| `src/tools/git.rs` | `git_status`, `git_log`, `git_diff`, `git_show` |
| `src/tools/shell.rs` | `run_command` (user confirmation, 30 s timeout) |
| `src/tools/memory_tools.rs` | `save_memory`, `read_memory`, `forget_memory` |
| `src/tools/diff.rs` | Colored diff generation for the confirm popup |
| `src/tools/workspace.rs` | Workspace path safety + output truncation |
| `src/memory.rs` | Memory store I/O, MEMORY.md index maintenance |
| `src/api.rs` | hmanlab-api HTTP client + async writer task for session persistence |
| `src/telegram.rs` | Telegram bot — long-poll loop, pairing codes, allowlist, message chunking |
| `src/config.rs` | Config file I/O, setup wizard, BYOK model definitions |

</details>

---

## Security model

hmanlab is local-first. Your secrets and your conversation stay on your machine:

- **BYOK provider keys** (z.ai, Ollama Cloud, OpenCode Go) live in `~/.config/hmanlab/config.json` with file mode `0600`. They're sent **only** to the matching provider — never to the hmanlab-api session backend.
- **Agentic tools that touch the filesystem or shell** (`edit_file`, `write_file`, `run_command`, `save_memory`, `forget_memory`) open a confirmation dialog with a diff preview before running. `run_command` has a 30 s timeout.
- **The hmanlab-api backend** (default `https://be-ai.senireka.my`, override with `--api-url`) stores chat sessions for cross-device replay. It never sees your LLM provider keys; only the message text it's persisting. The default endpoint is the maintainer's hosted instance — you can run your own if you'd rather not rely on it.
- **Bugs and vulnerabilities** — please see [SECURITY.md](SECURITY.md) before opening a public issue.

---

## Contributing

Bug reports, features, and PRs welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) for the build/test loop and PR conventions. By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). For security issues, please follow [SECURITY.md](SECURITY.md) instead of opening a public issue.

## License

MIT — see [LICENSE](LICENSE).
