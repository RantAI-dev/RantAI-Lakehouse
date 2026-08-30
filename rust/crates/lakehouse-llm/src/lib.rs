//! `OpenAI`-compatible chat completions client, porting
//! `src/services/clients/llm.ts`.
//!
//! Defaults to `MiniMax`'s `OpenAI`-compatible endpoint, but can be
//! pointed at any other `OpenAI`-compatible node via config, without code
//! changes — the TypeScript makes the same promise via
//! `LLM_URL`/`LLM_MODEL`/`LLM_KEY`.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
        if content.is_empty()
            && let Some(reasoning) = msg.reasoning_content
        {
            content = reasoning;
        }
        Ok(content)
    }
}

/// One tool call the model wants executed, matching the TypeScript's
/// `ToolCall` (`llm.ts`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// The call's id (echoed back in the follow-up `tool` message).
    pub id: String,
    /// Always `"function"` for an `OpenAI`-compatible tool call.
    #[serde(rename = "type")]
    pub kind: String,
    /// The function invocation itself.
    pub function: ToolCallFunction,
}

/// A tool call's function name and (`JSON`-encoded) arguments string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    /// The tool's name, matching one of the `tools` schemas passed to
    /// [`LlmClient::chat_with_tools`].
    pub name: String,
    /// The call's arguments, as a raw `JSON` string (not yet parsed) —
    /// matching the `OpenAI` wire format exactly.
    pub arguments: String,
}

/// A chat message in the tool-calling loop, matching the TypeScript's
/// `LlmMessage` (`llm.ts`). Distinct from [`ChatMessage`]: `content` is
/// optional (a tool-calling assistant turn may carry only `tool_calls`),
/// and a `"tool"` role plus `tool_call_id`/`name` are added for feeding
/// tool results back to the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    /// The message's role, including `"tool"` (absent from [`ChatRole`]).
    pub role: LlmMessageRole,
    /// The message text. `None` for an assistant turn that only calls
    /// tools.
    pub content: Option<String>,
    /// Tool calls the assistant wants executed, present only on some
    /// assistant turns.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The [`ToolCall::id`] this message answers, present only on `"tool"`
    /// role messages.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_call_id: Option<String>,
    /// The tool's name, present only on `"tool"` role messages.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
}

/// [`LlmMessage`]'s role, matching the TypeScript union `"system" | "user" |
/// "assistant" | "tool"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmMessageRole {
    /// A system prompt.
    System,
    /// A user message.
    User,
    /// A prior (or current) assistant turn.
    Assistant,
    /// A tool result fed back to the model.
    Tool,
}

#[derive(Debug, Serialize)]
struct ChatWithToolsRequest<'a> {
    model: &'a str,
    messages: &'a [LlmMessage],
    tools: &'a [Value],
    tool_choice: &'a str,
    temperature: f64,
    max_tokens: u32,
    stream: bool,
}

impl LlmClient {
    /// Chat with function-calling enabled, matching `chatWithTools` in
    /// `llm.ts`. Returns the assistant's raw reply message — which may
    /// carry `tool_calls` instead of (or in addition to) `content` — for
    /// the caller's agentic loop to execute and feed back.
    ///
    /// Defaults differ from [`LlmClient::chat`]: `temperature` defaults to
    /// `0.2` (not `0.1`) and `max_tokens` to `1200` (not `700`), matching
    /// `chatWithTools`'s own `??` fallbacks.
    ///
    /// Any `<think>...</think>` block in the reply's `content` is stripped
    /// the same way [`LlmClient::chat`] strips it — but unlike `chat`,
    /// there is no `reasoning_content` fallback when that leaves `content`
    /// empty, matching `chatWithTools` verbatim (it only trims
    /// `msg.content`, never reads `reasoning_content`).
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::Transport`] on a network-level failure, or
    /// [`LlmError::Api`] when the endpoint responds with a non-2xx status.
    pub async fn chat_with_tools(
        &self,
        messages: &[LlmMessage],
        tools: &[Value],
        opts: ChatOptions,
    ) -> Result<LlmMessage, LlmError> {
        let body = ChatWithToolsRequest {
            model: &self.model,
            messages,
            tools,
            tool_choice: "auto",
            temperature: opts.temperature.unwrap_or(0.2),
            max_tokens: opts.max_tokens.unwrap_or(1200),
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
        let parsed: Value = resp.json().await.unwrap_or_default();
        let msg_value = parsed
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!({ "role": "assistant", "content": "" }));
        let mut msg: LlmMessage = serde_json::from_value(msg_value).unwrap_or(LlmMessage {
            role: LlmMessageRole::Assistant,
            content: Some(String::new()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
        if let Some(content) = &msg.content {
            let stripped = strip_think_blocks(content).trim().to_owned();
            msg.content = Some(stripped);
        }
        Ok(msg)
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

    #[tokio::test]
    async fn chat_with_tools_returns_tool_calls_when_present() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [ { "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        { "id": "call_1", "type": "function",
                          "function": { "name": "run_sql", "arguments": "{\"sql\":\"SELECT 1\"}" } }
                    ]
                } } ]
            })))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let msg = client
            .chat_with_tools(
                &[LlmMessage {
                    role: LlmMessageRole::User,
                    content: Some("hi".to_owned()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                }],
                &[],
                ChatOptions::default(),
            )
            .await
            .unwrap();
        assert!(msg.content.is_none());
        let calls = msg.tool_calls.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "run_sql");
    }

    #[tokio::test]
    async fn chat_with_tools_strips_think_blocks_from_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [ { "message": {
                    "role": "assistant",
                    "content": "<think>hmm</think>final answer"
                } } ]
            })))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let msg = client
            .chat_with_tools(&[], &[], ChatOptions::default())
            .await
            .unwrap();
        assert_eq!(msg.content.as_deref(), Some("final answer"));
    }

    #[tokio::test]
    async fn chat_with_tools_defaults_temperature_and_max_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_json(json!({
                "model": "m",
                "messages": [],
                "tools": [],
                "tool_choice": "auto",
                "temperature": 0.2,
                "max_tokens": 1200,
                "stream": false,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [ { "message": { "role": "assistant", "content": "ok" } } ]
            })))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        client
            .chat_with_tools(&[], &[], ChatOptions::default())
            .await
            .unwrap();
    }
}
