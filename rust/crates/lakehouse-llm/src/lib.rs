//! `OpenAI`-compatible chat completions client, porting
//! `src/services/clients/llm.ts`.
//!
//! Defaults to `MiniMax`'s `OpenAI`-compatible endpoint, but can be
//! pointed at any other `OpenAI`-compatible node via config, without code
//! changes — the TypeScript makes the same promise via
//! `LLM_URL`/`LLM_MODEL`/`LLM_KEY`.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A single chat message, matching the TypeScript's `ChatMessage`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// The message's role.
    pub role: ChatRole,
    /// The message text.
    pub content: String,
}

/// A chat message's role, matching the TypeScript union
/// `"system" | "user" | "assistant"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    /// A system prompt.
    System,
    /// A user message.
    User,
    /// A prior assistant reply.
    Assistant,
}

/// Optional overrides for [`chat`], matching the TypeScript's
/// `{ temperature?, maxTokens?, signal? }` (there is no Rust equivalent of
/// `AbortSignal` here — cancellation, if needed, is the caller's
/// responsibility via `tokio::select!`/`CancellationToken` around the
/// future).
#[derive(Debug, Clone, Copy, Default)]
pub struct ChatOptions {
    /// Sampling temperature. Default `0.1` when `None` (`llm.ts:26`, `??`).
    pub temperature: Option<f64>,
    /// Max tokens to generate. Default `700` when `None` (`llm.ts:27`,
    /// `??`).
    pub max_tokens: Option<u32>,
}

/// Errors produced while talking to the LLM's chat-completions endpoint.
#[derive(Debug, Error)]
pub enum LlmError {
    /// A transport-level failure surfaced by `reqwest`.
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    /// The endpoint responded with a non-2xx status. The message matches
    /// the TypeScript's `` `LLM ${res.status}: ${(await res.text()).slice(0, 200)}` ``
    /// exactly, including the 200-character truncation of the response
    /// body.
    #[error("{0}")]
    Api(String),
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    temperature: f64,
    max_tokens: u32,
    stream: bool,
}

#[derive(Debug, Default, Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Debug, Default, Deserialize)]
struct ChatResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

/// `OpenAI`-compatible chat completions client.
pub struct LlmClient {
    client: Client,
    url: String,
    model: String,
    key: String,
}

impl LlmClient {
    /// Build a client. `url` is the base URL (e.g.
    /// `"https://api.minimax.io/v1"`) — `/chat/completions` is appended,
    /// matching `` `${LLM_URL}/chat/completions` ``.
    #[must_use]
    pub fn new(url: String, model: String, key: String) -> Self {
        Self {
            client: Client::new(),
            url,
            model,
            key,
        }
    }

    /// Send a chat-completions request and return the assistant's final
    /// answer text, matching `chat(messages, opts)`.
    ///
    /// A model that "thinks" out loud (e.g. `MiniMax-M2`) may put its
    /// reasoning in `reasoning_content`, or wrap it in
    /// `<think>...</think>` inside `content`; only the final answer is
    /// wanted, so `<think>...</think>` blocks are stripped and, if that
    /// leaves the content empty, `reasoning_content` is used as a
    /// fallback — matching the TypeScript verbatim.
    ///
    /// A missing or empty `choices` array is NOT an error: `json.choices?.
    /// [0]?.message` is `undefined` in that case, `?? {}` makes it an
    /// empty object, and an empty object has no `content`, so this
    /// resolves to `Ok(String::new())` — the TypeScript never throws for
    /// this case, it silently returns `""`.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::Transport`] on a network-level failure, or
    /// [`LlmError::Api`] when the endpoint responds with a non-2xx status.
    pub async fn chat(
        &self,
        messages: &[ChatMessage],
        opts: ChatOptions,
    ) -> Result<String, LlmError> {
        let body = ChatRequest {
            model: &self.model,
            messages,
            temperature: opts.temperature.unwrap_or(0.1),
            max_tokens: opts.max_tokens.unwrap_or(700),
            stream: false,
        };
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.url))
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let truncated: String = text.chars().take(200).collect();
            return Err(LlmError::Api(format!(
                "LLM {}: {truncated}",
                status.as_u16()
            )));
        }
        let parsed: ChatResponse = resp.json().await.unwrap_or_default();
        let msg = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message)
            .unwrap_or_default();

        let raw_content = msg.content.unwrap_or_default();
        let mut content = strip_think_blocks(&raw_content).trim().to_owned();
        if content.is_empty() {
            if let Some(reasoning) = msg.reasoning_content {
                content = reasoning;
            }
        }
        Ok(content)
    }
}

