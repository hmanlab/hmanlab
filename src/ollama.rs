use anyhow::{anyhow, Result};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Function name for tool-result messages (role="tool").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Locally-injected user messages (Y/N quick-reply) that go to the model
    /// but never render in the chat UI. Never sent over the wire.
    #[serde(skip)]
    pub hidden: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub function: ToolCallFunction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunctionSpec,
}

#[derive(Clone, Debug, Serialize)]
pub struct ToolFunctionSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl Tool {
    pub fn function(name: &str, description: &str, parameters: Value) -> Self {
        Self {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    stream: bool,
    /// Disables the built-in chain-of-thought on thinking models (qwen3,
    /// deepseek-r1, etc.). No effect on non-thinking models.
    think: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [Tool]>,
}

#[derive(Deserialize)]
struct ChatChunk {
    message: Option<ChunkMessage>,
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Default, Deserialize)]
struct ChunkMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<ModelInfo>,
}

#[derive(Deserialize)]
struct ModelInfo {
    name: String,
}

/// What a single streamed chunk yields back to the agent loop.
#[derive(Debug, Clone)]
pub enum StreamItem {
    Content(String),
    ToolCalls(Vec<ToolCall>),
    Done {
        prompt_tokens: u32,
        completion_tokens: u32,
    },
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    pub base: String,
    /// Bearer token for Ollama Cloud's hosted API. `None` for local daemons,
    /// which don't authenticate. Set via `with_api_key` so the local
    /// constructor path stays unchanged.
    api_key: Option<String>,
}

impl Client {
    pub fn new(base: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .expect("reqwest client");
        let base = base.trim_end_matches('/').to_string();
        Self { http, base, api_key: None }
    }

    /// Construct a client that authenticates every request with a Bearer
    /// token — the shape Ollama Cloud's public API expects.
    pub fn with_api_key(base: String, api_key: String) -> Self {
        let mut c = Self::new(base);
        c.api_key = Some(api_key);
        c
    }

    /// Attach the Bearer header when this client was built with an API key.
    /// No-op for keyless local daemons, which short-circuit auth entirely.
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(k) => req.bearer_auth(k),
            None => req,
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", self.base);
        let resp: TagsResponse = self
            .auth(self.http.get(&url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.models.into_iter().map(|m| m.name).collect())
    }

    pub async fn stream_chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<Tool>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamItem>> + Send>>> {
        let url = format!("{}/api/chat", self.base);
        let req = ChatRequest {
            model,
            messages: &messages,
            stream: true,
            think: false,
            tools: tools.as_deref(),
        };
        let resp = self
            .auth(self.http.post(&url).json(&req))
            .send()
            .await?
            .error_for_status()?;
        let byte_stream = resp.bytes_stream();

        let stream = futures::stream::unfold(
            (byte_stream, Vec::<u8>::new(), false),
            |(mut bs, mut buf, mut done)| async move {
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
                        match serde_json::from_str::<ChatChunk>(trimmed) {
                            Ok(chunk) => {
                                let msg = chunk.message.unwrap_or_default();
                                // Tool calls can arrive on done:false OR done:true — emit
                                // them whenever present, then let later iterations drain
                                // any remaining content/Done marker.
                                if !msg.tool_calls.is_empty() {
                                    if chunk.done {
                                        done = true;
                                    }
                                    return Some((
                                        Ok(StreamItem::ToolCalls(msg.tool_calls)),
                                        (bs, buf, done),
                                    ));
                                }
                                if !msg.content.is_empty() {
                                    if chunk.done {
                                        done = true;
                                    }
                                    return Some((
                                        Ok(StreamItem::Content(msg.content)),
                                        (bs, buf, done),
                                    ));
                                }
                                if chunk.done {
                                    done = true;
                                    return Some((
                                        Ok(StreamItem::Done {
                                            prompt_tokens: chunk
                                                .prompt_eval_count
                                                .unwrap_or(0),
                                            completion_tokens: chunk.eval_count.unwrap_or(0),
                                        }),
                                        (bs, buf, done),
                                    ));
                                }
                                // Empty intermediate chunk — keep looping
                                continue;
                            }
                            Err(e) => {
                                return Some((
                                    Err(anyhow!("parse error: {e}")),
                                    (bs, buf, true),
                                ));
                            }
                        }
                    }
                    match bs.next().await {
                        Some(Ok(bytes)) => buf.extend_from_slice(&bytes),
                        Some(Err(e)) => return Some((Err(anyhow!("http: {e}")), (bs, buf, true))),
                        None => {
                            let trimmed = String::from_utf8_lossy(&buf).trim().to_string();
                            if trimmed.is_empty() {
                                return None;
                            }
                            match serde_json::from_str::<ChatChunk>(&trimmed) {
                                Ok(chunk) => {
                                    let msg = chunk.message.unwrap_or_default();
                                    if !msg.tool_calls.is_empty() {
                                        return Some((
                                            Ok(StreamItem::ToolCalls(msg.tool_calls)),
                                            (bs, Vec::new(), true),
                                        ));
                                    }
                                    if !msg.content.is_empty() {
                                        return Some((
                                            Ok(StreamItem::Content(msg.content)),
                                            (bs, Vec::new(), true),
                                        ));
                                    }
                                    return Some((
                                        Ok(StreamItem::Done {
                                            prompt_tokens: chunk
                                                .prompt_eval_count
                                                .unwrap_or(0),
                                            completion_tokens: chunk.eval_count.unwrap_or(0),
                                        }),
                                        (bs, Vec::new(), true),
                                    ));
                                }
                                Err(e) => {
                                    return Some((
                                        Err(anyhow!("parse-eof: {e}")),
                                        (bs, Vec::new(), true),
                                    ));
                                }
                            }
                        }
                    }
                }
            },
        );
        Ok(Box::pin(stream))
    }
}
