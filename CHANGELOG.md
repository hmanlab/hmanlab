# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-05-16

### Added
- First public release.
- Streaming TUI chat against local Ollama (`/api/chat`) or any OpenAI-compatible `/chat/completions` endpoint.
- BYOK providers: z.ai (subscription + usage URLs), Ollama Cloud (Bearer auth against `ollama.com`), OpenCode Go (`opencode.ai/zen/go/v1`).
- Agentic tool calls: `read_file`, `list_dir`, `find_files`, `git_status`, `git_log`, `git_diff`, `git_show`, `edit_file`, `write_file`, `run_command` (30 s timeout), and the memory tools. Every mutating call asks for confirmation in the TUI.
- Persistent memory store at `~/.hmanlab/memory/` (user scope) and `<workspace>/.hmanlab/memory/` (project scope), with an auto-maintained `MEMORY.md` index injected into the system prompt.
- `/compact` slash command + auto-compaction once the prompt token count crosses ~24 000. Compaction summary is persisted as a rolling `compact-current` project memory.
- `/disconnect` slash command with an arrow-key picker that lists every provider with a stored key and lets you remove one.
- Session persistence to the hmanlab-api backend (default `https://be-ai.senireka.my`, override with `--api-url`).
- Sidebar workspace tree with click-to-expand directories and click-to-open files.
- Inline markdown rendering (`**bold**`, `` `code` ``) and OSC 52 clipboard copy on drag-select.
- First-run wizard for Ollama URL + hmanlab-api key, saved to `~/.config/hmanlab/config.json` (mode 600).
- npm packaging via the per-arch optional-dependency pattern: umbrella `hmanlab` + `@hmanlab/{linux-x64,linux-arm64,darwin-x64,darwin-arm64,win32-x64}`.

[Unreleased]: https://github.com/rekabytes/hmanlab/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/rekabytes/hmanlab/releases/tag/v0.2.0
