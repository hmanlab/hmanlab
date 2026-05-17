use anyhow::{bail, Result};
use serde::{Deserialize, Deserializer, Serialize};
use tokio::sync::mpsc;

/// A session as returned by the hmanlab-api backend.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
}

/// Message rows have a BIGSERIAL `id`. postgres.js stringifies bigints,
/// so the wire format is "id":"42". Accept either a string or a number.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Message {
    #[serde(deserialize_with = "de_str_or_i64")]
    pub id: i64,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_calls: Option<serde_json::Value>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

fn de_str_or_i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        N(i64),
        S(String),
    }
    match Either::deserialize(d)? {
        Either::N(n) => Ok(n),
        Either::S(s) => s.parse().map_err(serde::de::Error::custom),
    }
}

#[derive(Deserialize)]
struct SessionEnvelope {
    session: Session,
}

#[derive(Deserialize)]
struct SessionsEnvelope {
    sessions: Vec<Session>,
}

#[derive(Deserialize)]
struct MessagesEnvelope {
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct MessageEnvelope {
    message: Message,
}

/// The account-info payload returned by `GET /v1/auth/me`. The same shape
/// is accepted whether the bearer is an API key (`bai_*`) or a web
/// session token, so the TUI uses this to introspect "who am I" given
/// only its stored API key — handy for `/settings`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Me {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub email: String,
    #[serde(default)]
    pub training_opt_in: bool,
    #[serde(default)]
    pub is_admin: bool,
}

#[derive(Deserialize)]
struct MeEnvelope {
    user: Me,
}

/// Thin async wrapper around the hmanlab-api HTTP API.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    api_key: String,
}

impl Client {
    pub fn new(base: String, api_key: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest client");
        let base = base.trim_end_matches('/').to_string();
        Self {
            http,
            base,
            api_key,
        }
    }

    /// Verify the key works. Used at startup so we can show a useful status.
    pub async fn check_auth(&self) -> Result<()> {
        let url = format!("{}/v1/auth/me", self.base);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("auth check failed ({})", resp.status());
        }
        Ok(())
    }

    /// Fetch the authenticated user's profile via `GET /v1/auth/me`.
    /// Differs from `check_auth` only in that it returns the parsed body
    /// — the TUI uses it for `/settings`. Errors bubble up verbatim so
    /// the caller can show the user a real diagnostic.
    pub async fn fetch_me(&self) -> Result<Me> {
        let url = format!("{}/v1/auth/me", self.base);
        let env: MeEnvelope = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(env.user)
    }

    pub async fn create_session(&self, model: &str) -> Result<Session> {
        let url = format!("{}/v1/sessions", self.base);
        let env: SessionEnvelope = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "model": model }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(env.session)
    }

    pub async fn list_sessions(&self, limit: i64) -> Result<Vec<Session>> {
        let url = format!("{}/v1/sessions?limit={}", self.base, limit);
        let env: SessionsEnvelope = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(env.sessions)
    }

    /// Find a session by an 8+ hex char UUID prefix (dashes ignored on input).
    /// Pulls the last 50 sessions and filters client-side.
    pub async fn find_session_by_prefix(&self, prefix: &str) -> Result<Session> {
        let clean: String = prefix
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect::<String>()
            .to_lowercase();
        if clean.len() < 4 {
            bail!("prefix too short (need ≥4 hex chars)");
        }
        let candidates = self.list_sessions(50).await?;
        let mut matches: Vec<Session> = candidates
            .into_iter()
            .filter(|s| s.id.replace('-', "").to_lowercase().starts_with(&clean))
            .collect();
        match matches.len() {
            0 => bail!("no session matches '{prefix}'"),
            1 => Ok(matches.remove(0)),
            n => bail!("{n} sessions match '{prefix}' — use a longer prefix"),
        }
    }

    pub async fn load_recent_messages(&self, session_id: &str, limit: i64) -> Result<Vec<Message>> {
        let url = format!(
            "{}/v1/sessions/{}/messages?limit={}",
            self.base, session_id, limit
        );
        let env: MessagesEnvelope = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(env.messages)
    }

    pub async fn load_older_messages(
        &self,
        session_id: &str,
        before_id: i64,
        limit: i64,
    ) -> Result<Vec<Message>> {
        let url = format!(
            "{}/v1/sessions/{}/messages?limit={}&before={}",
            self.base, session_id, limit, before_id
        );
        let env: MessagesEnvelope = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(env.messages)
    }

    pub async fn post_message(&self, session_id: &str, body: serde_json::Value) -> Result<Message> {
        let url = format!("{}/v1/sessions/{}/messages", self.base, session_id);
        let env: MessageEnvelope = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(env.message)
    }
}

/// Writer-task ops. Same role as the old DbOp — keeps writes serialized and
/// lazily creates a session on first user message. The shape mirrors what we
/// want to land in the DB so future fine-tune exports get the full agent loop:
/// user text, assistant text (with tool_calls when applicable), and tool
/// results with the function name.
pub enum ApiOp {
    UserMessage {
        content: String,
        model: String,
    },
    /// Final assistant reply for a turn (no tool_calls). Persisted at end of
    /// the agent loop after streaming completes.
    AssistantMessage {
        content: String,
        model: String,
    },
    /// Intermediate assistant turn — emitted tool_calls and is about to
    /// hand off to tools. Tool calls are stored verbatim.
    AssistantToolCalls {
        content: String,
        tool_calls: serde_json::Value,
        model: String,
    },
    /// Output of a tool invocation. `name` is the function name.
    ToolResult {
        name: String,
        output: String,
    },
    EndSession,
    SetSession(String),
}

pub async fn run_writer(client: Client, mut rx: mpsc::UnboundedReceiver<ApiOp>) {
    let mut session_id: Option<String> = None;
    while let Some(op) = rx.recv().await {
        match op {
            ApiOp::UserMessage { content, model } => {
                let sid = match session_id.clone() {
                    Some(s) => s,
                    None => match client.create_session(&model).await {
                        Ok(s) => {
                            session_id = Some(s.id.clone());
                            s.id
                        }
                        Err(_) => {
                            // Persistence failure — drop silently. The TUI is in
                            // alt-screen mode, so eprintln! here would scramble
                            // the rendered frame.
                            continue;
                        }
                    },
                };
                let _ = client
                    .post_message(
                        &sid,
                        serde_json::json!({
                            "role": "user",
                            "content": content,
                            "model": model,
                        }),
                    )
                    .await;
            }
            ApiOp::AssistantMessage { content, model } => {
                let Some(sid) = session_id.clone() else {
                    continue;
                };
                let _ = client
                    .post_message(
                        &sid,
                        serde_json::json!({
                            "role": "assistant",
                            "content": content,
                            "model": model,
                        }),
                    )
                    .await;
            }
            ApiOp::AssistantToolCalls {
                content,
                tool_calls,
                model,
            } => {
                let Some(sid) = session_id.clone() else {
                    continue;
                };
                let _ = client
                    .post_message(
                        &sid,
                        serde_json::json!({
                            "role": "assistant",
                            "content": content,
                            "tool_calls": tool_calls,
                            "model": model,
                        }),
                    )
                    .await;
            }
            ApiOp::ToolResult { name, output } => {
                let Some(sid) = session_id.clone() else {
                    continue;
                };
                let _ = client
                    .post_message(
                        &sid,
                        serde_json::json!({
                            "role": "tool",
                            "content": output,
                            "name": name,
                        }),
                    )
                    .await;
            }
            ApiOp::EndSession => session_id = None,
            ApiOp::SetSession(id) => session_id = Some(id),
        }
    }
}
