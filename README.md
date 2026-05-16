# hmanlab

A terminal UI for chatting with local and cloud LLMs — with agentic tool use, persistent memory, auto-compaction, and session persistence via [hmanlab-api](https://be-ai.senireka.my).

Built in Rust with [ratatui](https://ratatui.rs).

[![status](https://img.shields.io/badge/status-alpha-orange)](https://github.com/rekabytes/hmanlab)
[![license](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![ci](https://github.com/rekabytes/hmanlab/actions/workflows/ci.yml/badge.svg)](https://github.com/rekabytes/hmanlab/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/hmanlab?label=npm)](https://www.npmjs.com/package/hmanlab)

## Features

- **Streaming replies** — tokens appear as they arrive from Ollama or any OpenAI-compatible endpoint
- **Multiple providers** — local Ollama, Ollama Cloud, z.ai (subscription & usage-based), and OpenCode Go — all from one TUI
- **Agentic tool calls** — the model can read files, explore directories, run git commands, edit and write files, execute shell commands, and recall persistent memories (each destructive action confirmed by you)
- **Persistent memory** — save and recall durable facts about you, your project, or how to behave across sessions
- **Auto-compaction** — when the context window fills up, old turns are automatically compressed into a summary so the conversation can keep going
- **Session persistence** — chats are saved to the hmanlab-api backend over HTTPS so future clients (mobile, web) can share your history
- **Session browsing** — `/sessions` to list, `/load <id>` to resume, `/more` to page through older messages
- **Sidebar + file viewer** — browse your workspace tree and open files inline without leaving the TUI
- **Inline markdown** — `**bold**` and `` `code` `` rendered with style
- **Thinking block folding** — `<think?>…</think?>` reasoning blocks collapse by default; click or Ctrl+T to expand
- **Mouse support** — drag to select text (copies via OSC 52), wheel to scroll, click on tool blocks to expand/collapse
- **Y/N quick-reply** — when the model asks a yes/no question, just press Y or N
- **First-run wizard** — guided setup for API key and provider selection on first launch
- **Token tracking** — running prompt + completion token count shown in the header

## Security model

hmanlab is local-first. Your secrets and your conversation stay on your machine:

- **BYOK provider keys** (z.ai, Ollama Cloud, OpenCode Go) live in `~/.config/hmanlab/config.json` with file mode `0600`. They're sent **only** to the matching provider — never to the hmanlab-api session backend.
- **Agentic tools that touch the filesystem or shell** (`edit_file`, `write_file`, `run_command`, `save_memory`, `forget_memory`) open a confirmation dialog with a diff preview before running. `run_command` has a 30 s timeout.
- **The hmanlab-api backend** (default `https://be-ai.senireka.my`, override with `--api-url`) stores chat sessions for cross-device replay. It never sees your LLM provider keys; only the message text it's persisting. The default endpoint is the maintainer's hosted instance — you can run your own if you'd rather not rely on it.
- **Bugs and vulnerabilities** — please see [SECURITY.md](SECURITY.md) before opening a public issue.

## Install

### One-liner (Linux & macOS)

```bash
curl -fsSL https://github.com/rekabytes/hmanlab/releases/latest/download/install.sh | sh
```

Detects your OS+arch, downloads the matching binary from the latest GitHub Release, and installs it to `~/.local/bin/hmanlab`. No Node required. Pass `HMANLAB_INSTALL_DIR=/usr/local/bin sh` to override the install location.

### From npm (recommended)

Prebuilt binaries are published for `linux-x64`, `linux-arm64`, `darwin-x64`, `darwin-arm64`, and `win32-x64`.

```bash
# Global — `hmanlab` on PATH from any shell
npm install -g hmanlab

# Or run once without installing
npx hmanlab

# Or pin it to a project's dev deps
npm install --save-dev hmanlab
```

`npm` only downloads the binary for your platform thanks to the `@hmanlab/<plat>-<arch>` optional-dependency pattern — install footprint is ~5 MB.

### From source

Requires Rust 1.74+ and a reachable Ollama server.

```bash
cargo install --git https://github.com/rekabytes/hmanlab
# or
git clone https://github.com/rekabytes/hmanlab && cd hmanlab
cargo build --release
./target/release/hmanlab --help
```

## First run

When you launch hmanlab without a configured API key, an interactive wizard walks you through:

1. **hmanlab API key** — validates against the backend and saves to `~/.config/hmanlab/config.json` (mode 600)
2. **Provider selection** — optionally add a z.ai subscription key, z.ai usage-based key, or local Ollama URL. Skip everything and configure later from inside the TUI

Config is stored at `~/.config/hmanlab/config.json`. You can also set everything via CLI flags or environment variables.

## CLI flags

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
./target/release/hmanlab --host http://192.168.3.3:11434 --model qwen3:8b

# With persistence — prefer the env var so the key doesn't land in shell history
HMANLAB_API_KEY=bai_yourkeyhere ./target/release/hmanlab \
  --host http://192.168.3.3:11434 \
  --model qwen3:8b
```

## Slash commands

| Command | Action |
|---|---|
| `/help`, `/?` | Show inline help |
| `/new`, `/n` | Start a fresh session (Ctrl+N) |
| `/sessions`, `/hist` | List recent saved sessions |
| `/load <id-prefix>` | Load a session (10 most recent messages) |
| `/more`, `/older` | Load 10 older messages in current loaded session |
| `/model` | Open model picker (Ctrl+M) |
| `/model <name>` | Switch model (partial match works) |
| `/models`, `/ls` | List available models |
| `/host <url>` | Change Ollama host |
| `/workspace <path>` | Change agent workspace |
| `/compact` | Manually compact conversation history |
| `/disconnect` | Remove a BYOK provider and its models |
| `/clear` | Clear visible chat (session keeps going) |
| `/quit`, `/exit` | Quit (Esc) |

## Key bindings

| Key | Action |
|---|---|
| `Enter` | Send message |
| `Shift+Enter` | Newline in input |
| `Ctrl+N` | New session |
| `Ctrl+M` | Open model picker |
| `Ctrl+T` | Fold/unfold all tool blocks and thinking blocks |
| `Ctrl+C` | Cancel generation (or quit when idle) |
| `Ctrl+L` | Clear chat history |
| `Esc` | Quit (or close viewer/sidebar popup) |
| `Mouse wheel` | Scroll chat |
| `Drag` | Select text; release copies to clipboard (OSC 52) |
| `Click` on tool block | Toggle fold |
| `Click` on thinking block | Toggle fold |
| `PgUp/PgDn`, `Home/End` | Scroll |
| `Y` / `N` | Quick-reply when AI asks a yes/no question |

## Agent tools

When using a tool-capable model (e.g. qwen2.5, qwen3), the AI can autonomously call tools to explore and edit your codebase. Each destructive action (`edit_file`, `write_file`, `run_command`, `save_memory`) requires your approval via a confirm dialog.

Available tools:

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
| `write_file` | Create or overwrite a file (user confirms) |
| `run_command` | Shell command in workspace (user confirms, 30s timeout) |
| `save_memory` | Save durable facts to persistent memory store (user confirms) |
| `read_memory` | Fetch saved memory by slug |
| `forget_memory` | Delete a saved memory (user confirms) |

The agent can chain multiple tool calls in a single turn — reading files, exploring directories, then writing changes — all without stopping to ask permission between reads.

## Persistent memory

Memory lets the AI remember facts across sessions. There are two scopes:

- **User scope** — facts about you that apply across every project (e.g., your role, preferences, habits)
- **Project scope** — facts specific to this workspace (e.g., project decisions, architecture choices, external references)

When the agent calls `save_memory`, it provides:
- `scope` — `user` or `project`
- `name` — a kebab-case slug (e.g., `user-role-data-scientist`)
- `type` — `user` (profile facts), `project` (state of work), `feedback` (behaviour rules), or `reference` (pointers to external systems)
- `description` — a one-line summary that appears in the MEMORY.md index
- `body` — the full content in markdown (up to 16 KB)

Memories are stored under:
- `~/.hmanlab/memory/` for user-scope memories
- `<workspace>/.hmanlab/memory/` for project-scope memories

A `MEMORY.md` index is auto-maintained in each scope, listing all saved memories with their slugs and descriptions. This index is loaded into the system prompt at startup so the AI knows what memories are available and can request them via `read_memory`.

## BYOK models (z.ai)

Press `Ctrl+M` and select **"+ Add z.ai key"** to configure a z.ai coding plan API key. Once added, three cloud models become available in the picker:

- `glm-4.7` (default)
- `glm-4.6`
- `glm-5.1`

The key is stored locally in `~/.config/hmanlab/config.json` and sent only to z.ai — never to the hmanlab-api backend.

## Architecture

```
hmanlab (Rust TUI binary)
   │
   ├── Ollama API (--host, local or LAN)
   │   └── streaming chat + tool calls
   │
   ├── z.ai / OpenAI-compat (-- BYOK, added in-app)
   │   └── streaming chat + tool calls via /chat/completions
   │
   ├── Memory store (~/.hmanlab/memory/ + <workspace>/.hmanlab/memory/)
   │   └── markdown files + auto-maintained MEMORY.md index
   │
   └── HTTPS → hmanlab-api (--api-url)
                  └── Postgres (session + message persistence)
```

The TUI is a pure client. All persistence lives in hmanlab-api so future mobile and web clients can share your conversation history. When the API is unreachable, hmanlab still works fully — just without session saving.

### Source layout

| File | Purpose |
|---|---|
| `src/main.rs` | CLI parsing, terminal setup, event loop |
| `src/agent.rs` | Agent loop — streams from LLM, dispatches tool calls, loops until final answer |
| `src/app/mod.rs` | `App` struct, state enums (`Mode`, `StreamMsg`, `PickerEntry`), constructor |
| `src/app/event.rs` | Keyboard/mouse handling per mode, slash-command parser, model picker, confirm dialog |
| `src/app/stream.rs` | `StreamMsg` handler — token chunks, tool results, session loading, confirm requests |
| `src/app/backend.rs` | `LlmBackend` enum, Ollama vs OpenAI-compat routing, z.ai key management |
| `src/ui/mod.rs` | Top-level render dispatch, header bar, status bar |
| `src/ui/chat.rs` | Message history rendering, input box, mouse selection + clipboard |
| `src/ui/popups.rs` | Model picker, session picker, add-key dialog, confirm popup with diff preview |
| `src/ui/markdown.rs` | Inline markdown parser (`**bold**`, `` `code` ``) + styled word-wrap |
| `src/ollama.rs` | Ollama `/api/chat` streaming client with tool-call support |
| `src/openai_compat.rs` | OpenAI-compatible `/chat/completions` SSE streaming client (z.ai) |
| `src/tools/mod.rs` | Tool dispatch, `ConfirmRequest` / `ToolContext` types |
| `src/tools/definitions.rs` | Tool JSON schemas + system prompt (the model-facing surface) |
| `src/tools/read.rs` | `read_file`, `list_dir`, `find_files` |
| `src/tools/write.rs` | `edit_file`, `write_file` (user confirmation with diff preview) |
| `src/tools/git.rs` | `git_status`, `git_log`, `git_diff`, `git_show` |
| `src/tools/shell.rs` | `run_command` (user confirmation, 30 s timeout) |
| `src/tools/memory.rs` | `save_memory`, `read_memory`, `forget_memory` |
| `src/tools/diff.rs` | Colored diff generation for the confirm popup |
| `src/tools/workspace.rs` | Workspace path safety + output truncation |
| `src/memory.rs` | Memory store I/O, MEMORY.md index maintenance |
| `src/api.rs` | hmanlab-api HTTP client + async writer task for session persistence |
| `src/config.rs` | Config file I/O, setup wizard, BYOK model definitions |

## Contributing

Bug reports, features, and PRs welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) for the build/test loop and PR conventions. For security issues, please follow [SECURITY.md](SECURITY.md) instead of opening a public issue.

## License

MIT — see [LICENSE](LICENSE).