//! The async task that drives one chat round: stream tokens from the LLM,
//! dispatch any tool calls the model emits, persist results, and loop until
//! the model produces a final text answer (or exhausts MAX_TURNS).
//!
//! Not part of `App` state — `App` only holds the things the UI needs to
//! render and the channels to talk to this task. The agent runs spawned
//! from `app.rs` and communicates back via `StreamMsg`.

use futures::StreamExt;
use std::path::PathBuf;
use tokio::sync::mpsc;

use crate::app::{LlmBackend, StreamMsg};
use crate::ollama::{ChatMessage, StreamItem, ToolCall};
use crate::tools;

/// Hard cap on agent-loop iterations. If a model keeps calling tools without
/// ever emitting a final reply, we kill the run rather than burning tokens
/// forever — this is a panic-button, not a normal exit condition.
const MAX_TURNS: usize = 10;

pub async fn agent_loop(
    backend: LlmBackend,
    model: String,
    history: Vec<ChatMessage>,
    workspace: PathBuf,
    tx: mpsc::UnboundedSender<StreamMsg>,
) {
    // Bridge confirmation requests from tools.rs to the UI's StreamMsg channel.
    let (confirm_tx, mut confirm_rx) = mpsc::unbounded_channel::<tools::ConfirmRequest>();
    let bridge_tx = tx.clone();
    tokio::spawn(async move {
        while let Some(req) = confirm_rx.recv().await {
            let _ = bridge_tx.send(StreamMsg::ConfirmRequest(req));
        }
    });

    let ctx = tools::ToolContext {
        workspace: workspace.clone(),
        confirm_tx,
    };
    let tool_defs = tools::tool_definitions();

    let mut full_history: Vec<ChatMessage> = vec![ChatMessage {
        role: "system".into(),
        content: tools::system_prompt(&workspace),
        ..Default::default()
    }];
    full_history.extend(history);

    for _ in 0..MAX_TURNS {
        let stream_res = match &backend {
            LlmBackend::Ollama(c) => {
                c.stream_chat(&model, full_history.clone(), Some(tool_defs.clone()))
                    .await
            }
            LlmBackend::OpenAi(c) => {
                c.stream_chat(&model, full_history.clone(), Some(tool_defs.clone()))
                    .await
            }
        };
        let mut stream = match stream_res {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(StreamMsg::Error(e.to_string()));
                return;
            }
        };

        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut turn_prompt_tokens: u32 = 0;
        let mut turn_completion_tokens: u32 = 0;
        while let Some(item) = stream.next().await {
            match item {
                Ok(StreamItem::Content(s)) => {
                    let _ = tx.send(StreamMsg::Chunk(s.clone()));
                    content.push_str(&s);
                }
                Ok(StreamItem::ToolCalls(calls)) => {
                    tool_calls.extend(calls);
                }
                Ok(StreamItem::Done {
                    prompt_tokens,
                    completion_tokens,
                }) => {
                    turn_prompt_tokens = prompt_tokens;
                    turn_completion_tokens = completion_tokens;
                }
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(e.to_string()));
                    return;
                }
            }
        }

        if tool_calls.is_empty() {
            let _ = tx.send(StreamMsg::Done {
                prompt_tokens: turn_prompt_tokens,
                completion_tokens: turn_completion_tokens,
            });
            return;
        }

        let _ = tx.send(StreamMsg::AssistantTurnEnded {
            tool_calls: tool_calls.clone(),
        });
        full_history.push(ChatMessage {
            role: "assistant".into(),
            content: content.clone(),
            tool_calls: Some(tool_calls.clone()),
            name: None,
            hidden: false,
        });

        for tc in &tool_calls {
            let _ = tx.send(StreamMsg::ToolStart {
                name: tc.function.name.clone(),
                args: tc.function.arguments.clone(),
            });
            let output =
                match tools::execute_tool(&tc.function.name, &tc.function.arguments, &ctx).await {
                    Ok(s) => s,
                    Err(e) => format!("error: {e}"),
                };
            let _ = tx.send(StreamMsg::ToolResult {
                output: output.clone(),
            });
            full_history.push(ChatMessage {
                role: "tool".into(),
                content: output,
                name: Some(tc.function.name.clone()),
                tool_calls: None,
                hidden: false,
            });
        }

        let _ = tx.send(StreamMsg::NewAssistantTurn);
    }

    let _ = tx.send(StreamMsg::Error(format!(
        "agent exceeded {MAX_TURNS} turns without producing a final answer"
    )));
}
