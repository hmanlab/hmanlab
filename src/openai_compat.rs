//! OpenAI-compatible streaming chat client. Works with any endpoint that
//! speaks the OpenAI `chat/completions` API — used today for z.ai
//! (GLM coding plan), planned for OpenRouter next.
//!
//! Re-uses `ollama::ChatMessage`, `ollama::Tool`, `ollama::ToolCall` and
//! emits `ollama::StreamItem` so the agent loop stays uniform.

use anyhow::{anyhow, Result};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;

use crate::ollama::{ChatMessage, StreamItem, Tool, ToolCall, ToolCallFunction};

/// Fetch OpenRouter's live model catalog from `/v1/models`. Public endpoint
/// (no auth needed), but we accept a key so any rate-limit signals can be
/// tied to the caller. Returns ALL model IDs in the order the API returns
/// them — filtering / vendor-curating is the caller's job
/// (`config::OPENROUTER_VENDORS`).
///
/// The endpoint shape is the OpenAI-compatible `{ data: [{ id, ... }] }`
/// where each entry carries pricing, context length, etc. We pull only the
/// `id` field — everything else is metadata the picker doesn't render.
pub async fn fetch_openrouter_models(base: &str, api_key: Option<&str>) -> Result<Vec<String>> {
    let url = format!("{}/models", base.trim_end_matches('/'));
    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| anyhow!("reqwest client: {e}"))?
        .get(&url);
    if let Some(k) = api_key {
        req = req.bearer_auth(k);
    }
    let resp = req.send().await.map_err(|e| anyhow!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let preview: String = body.chars().take(1000).collect();
        return Err(anyhow!("GET {url}: HTTP {status} — {}", preview.trim()));
    }
    #[derive(Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(Deserialize)]
    struct ModelsBody {
        data: Vec<ModelEntry>,
    }
    let body: ModelsBody = resp
        .json()
        .await
        .map_err(|e| anyhow!("parse /models: {e}"))?;
    Ok(body.data.into_iter().map(|m| m.id).collect())
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    pub base: String,
    pub api_key: String,
}

