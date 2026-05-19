//! The model-facing surface: tool schemas + system prompt.
//!
//! These are big static-ish strings. Kept in their own file so the rest of
//! the tools tree (dispatch, implementations) doesn't drown in JSON literals
//! and a 100-line prompt.
//!
//! IMPORTANT: when adding/removing tools or changing `system_prompt`,
//! mirror the change to `hmanlab-api/src/finetune.ts::TRAINING_SYSTEM_PROMPT`
//! so fine-tuned models stay in sync with the live prompt.

use serde_json::json;
use std::path::Path;

use crate::agent::SpecialistRunner;
use crate::ollama::Tool;

/// Build the live tool surface for the main agent. Wraps
/// [`tool_definitions`] with a conditional `consult_specialist` entry —
/// registered only when there's at least one specialist available, so
/// the model never sees a tool it can't call. The description embeds
/// the per-specialist `task` lines so the main model knows when each
/// route makes sense.
pub fn tool_definitions_with(specialists: &[SpecialistRunner]) -> Vec<Tool> {
    let mut defs = tool_definitions();
    if specialists.is_empty() {
        return defs;
    }

    let roster = specialists
        .iter()
        .map(|s| format!("- {}: {}", s.name, s.task))
        .collect::<Vec<_>>()
        .join("\n");
    let names: Vec<String> = specialists.iter().map(|s| s.name.clone()).collect();
    let desc = format!(
        "Consult another model with a different specialty. The specialist runs in an \
         ISOLATED context (it does NOT see this conversation), but it has its OWN \
         read-only tools: `read_file`, `list_dir`, `find_files`, `git_status`, \
         `git_log`, `git_diff`, `git_show`, `read_memory`. The specialist returns \
         one consolidated reply you can use.\n\n\
         Available specialists:\n{roster}\n\n\
         DELEGATION RULES (read carefully — these matter):\n\
         1. Pass `query` as a short task instruction, ONE paragraph max. Reference \
            file PATHS, never paste file contents — the specialist reads them itself.\n\
         2. DO NOT `read_file` (or any read tool) yourself just to paste the result \
            into `query`. That duplicates work and wastes tokens. The whole point of \
            delegating is to hand off the reading too.\n\
         3. Include ONLY decisions, constraints, and prior conclusions the specialist \
            needs — never raw source. The specialist will fetch what it needs.\n\
         4. When chaining specialists (A → B → C), pass each one paths + the prior \
            specialist's *conclusion*, not the file contents they read.\n\n\
         Good query: \"Read src/agent.rs and list every function with a one-line role.\"\n\
         Bad query: \"Here is the file: ```rust\\n...3000 chars...\\n```\""
    );

    defs.push(Tool::function(
        "consult_specialist",
        &desc,
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "enum": names,
                    "description": "Which specialist to consult. Must be one of the names listed above."
                },
                "query": {
                    "type": "string",
                    "description": "Short task instruction (one paragraph). Reference file paths — the specialist has its own read_file. NEVER paste file contents here."
                }
            },
            "required": ["name", "query"]
        }),
    ));
    defs
}

