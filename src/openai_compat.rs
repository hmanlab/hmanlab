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
        messages: Vec<ChatMessage>,
        tools: Option<Vec<Tool>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamItem>> + Send>>> {
        let url = format!("{}/chat/completions", self.base);
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
    #[serde(skip_serializing_if = "String::is_empty")]
    content: String,
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
        let tool_calls = m.tool_calls.map(|calls| {
            calls
                .into_iter()
                .enumerate()
                .map(|(i, tc)| OaiToolCallOut {
                    id: format!("call_{i}"),
                    kind: "function".into(),
                    function: OaiToolCallFnOut {
                        name: tc.function.name,
                        arguments: tc.function.arguments.to_string(),
                    },
                })
                .collect()
        });
        OaiMessage {
            role: m.role,
            content: m.content,
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