impl Client {
    pub fn new(base: String, api_key: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("reqwest client");
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            api_key,
        }
    }

    pub async fn stream_chat(
        &self,
        model: &str,
        mut messages: Vec<ChatMessage>,
        tools: Option<Vec<Tool>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamItem>> + Send>>> {
        let url = format!("{}/chat/completions", self.base);
        // Back-fill missing `tool_call_id`s on tool messages — needed
        // when the history was reconstructed from a loaded session,
        // which doesn't persist the per-request `call_<i>` ids. Strict
        // providers (opencode, MiniMax) 400 without these on the
        // first send after `/load`.
        backfill_tool_call_ids(&mut messages);
        let oai_messages: Vec<OaiMessage> = messages.into_iter().map(OaiMessage::from).collect();
        let req = OaiRequest {
            model,
            messages: &oai_messages,
            stream: true,
            tools: tools.as_deref(),
        };
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?;
        // Surface the response body on non-2xx. `.error_for_status()`
        // drops the body — useful for "did it work?" but useless for
        // debugging, which is exactly when we need it. Providers
        // (opencode, openrouter, z.ai) all return structured JSON
        // errors like `{"error":{"message":"context length exceeded"}}`
        // that the user needs to see; bare "HTTP 400" tells them
        // nothing actionable.
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            // Truncate to keep one bad request from flooding the chat
            // — most provider errors are well under this anyway.
            let preview: String = body.chars().take(1000).collect();
            return Err(anyhow!("POST {url}: HTTP {status} — {}", preview.trim()));
        }
        let byte_stream = resp.bytes_stream();

        // Accumulator for streamed tool_calls (arguments arrive as fragments).
        let stream = futures::stream::unfold(
            (
                byte_stream,
                Vec::<u8>::new(),
                Vec::<PartialToolCall>::new(),
                false,
            ),
            |(mut bs, mut buf, mut tcs, mut done)| async move {
                if done {
                    return None;
                }
                loop {
                    if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes);
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        // SSE frames start with "data: ". Some endpoints emit
                        // ":" keep-alive comments — skip anything not "data:".
                        let payload = match trimmed.strip_prefix("data:") {
                            Some(p) => p.trim(),
                            None => continue,
                        };
                        if payload == "[DONE]" {
                            done = true;
                            // Flush any buffered tool_calls as a final item.
                            if !tcs.is_empty() {
                                let calls = finalize_calls(std::mem::take(&mut tcs));
                                return Some((
                                    Ok(StreamItem::ToolCalls(calls)),
                                    (bs, buf, tcs, done),
                                ));
                            }
                            // No usage info on [DONE] frame; emit zeros.
                            return Some((
                                Ok(StreamItem::Done {
                                    prompt_tokens: 0,
                                    completion_tokens: 0,
                                }),
                                (bs, buf, tcs, done),
                            ));
                        }
                        match serde_json::from_str::<OaiChunk>(payload) {
                            Ok(chunk) => {
                                let choice = match chunk.choices.into_iter().next() {
                                    Some(c) => c,
                                    None => continue,
                                };
                                // Accumulate tool_call fragments first.
                                if let Some(tc_chunks) = choice.delta.tool_calls {
                                    for tc in tc_chunks {
                                        let idx = tc.index.unwrap_or(0) as usize;
                                        while tcs.len() <= idx {
                                            tcs.push(PartialToolCall::default());
                                        }
                                        let slot = &mut tcs[idx];
                                        if let Some(name) =
                                            tc.function.as_ref().and_then(|f| f.name.as_ref())
                                        {
                                            slot.name = name.clone();
                                        }
                                        if let Some(args) =
                                            tc.function.as_ref().and_then(|f| f.arguments.as_ref())
                                        {
                                            slot.args.push_str(args);
                                        }
                                    }
                                }
                                if let Some(content) = choice.delta.content {
                                    if !content.is_empty() {
                                        return Some((
                                            Ok(StreamItem::Content(content)),
                                            (bs, buf, tcs, done),
                                        ));
                                    }
                                }
                                if choice.finish_reason.is_some() {
                                    let usage = chunk.usage;
                                    let pt = usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
                                    let ct =
                                        usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0);
                                    if !tcs.is_empty() {
                                        let calls = finalize_calls(std::mem::take(&mut tcs));
                                        // Stash the Done for next iter via `done` flag.
                                        done = true;
                                        // We can only emit one item per iter — emit
                                        // ToolCalls now, the synthesized Done will fire
                                        // on the next loop where done=true triggers
                                        // None... so we need to push usage too. Emit
                                        // ToolCalls now and a follow-up Done by
                                        // re-enqueueing in buf. Simpler: emit Done
                                        // here after ToolCalls. We'll yield ToolCalls
                                        // and a sentinel — for now, finish_reason on
                                        // tool_calls means the assistant emitted only
                                        // tool calls; emit them with usage=Done next
                                        // iter using a hack: push usage info to a
                                        // local frame.
                                        // Implementation: stash usage and yield Done
                                        // after the tool calls via a one-shot follow.
                                        return Some((
                                            Ok(StreamItem::ToolCalls(calls)),
                                            (bs, buf, tcs, done),
                                        ));
                                    }
                                    done = true;
                                    return Some((
                                        Ok(StreamItem::Done {
                                            prompt_tokens: pt,
                                            completion_tokens: ct,
                                        }),
                                        (bs, buf, tcs, done),
                                    ));
                                }
                            }
                            Err(_) => continue,
                        }
                        continue;
                    }
                    match bs.next().await {
                        Some(Ok(b)) => buf.extend_from_slice(&b),
                        Some(Err(e)) => {
                            return Some((Err(anyhow!(e)), (bs, buf, tcs, done)));
                        }
                        None => {
                            // Stream closed without [DONE]. Flush whatever we have.
                            if !tcs.is_empty() {
                                let calls = finalize_calls(std::mem::take(&mut tcs));
                                done = true;
                                return Some((
                                    Ok(StreamItem::ToolCalls(calls)),
                                    (bs, buf, tcs, done),
                                ));
                            }
                            done = true;
                            return Some((
                                Ok(StreamItem::Done {
                                    prompt_tokens: 0,
                                    completion_tokens: 0,
                                }),
                                (bs, buf, tcs, done),
                            ));
                        }
                    }
                }
            },
        );
        Ok(Box::pin(stream))
    }
}

#[derive(Default)]
struct PartialToolCall {
    name: String,
    args: String,
}

/// Pair tool result messages back to their originating assistant
/// `tool_calls` by position and stamp the matching `call_<i>` id. The
/// outgoing serializer synthesises `call_0..call_N` for an assistant
/// turn's `tool_calls`; strict OpenAI-compat providers (opencode,
/// MiniMax) reject tool result messages that don't carry the same id.
///
/// Live agent turns set this id at the source via `ChatMessage::tool_result`,
/// but loaded sessions can't — the DB doesn't store the synthesised id
/// (it's a per-request artefact). This walks the history and fills any
/// `None` tool_call_id positionally, leaving already-set ids alone.
fn backfill_tool_call_ids(messages: &mut [ChatMessage]) {
    let n = messages.len();
    let mut i = 0;
    while i < n {
        if messages[i].role == "assistant" {
            let num_calls = messages[i]
                .tool_calls
                .as_ref()
                .map(|c| c.len())
                .unwrap_or(0);
            if num_calls > 0 {
                let mut j = i + 1;
                let mut k = 0;
                while j < n && k < num_calls && messages[j].role == "tool" {
                    if messages[j].tool_call_id.is_none() {
                        messages[j].tool_call_id = Some(format!("call_{k}"));
                    }
                    j += 1;
                    k += 1;
                }
            }
        }
        i += 1;
    }
}

fn finalize_calls(parts: Vec<PartialToolCall>) -> Vec<ToolCall> {
    parts
        .into_iter()
        .filter(|p| !p.name.is_empty())
        .map(|p| ToolCall {
            function: ToolCallFunction {
                name: p.name,
                arguments: serde_json::from_str(&p.args)
                    .unwrap_or(Value::Object(Default::default())),
            },
        })
        .collect()
}