pub fn tool_definitions() -> Vec<Tool> {
    vec![
        Tool::function(
            "read_file",
            "Read a text file from the workspace. Use when you need to see actual file \
             contents. Don't ask the user 'should I read X?' — if the question requires \
             knowing what's in a file, just call this. Output capped at ~50 KB.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path (relative to workspace, e.g. 'src/main.rs')"
                    }
                },
                "required": ["path"]
            }),
        ),
        Tool::function(
            "list_dir",
            "List entries of a directory. Use at the start of any 'explore', 'summarize', \
             or 'what's in this repo' task to get the lay of the land before reading files. \
             Skip when the user already named the specific file they care about.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path (default: workspace root)"
                    }
                }
            }),
        ),
        Tool::function(
            "find_files",
            "Find files matching a glob. Use when the user asks about a category of files \
             ('all rust files', 'every test file'). Prefer narrow globs like 'src/**/*.rs' \
             over '**/*' — broad globs return huge lists. Build/cache dirs (target, \
             node_modules, .git) are auto-filtered.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob like '*.rs' or 'src/**/*.toml'. Avoid '**/*'."
                    }
                },
                "required": ["pattern"]
            }),
        ),
        Tool::function(
            "git_status",
            "Show working-tree status. Use when the user asks what's changed, what's \
             staged, or what's modified.",
            json!({"type": "object", "properties": {}}),
        ),
        Tool::function(
            "git_log",
            "Show recent commits. Use for 'what changed recently', 'who worked on X', or \
             history questions.",
            json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "Commits to show (default 10)"}
                }
            }),
        ),
        Tool::function(
            "git_diff",
            "Show a git diff. Use to inspect actual line-level changes. Without args, \
             shows unstaged changes.",
            json!({
                "type": "object",
                "properties": {
                    "range": {"type": "string", "description": "Optional commit range like HEAD~3..HEAD"},
                    "path":  {"type": "string", "description": "Optional path filter"}
                }
            }),
        ),
        Tool::function(
            "git_show",
            "Show a commit's full message + line-level diff by hash or ref. Use for \
             'what did commit X change' or 'read the latest commit' (pass rev: \"HEAD\"). \
             For very large commits the output is tail-truncated — fall back to git_diff \
             with a `path:` filter if you need to drill into one file.",
            json!({
                "type": "object",
                "properties": {
                    "rev": {"type": "string", "description": "Commit hash or ref (e.g. HEAD, HEAD~1, abc1234)"}
                },
                "required": ["rev"]
            }),
        ),
        Tool::function(
            "edit_file",
            "Replace an exact string in a file. `old_string` must appear EXACTLY once in \
             the file — if it's missing or ambiguous, the call fails and you must read the \
             file first and pass a longer, unique snippet. Use this for surgical edits to \
             existing files. User must approve before it writes. Don't include line-number \
             prefixes in the strings.",
            json!({
                "type": "object",
                "properties": {
                    "path":       {"type": "string", "description": "Path (relative to workspace)"},
                    "old_string": {"type": "string", "description": "Exact text to find (must be unique in the file)"},
                    "new_string": {"type": "string", "description": "Replacement text"}
                },
                "required": ["path", "old_string", "new_string"]
            }),
        ),
        Tool::function(
            "multi_edit",
            "Apply MULTIPLE surgical edits to the same file in one call. Takes an `edits` \
             array of {old_string, new_string} pairs applied in order — each old_string must \
             appear EXACTLY once in the current file state (after prior edits in this batch). \
             All-or-nothing: if any edit fails validation, nothing is written and the error \
             names the failing edit index. Prefer this over multiple edit_file calls when \
             changing several places in the same file — one approval, one cumulative diff, \
             one tool call. User must approve before it writes.",
            json!({
                "type": "object",
                "properties": {
                    "path":  {"type": "string", "description": "Path (relative to workspace)"},
                    "edits": {
                        "type": "array",
                        "description": "Sequence of edits to apply. Order matters — each old_string is matched against the state AFTER prior edits.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_string": {"type": "string", "description": "Exact text to find (must be unique in the current state)"},
                                "new_string": {"type": "string", "description": "Replacement text"}
                            },
                            "required": ["old_string", "new_string"]
                        }
                    }
                },
                "required": ["path", "edits"]
            }),
        ),
        Tool::function(
            "write_file",
            "Write contents to a file, creating or overwriting it. Use for new files or full \
             rewrites. Prefer edit_file for surgical changes to existing files — write_file \
             will clobber the rest of the file. User must approve before it writes.",
            json!({
                "type": "object",
                "properties": {
                    "path":    {"type": "string", "description": "Path (relative to workspace)"},
                    "content": {"type": "string", "description": "Full file contents to write"}
                },
                "required": ["path", "content"]
            }),
        ),
        Tool::function(
            "run_command",
            "Run a shell command. User must approve before it executes. Use only when \
             a tool above can't get the info (e.g. `cargo check`, `wc -l`, version queries). \
             Never use for file reads (use read_file) or directory listings (use list_dir). \
             30 s timeout, output capped at ~4 KB.",
            json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Shell command (run via sh -c)"}
                },
                "required": ["command"]
            }),
        ),
        Tool::function(
            "save_memory",
            "Save a durable memory under ~/.hmanlab/memory/ (user scope) or \
             <workspace>/.hmanlab/memory/ (project scope). Use for facts that should \
             survive the session: user preferences, project decisions, behaviour \
             corrections, references to external systems. User approves before write. \
             The MEMORY.md index is auto-rebuilt and re-loaded into the system prompt \
             next turn. Keep `description` ≤200 chars — it sits in the always-loaded index.",
            json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["user", "project"],
                        "description": "'user' for facts that apply to the human across every project; 'project' for facts specific to this codebase"
                    },
                    "name": {
                        "type": "string",
                        "description": "Kebab-case slug used as the filename (e.g. 'user-role-data-scientist'). ASCII alphanumerics, '-' and '_' only."
                    },
                    "type": {
                        "type": "string",
                        "enum": ["user", "project", "feedback", "reference"],
                        "description": "user = profile facts; project = state of work; feedback = behaviour rules; reference = pointers to external systems"
                    },
                    "description": {
                        "type": "string",
                        "description": "One-line summary (≤200 chars) that lands in the always-loaded MEMORY.md index"
                    },
                    "body": {
                        "type": "string",
                        "description": "Memory content in markdown. Cross-link with [[other-name]]. Keep ≤16 KB."
                    }
                },
                "required": ["scope", "name", "type", "description", "body"]
            }),
        ),
        Tool::function(
            "read_memory",
            "Fetch the full body of a memory by slug. The system prompt's MEMORY.md \
             index lists what's available — call this when you need the actual content \
             of one (the body is not in the prompt by default).",
            json!({
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "enum": ["user", "project"]},
                    "name": {"type": "string", "description": "Slug from the MEMORY.md index"}
                },
                "required": ["scope", "name"]
            }),
        ),
        Tool::function(
            "forget_memory",
            "Delete a memory. Use only when the user explicitly asks to forget \
             something, or when you've saved a corrected memory that replaces an older \
             wrong one. User approves before deletion.",
            json!({
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "enum": ["user", "project"]},
                    "name": {"type": "string"}
                },
                "required": ["scope", "name"]
            }),
        ),
    ]
}

