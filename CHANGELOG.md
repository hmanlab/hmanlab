# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.5] - 2026-05-17

### Added
- `/update` slash command — checks the npm registry for the latest published version, prints `current → latest`, and runs `npm install -g hmanlab@latest` in the background so you can keep chatting while it installs. Detects cargo installs (binary under `~/.cargo/bin` or `target/`) and surfaces `cargo install hmanlab --force` instead. Reports a clean fallback when `npm` isn't on `PATH`.
- `/settings` slash command (aliases: `/whoami`, `/account`, `/me`) — shows the running version (with an upgrade hint if npm has a newer one), active model, Ollama host, configured BYOK providers (presence only — never the key), workspace, and your authenticated account (name, email, training opt-in, admin badge) fetched from `/v1/auth/me`.
- `api::Client::fetch_me()` and a public `Me` struct in `src/api.rs` for the account look-up.
- Public `update_check::fetch_latest_npm()` and `update_check::newer()` so other modules can run a fresh registry check without going through the 24 h startup cache.

### Changed
- **`Esc` no longer quits.** In chat mode it now interrupts an in-flight generation (same effect as `Ctrl+C` mid-stream), or clears the draft input and dismisses any open `/`/`@` autocomplete popup, or no-ops. Quit is `Ctrl+C` (when idle), `Ctrl+Q`, `/quit`, or `/exit`.
- `README.md` install table now shows each method's exact binary location (`~/.local/bin/hmanlab`, `$(npm root -g)/../bin/hmanlab`, `~/.cargo/bin/hmanlab`) and warns against mixing channels — the most common cause of "update doesn't take effect."
- New `README.md` **Updating** section: a `which hmanlab` → command lookup table so users always know the right update path for their install, and explicit notes on `/update`'s curl-install limitation.
- `README.md` slash-commands and key-bindings tables refreshed to cover `/update`, `/settings`, the new `Esc` behavior, and `Ctrl+Q`.

## [0.1.4] - 2026-05-17

### Added
- Subpackage `README.md` + `LICENSE` for each `@hmanlab/<plat>` artifact, so `npmjs.com` and Socket can render docs and license info per-platform.
- `npm test` smoke check for `bin/hmanlab.js` (asserts the "no prebuilt binary" error path) and a `node-smoke` CI job on ubuntu/macos/windows.
- OpenSSF Scorecard workflow (`.github/workflows/scorecard.yml`) — weekly + on push to `main` + on branch-protection-rule changes; publishes results to the public dataset Socket reads.

### Changed
- **Supply chain — `npm publish` now uses OIDC trusted publishing.** The `release.yml` `publish` job no longer reads `NPM_TOKEN`; npm mints a short-lived token via GitHub's OIDC issuer, authorised by the Trusted Publisher entry on npmjs.com. Requires the `NPM_TOKEN` secret to be deleted from the repo once a release publishes green.
- Pinned every GitHub Action in `ci.yml` and `release.yml` to a commit SHA (with the human-readable version as a comment). Stops tag-based supply-chain attacks on the build/publish pipeline.
- Backfilled `0.1.1` / `0.1.2` / `0.1.3` entries above.

## [0.1.3] - 2026-05-16

### Changed
- UI redesign: Catppuccin Mocha palette applied across the TUI.
- README restructured with a centered hero, grouped sections, and collapsible details.

### Added
- Slash-command autocomplete and `@`-file autocomplete in the input box.

## [0.1.2] - 2026-05-16

### Added
- One-line curl installer (`curl -fsSL …/install.sh | sh`) and per-platform binaries attached to GitHub Releases.

### Fixed
- Release publish is now idempotent and retries on the npm packument race (409 "Failed to save packument") so a partial-failure re-run picks up where it left off.

## [0.1.1] - 2026-05-16

### Fixed
- Release workflow now fires on Release **publish**, not on bare tag push — lets you draft notes before kicking off the build + npm publish pipeline.

## [0.1.0] - 2026-05-16

### Added
- First public release.
- Background update check on startup. Hits `registry.npmjs.org/hmanlab` once per launch (cached 24 h, skipped on debug builds, 3 s timeout, fails silently), and surfaces a green `vX.Y.Z available — npm i -g hmanlab` notice in the header when a newer release is published. Never blocks startup; never modifies the user's machine.
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

[0.1.5]: https://github.com/rekabytes/hmanlab/compare/0.1.4...0.1.5
[0.1.4]: https://github.com/rekabytes/hmanlab/compare/0.1.3...0.1.4
[0.1.3]: https://github.com/rekabytes/hmanlab/compare/0.1.2...0.1.3
[0.1.2]: https://github.com/rekabytes/hmanlab/compare/0.1.1...0.1.2
[0.1.1]: https://github.com/rekabytes/hmanlab/compare/v0.1.0...0.1.1
[0.1.0]: https://github.com/rekabytes/hmanlab/releases/tag/v0.1.0