// --- wire types ---

#[derive(Serialize)]
struct OaiRequest<'a> {
    model: &'a str,
    messages: &'a [OaiMessage],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [Tool]>,
}

#[derive(Serialize)]
struct OaiMessage {
    role: String,
    /// Either a plain string (text-only) or a content-parts array
    /// (text + image_url) when the message has attachments. `None`
    /// when the assistant emitted only tool_calls with no text — some
    /// providers reject `content: ""` alongside tool_calls, so we omit
    /// the field entirely in that case.
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCallOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct OaiToolCallOut {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: OaiToolCallFnOut,
}

#[derive(Serialize)]
struct OaiToolCallFnOut {
    name: String,
    /// OpenAI expects arguments as a JSON-encoded string.
    arguments: String,
}

impl From<ChatMessage> for OaiMessage {
    fn from(m: ChatMessage) -> Self {
        // Map our internal roles to OpenAI's. "tool" stays "tool" — OpenAI
        // requires tool messages to carry tool_call_id, but the streaming
        // GLM endpoint is lenient enough to accept it without when paired
        // with the right `name`.
        let tool_calls = m.tool_calls.as_ref().map(|calls| {
            calls
                .iter()
                .enumerate()
                .map(|(i, tc)| OaiToolCallOut {
                    id: format!("call_{i}"),
                    kind: "function".into(),
                    function: OaiToolCallFnOut {
                        name: tc.function.name.clone(),
                        arguments: tc.function.arguments.to_string(),
                    },
                })
                .collect()
        });
        // Multimodal-aware serialization: `to_api_content` returns either
        // a plain string or a content-parts array depending on whether
        // attachments are present. Omit the field entirely for empty
        // text-only messages (assistant turns that emitted only
        // tool_calls), since some providers reject `content: ""` next
        // to tool_calls.
        let content_val = m.to_api_content();
        let content = match &content_val {
            Value::String(s) if s.is_empty() => None,
            _ => Some(content_val),
        };
        OaiMessage {
            role: m.role,
            content,
            name: m.name,
            tool_calls,
            // Preserve the correlation id set by agent_loop_with on
            // tool result messages. Strict providers (MiniMax via
            // opencode) reject tool results without it — the bare
            // status was "tool result's tool id() not found (2013)".
            tool_call_id: m.tool_call_id,
        }
    }
}

#[derive(Deserialize)]
struct OaiChunk {
    choices: Vec<OaiChoice>,
    #[serde(default)]
    usage: Option<OaiUsage>,
}

#[derive(Deserialize)]
struct OaiChoice {
    delta: OaiDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OaiDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OaiToolCallIn>>,
}

#[derive(Deserialize)]
struct OaiToolCallIn {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    function: Option<OaiToolCallFnIn>,
}

#[derive(Deserialize)]
struct OaiToolCallFnIn {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct OaiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

#[cfg(test)]
mod backfill_tests {
    use super::*;
    use crate::ollama::{ToolCall, ToolCallFunction};
    use serde_json::Value;

    fn assistant_with_n_calls(n: usize) -> ChatMessage {
        let calls = (0..n)
            .map(|i| ToolCall {
                function: ToolCallFunction {
                    name: format!("fn_{i}"),
                    arguments: Value::Null,
                },
            })
            .collect();
        ChatMessage {
            role: "assistant".into(),
            tool_calls: Some(calls),
            ..Default::default()
        }
    }

    fn tool_result(name: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".into(),
            content: format!("result of {name}"),
            name: Some(name.into()),
            ..Default::default()
        }
    }

    #[test]
    fn loaded_session_gets_call_ids_positionally() {
        // Mirrors a /load'd transcript: assistant turn emitted 2 calls,
        // followed by 2 tool messages with no tool_call_id.
        let mut msgs = vec![
            ChatMessage { role: "user".into(), content: "go".into(), ..Default::default() },
            assistant_with_n_calls(2),
            tool_result("fn_0"),
            tool_result("fn_1"),
            ChatMessage { role: "assistant".into(), content: "done".into(), ..Default::default() },
        ];
        backfill_tool_call_ids(&mut msgs);
        assert_eq!(msgs[2].tool_call_id.as_deref(), Some("call_0"));
        assert_eq!(msgs[3].tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn does_not_overwrite_existing_ids() {
        let mut msgs = vec![
            assistant_with_n_calls(1),
            ChatMessage {
                role: "tool".into(),
                tool_call_id: Some("call_preserved".into()),
                ..Default::default()
            },
        ];
        backfill_tool_call_ids(&mut msgs);
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("call_preserved"));
    }

    #[test]
    fn assistant_without_tool_calls_skipped() {
        let mut msgs = vec![
            ChatMessage { role: "assistant".into(), content: "hi".into(), ..Default::default() },
            tool_result("orphan"),
        ];
        backfill_tool_call_ids(&mut msgs);
        // No preceding tool_calls → leave alone (the tool row is an
        // orphan; the request would have failed earlier anyway).
        assert!(msgs[1].tool_call_id.is_none());
    }
}