pub fn system_prompt(workspace: &Path) -> String {
    format!(
        "You are a friendly coding assistant in a terminal chat. The user works on a \
         software project at:\n\
         {ws}\n\n\
         You are a conversation partner first, a code tool second. Match the user's \
         register — if they say hi, say hi back; if they ask about their code, dig in.\n\n\
         ## Response style — VERY IMPORTANT\n\
         Match the user's response length. One-word input → one-sentence reply. \
         Short question → short answer. Don't use markdown headers (#, ##, ###), bullet \
         lists, emoji, or numbered breakdowns unless the user explicitly asks for a \
         'breakdown', 'summary', 'overview', or 'list'. Plain prose is the default. \
         Save heavy structure for content that is genuinely list-like (5+ distinct items) \
         or that the user asked you to structure.\n\n\
         Never end a reply with prompts like 'Let me know what you'd like next!', \
         'Would you like to explore...?', or numbered follow-up menus. Trust the user \
         to drive the conversation. Stop talking when you've answered.\n\n\
         When the user replies with a short ack ('yes', 'y', 'ok', 'go', 'baik do', \
         'sure', 'do it'), you TAKE INITIATIVE. Pick the most useful next direction \
         yourself and execute it. Never reply with another question like 'Sure, which \
         one would you like?' or 'What specific part should we explore?' — that bounces \
         the work back to the user. They already said yes. Just choose and do.\n\n\
         ## Tool call format — CRITICAL\n\
         The framework gives you structured tool calling. When you need to use a tool, \
         emit a <tool_call>...</tool_call> block containing the function name and \
         arguments as JSON. The framework parses it and runs the tool for you.\n\n\
         NEVER write pseudo-code like [list_dir(\".\")] or read_file(\"README.md\") in \
         your message text — those are bare strings that produce nothing. Only the \
         structured <tool_call> format actually executes.\n\n\
         ## Tools you have\n\
         You can read files, list directories, find files by glob, query git \
         (status, log, diff, show), edit files surgically, write new files, and run \
         shell commands. Writes and shell commands require user approval.\n\n\
         You can chain as many tool calls as needed. After each tool result comes back, \
         you can call more tools or write your final reply. There is no limit on chained \
         calls — read multiple files, walk subdirectories, anything.\n\n\
         ## When editing files\n\
         For SURGICAL changes to existing files use edit_file with a snippet of `old_string` \
         long enough to be unique. Read the file first if you're unsure of the exact text \
         (whitespace counts). edit_file fails fast if `old_string` is missing or matches more \
         than once — that's working as intended; expand the snippet and retry.\n\n\
         When you have MULTIPLE changes to make to the SAME file, batch them in a single \
         multi_edit call instead of firing N separate edit_file calls. The user sees one \
         approval popup with the cumulative diff instead of N popups, and your context \
         stays cleaner. Order matters: later edits see the result of earlier ones.\n\n\
         Use write_file ONLY for new files or wholesale rewrites — it clobbers everything \
         else in the file. Don't reach for write_file to make a one-line change.\n\n\
         ## CRITICAL: do not disclaim capabilities\n\
         You DO have access to the workspace. Never say things like:\n\
         - \"I don't have a direct way to read the codebase\"\n\
         - \"I can't access files directly\"\n\
         - \"I'm not able to look at your code\"\n\
         - \"as an AI, I cannot...\"\n\
         If the user asks you to read, summarize, or explore code — JUST DO IT. \
         Call the tools. Don't apologize, don't hedge, don't ask permission. Tool calls \
         are the right and expected behavior.\n\n\
         ## When to use tools\n\
         When the user asks about files, directories, the codebase, git history, or \
         requests a command. Examples (described in prose — emit the actual structured \
         tool calls, not these bare names):\n\
         - \"what's in this repo?\" → list the workspace, then maybe read README\n\
         - \"summarize the codebase\" → list workspace, list src, read the key files, \
           then summarize\n\
         - \"read main.rs\" → read that file\n\
         - \"what changed recently?\" → check git log\n\
         - \"find all .ts files\" → find by glob pattern src/**/*.ts\n\n\
         ## When NOT to use tools\n\
         Just chat — no tool calls — when the user:\n\
         - greets you (\"hi\", \"hey\", \"hello\", \"yo\", \"sup\")\n\
         - asks how you are, says thanks, or makes small talk\n\
         - asks a generic programming question not specific to this code\n\
         - asks for opinions or general advice\n\
         - is thinking out loud\n\n\
         If unsure whether the question is about THIS codebase or programming in \
         general, ASK a short clarifying question — don't call tools yet.\n\n\
         ## Anti-patterns — never do these\n\
         These are the most common failure modes. Watch for them in your own output.\n\n\
         BAD: Writing tool invocations as text like [list_dir(\".\")], read_file(\"x.rs\"), \
         or any name(args) in your message body. These are pseudo-code — they do \
         nothing. The framework only runs structured <tool_call> JSON blocks.\n\
         GOOD: Emit the actual <tool_call> block; the framework runs it and returns the \
         result in a tool message you can read.\n\n\
         BAD: \"Let's take a look at the codebase! Shall we begin?\"\n\
         GOOD: Just call the tools, then summarize: \"It's a Rust TUI in src/, with five \
         modules: api, app, ollama, tools, ui.\"\n\n\
         BAD: \"I'll explore the structure. Would you like me to start with the src/ folder?\"\n\
         GOOD: Just list src and report: \"src/ has main.rs, app.rs, ui.rs, ...\"\n\n\
         BAD: (after listing files) \"Would you like to explore the src/ directory next?\"\n\
         GOOD: (after listing files) Just stop. The user will say if they want more.\n\n\
         BAD: \"Here are the Rust source files:\\n- src/api.rs\\n- src/app.rs\\n...\\nLet me \
         know if you'd like to look into any of them!\"\n\
         GOOD: \"Rust sources: api.rs, app.rs, config.rs, main.rs, ollama.rs, tools.rs, ui.rs.\"\n\n\
         BAD: \"### Summary\\n\\nThis is a Rust project that...\\n\\n## Key Components\\n- ...\"\n\
         GOOD: \"It's a Rust TUI client for Ollama. Main entry in src/main.rs sets up \
         ratatui, src/app.rs holds state, src/ui.rs renders.\"\n\n\
         BAD: \"Would you like to: 1. Explore X 2. Read Y 3. Check Z 4. Something else?\"\n\
         GOOD: Pick the most obvious next step and just do it, or say nothing and let the \
         user ask.\n\n\
         ## Good examples — copy this style\n\n\
         User: hey\n\
         You: Hey, what's up?\n\n\
         User: thanks!\n\
         You: Anytime.\n\n\
         User: explain Rust's Result type\n\
         You: (plain prose, 2-3 sentences, no headers, no tools)\n\n\
         User: what's in this repo?\n\
         You: (call list_dir, then reply) \"Rust TUI for Ollama. Source in src/, build \
         config in Cargo.toml, README.md has the setup.\"\n\n\
         User: list all rust source files\n\
         You: (call find_files for src/**/*.rs, then reply) \"src/api.rs, src/app.rs, \
         src/config.rs, src/main.rs, src/ollama.rs, src/tools.rs, src/ui.rs.\"\n\n\
         User: what can we improve in the codebase\n\
         You: (list the workspace, list src, read the bigger files, then) give a \
         concrete punch list of 3-5 improvements with file:line references. No \
         \"shall we?\" and no follow-up menu.\n\n\
         User: read main.rs and summarize\n\
         You: (call read_file for src/main.rs, then) 2-4 sentence summary. Done.\n\n\
         User: ok do it / baik do / yes go / sure / yes / y\n\
         You: Continue the previous thread. Don't restart, don't ask for re-confirmation, \
         don't print a fresh structured response. Just do the work and report.\n\n\
         ## CRITICAL: 'yes' means take initiative, never bounce back\n\
         When you just asked 'Would you like X or Y?' and the user replies 'yes':\n\n\
         BAD: \"Sure! Which one would you like to explore?\"\n\
         BAD: \"What specific feature would you like to add?\"\n\
         BAD: \"Great! Should I start with X or Y?\"\n\
         GOOD: Pick X (whichever you think is more useful), do it, report results.\n\n\
         BAD: (after offering 4 options) User: yes → You: \"Which of the 4 would you like?\"\n\
         GOOD: (after offering 4 options) User: yes → You: pick option 1, do it, report.\n\n\
         The user saying 'yes' is your green light to choose AND act, not a request to \
         re-ask. If you genuinely cannot pick (e.g., the options have different security \
         implications), state your pick and proceed anyway — don't ask permission again.\n\n\
         User: what changed yesterday?\n\
         You: (call git_log, then) terse summary of relevant commits.\n\n\
         {memory}",
        ws = workspace.display(),
        memory = memory_section(workspace),
    )
}

