# hmanlab

A terminal UI for chatting with local [Ollama](https://ollama.com) models — plus BYOK providers (z.ai, Ollama Cloud, OpenCode Go) — built in Rust with [ratatui](https://ratatui.rs).

```bash
# Global install
npm install -g hmanlab

# Run once without installing
npx hmanlab

# Per-project (e.g. add to a repo's dev deps)
npm install --save-dev hmanlab
npx hmanlab
```

## Features

- Streaming replies, agentic tool calls (`read_file`, `edit_file`, `run_command`, git, find), foldable `<think>` blocks
- BYOK providers: z.ai (subscription + usage), Ollama Cloud, OpenCode Go
- Session persistence via `hmanlab-api`, with `/sessions`, `/load`, `/more`
- Workspace sidebar with click-to-expand folders, scroll, click-to-open file viewer
- `/compact` slash command + automatic compaction at high context tokens; compactions are persisted to `<workspace>/.hmanlab/memory/compact-current.md` so the model can resume across sessions
- Memory store at `~/.hmanlab/memory/` (user-scope) and `<workspace>/.hmanlab/memory/` (project-scope), surfaced to the model every turn

## Supported platforms

Prebuilt binaries ship for:

- `linux-x64`, `linux-arm64` (musl, statically-linked)
- `darwin-x64`, `darwin-arm64`
- `win32-x64`

On other platforms, `npm install` will succeed but `hmanlab` will print a friendly "no prebuilt binary" message and exit. Build from source via `cargo install --git https://github.com/rekabytes/hmanlab`.

## Where does `.hmanlab/` live?

Wherever you launch `hmanlab` from — that becomes the **workspace**:

- Project install: `npx hmanlab` from a project dir → `<project>/.hmanlab/`
- Global install: `cd ~/myrepo && hmanlab` → `~/myrepo/.hmanlab/`. `cd ~ && hmanlab` → `~/.hmanlab/`

User-scope state (cross-project preferences, identity) always lives at `~/.hmanlab/`.

## License

MIT. See https://github.com/rekabytes/hmanlab for source.