/// Strip every `<think>...</think>` block (case-insensitive, `.` matching
/// newlines), matching the TypeScript's
/// `content.replace(/<think>[\s\S]*?<\/think>/gi, "")`.
fn strip_think_blocks(content: &str) -> String {
    let lower = content.to_ascii_lowercase();
    let mut result = String::with_capacity(content.len());
    let mut pos = 0usize;
    loop {
        let Some(open_rel) = lower[pos..].find("<think>") else {
            result.push_str(&content[pos..]);
            break;
        };
        let open = pos + open_rel;
        let after_open = open + "<think>".len();
        let Some(close_rel) = lower[after_open..].find("</think>") else {
            result.push_str(&content[pos..]);
            break;
        };
        let close_end = after_open + close_rel + "</think>".len();
        result.push_str(&content[pos..open]);
        pos = close_end;
    }
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: ChatRole::User,
            content: content.to_owned(),
        }
    }

    #[tokio::test]
    async fn sends_expected_request_body_and_defaults() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer sekret"))
            .and(body_json(json!({
                "model": "MiniMax-M3",
                "messages": [ { "role": "user", "content": "hi" } ],
                "temperature": 0.1,
                "max_tokens": 700,
                "stream": false,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [ { "message": { "content": "hello there" } } ]
            })))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "MiniMax-M3".to_owned(), "sekret".to_owned());
        let out = client
            .chat(&[user_msg("hi")], ChatOptions::default())
            .await
            .unwrap();
        assert_eq!(out, "hello there");
    }

    #[tokio::test]
    async fn honors_explicit_temperature_and_max_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_json(json!({
                "model": "m",
                "messages": [ { "role": "user", "content": "hi" } ],
                "temperature": 0.7,
                "max_tokens": 42,
                "stream": false,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [ { "message": { "content": "ok" } } ]
            })))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let opts = ChatOptions {
            temperature: Some(0.7),
            max_tokens: Some(42),
        };
        let out = client.chat(&[user_msg("hi")], opts).await.unwrap();
        assert_eq!(out, "ok");
    }

    #[tokio::test]
    async fn missing_choices_array_returns_empty_string_not_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let out = client
            .chat(&[user_msg("hi")], ChatOptions::default())
            .await
            .unwrap();
        assert_eq!(out, "");
    }

    #[tokio::test]
    async fn empty_choices_array_returns_empty_string_not_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "choices": [] })))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let out = client
            .chat(&[user_msg("hi")], ChatOptions::default())
            .await
            .unwrap();
        assert_eq!(out, "");
    }

    #[tokio::test]
    async fn strips_think_blocks_and_trims() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [ { "message": { "content": "<think>pondering...</think>  final answer  " } } ]
            })))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let out = client
            .chat(&[user_msg("hi")], ChatOptions::default())
            .await
            .unwrap();
        assert_eq!(out, "final answer");
    }

    #[tokio::test]
    async fn falls_back_to_reasoning_content_when_stripped_content_is_empty() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [ { "message": {
                    "content": "<think>only thoughts</think>",
                    "reasoning_content": "the reasoning trail"
                } } ]
            })))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let out = client
            .chat(&[user_msg("hi")], ChatOptions::default())
            .await
            .unwrap();
        assert_eq!(out, "the reasoning trail");
    }

    #[tokio::test]
    async fn surfaces_api_errors_with_status_and_truncated_body() {
        let server = MockServer::start().await;
        let long_body = "e".repeat(500);
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(long_body.clone()))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let err = client
            .chat(&[user_msg("hi")], ChatOptions::default())
            .await
            .unwrap_err();
        let LlmError::Api(msg) = err else {
            panic!("expected Api error");
        };
        assert_eq!(msg, format!("LLM 500: {}", &long_body[..200]));
    }
}