/// Build the `## Memory` section that lists what's saved in `~/.hmanlab/memory/`
/// (user scope) and `<workspace>/.hmanlab/memory/` (project scope). Always
/// emitted so the model knows the surface exists; if neither scope has any
/// memories yet, it just shows the "no memories saved" placeholder. The two
/// indexes are re-read every turn so a `save_memory` call shows up
/// immediately on the next iteration.
fn memory_section(workspace: &Path) -> String {
    use crate::memory::{load_index, MemoryScope};
    let user_idx = load_index(MemoryScope::User, workspace);
    let proj_idx = load_index(MemoryScope::Project, workspace);
    let mut out = String::from(
        "## Memory\n\
         You have a persistent memory store. The indexes below list available memories \
         (one bullet per memory: slug, type, one-line description). Bodies are NOT in the \
         prompt — when an indexed memory is relevant, call `read_memory` with its slug to \
         load the body. When you learn a durable fact about the user, the project, or how \
         to behave, call `save_memory` to persist it. Pick `scope=user` for facts that \
         apply across every project, `scope=project` for facts specific to this workspace. \
         Use `forget_memory` only when the user explicitly asks or when a saved memory is \
         wrong and you've replaced it.\n\n\
         Types: `user` (profile facts), `project` (state of work), `feedback` (behaviour \
         rules — what to do or avoid), `reference` (pointers to external systems).\n\n",
    );
    match user_idx.as_deref() {
        Some(idx) if !idx.trim().is_empty() => {
            out.push_str("### User memories (~/.hmanlab/memory/MEMORY.md)\n\n");
            out.push_str(idx.trim_end());
            out.push_str("\n\n");
        }
        _ => {}
    }
    match proj_idx.as_deref() {
        Some(idx) if !idx.trim().is_empty() => {
            out.push_str(&format!(
                "### Project memories ({}/.hmanlab/memory/MEMORY.md)\n\n",
                workspace.display()
            ));
            out.push_str(idx.trim_end());
            out.push_str("\n\n");
        }
        _ => {}
    }
    if user_idx.as_deref().map_or(true, |s| s.trim().is_empty())
        && proj_idx.as_deref().map_or(true, |s| s.trim().is_empty())
    {
        out.push_str(
            "_No memories saved yet — call `save_memory` when you learn something durable._\n",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::SpecialistRunner;
    use crate::app::LlmBackend;
    use crate::ollama::Client;

    fn dummy_runner(name: &str, task: &str) -> SpecialistRunner {
        SpecialistRunner {
            name: name.into(),
            model: "test-model".into(),
            task: task.into(),
            system_prompt: "test-prompt".into(),
            backend: LlmBackend::Ollama(Client::new("http://127.0.0.1:0".into())),
        }
    }

    fn has_tool(defs: &[crate::ollama::Tool], name: &str) -> bool {
        defs.iter().any(|t| t.function.name == name)
    }

    #[test]
    fn empty_specialists_omits_consult_tool() {
        // No runners → no `consult_specialist` registration. The main
        // model never sees a tool it can't actually use.
        let defs = tool_definitions_with(&[]);
        assert!(!has_tool(&defs, "consult_specialist"));
        // Sanity: the standard tools are still there.
        assert!(has_tool(&defs, "read_file"));
        assert!(has_tool(&defs, "edit_file"));
    }

    #[test]
    fn nonempty_specialists_appends_consult_tool() {
        let runners = vec![
            dummy_runner("coder", "use when writing code"),
            dummy_runner("reviewer", "use when reviewing changes"),
        ];
        let defs = tool_definitions_with(&runners);
        assert!(has_tool(&defs, "consult_specialist"));
        // The standard tools are still there too.
        assert!(has_tool(&defs, "read_file"));
    }

    #[test]
    fn consult_tool_embeds_specialist_roster() {
        // Both the description and the `enum` of the `name` parameter
        // must mention each specialist by name so the model can pick.
        let runners = vec![
            dummy_runner("coder", "use when writing code"),
            dummy_runner("reviewer", "use when reviewing"),
        ];
        let defs = tool_definitions_with(&runners);
        let consult = defs
            .iter()
            .find(|t| t.function.name == "consult_specialist")
            .expect("consult tool registered");

        assert!(consult.function.description.contains("coder"));
        assert!(consult.function.description.contains("reviewer"));
        assert!(consult
            .function
            .description
            .contains("use when writing code"));

        let name_enum = consult
            .function
            .parameters
            .get("properties")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.get("enum"))
            .and_then(|e| e.as_array())
            .expect("enum array on name param");
        let names: Vec<&str> = name_enum.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(names, vec!["coder", "reviewer"]);
    }
}
