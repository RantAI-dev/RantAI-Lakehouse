//! Streamed tool-calling replies (`stream: true`).
//!
//! [`LlmClient::chat_with_tools_streamed`] asks the endpoint for
//! server-sent events and hands each piece of *visible* answer text to a
//! callback as it arrives, so the console can show the answer while it is
//! being written. What it returns is the same [`LlmMessage`] the
//! non-streamed call returns (content with `<think>` blocks stripped and
//! trimmed, tool calls reassembled from their fragments): the agentic loop
//! behind it does not change, only how early the user sees the words.
//!
//! An endpoint that ignores `stream: true` and answers with plain JSON is
//! handled too: the whole content is handed to the callback at once.

use reqwest::header::CONTENT_TYPE;
use serde_json::Value;

use crate::{
    ChatOptions, ChatWithToolsRequest, LlmClient, LlmError, LlmMessage, LlmMessageRole, ToolCall,
    ToolCallFunction, error_for_status, reply_from_json, strip_think_blocks,
};

/// Tool calls arrive in fragments keyed by `index`; more than this many
/// distinct indexes in one reply is treated as garbage and ignored rather
/// than allocated.
const MAX_TOOL_CALLS: usize = 64;

/// Hides every span between an opening and a closing tag from text that
/// arrives in pieces, releasing only what is certainly outside a span.
///
/// Matching is ASCII-case-insensitive (`<Think>` is hidden too). A piece
/// ending in something that could be the start of an opening tag (`<thi`)
/// is held back until the next piece settles it, so a tag split across
/// chunks never flashes on screen. An unclosed span hides everything after
/// its opening tag, the same as the non-streamed strip leaving no visible
/// answer text behind it.
#[derive(Debug)]
pub struct HiddenSpans {
    /// `(open, close)` tag pairs, lowercased.
    pairs: Vec<(String, String)>,
    /// Text received but not yet released or discarded.
    buf: String,
    /// Index into `pairs` of the span currently open, if any.
    inside: Option<usize>,
}

impl HiddenSpans {
    /// A filter hiding each `(open, close)` span. Tags must be ASCII.
    #[must_use]
    pub fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            pairs: pairs
                .iter()
                .map(|(o, c)| (o.to_ascii_lowercase(), c.to_ascii_lowercase()))
                .collect(),
            buf: String::new(),
            inside: None,
        }
    }

    /// Feeds the next piece of text and returns what can be shown now.
    pub fn push(&mut self, text: &str) -> String {
        self.buf.push_str(text);
        let mut out = String::new();
        loop {
            let lower = self.buf.to_ascii_lowercase();
            if let Some(i) = self.inside {
                let close = &self.pairs[i].1;
                if let Some(at) = lower.find(close.as_str()) {
                    self.buf.drain(..at + close.len());
                    self.inside = None;
                    continue;
                }
                // Keep only what could be the start of the closing tag.
                let mut cut = self.buf.len().saturating_sub(close.len() - 1);
                while !self.buf.is_char_boundary(cut) {
                    cut -= 1;
                }
                self.buf.drain(..cut);
                return out;
            }
            // The earliest tag of either kind. A closing tag with no opening
            // one before it is dropped on its own: `MiniMax` streams its
            // reasoning in `reasoning_content` yet still sends a bare
            // `</think>` in `content`.
            let first_tag = self
                .pairs
                .iter()
                .enumerate()
                .flat_map(|(i, (open, close))| {
                    [
                        lower.find(open.as_str()).map(|at| (at, i, false)),
                        lower.find(close.as_str()).map(|at| (at, i, true)),
                    ]
                })
                .flatten()
                .min();
            if let Some((at, i, stray_close)) = first_tag {
                out.push_str(&self.buf[..at]);
                let (open, close) = &self.pairs[i];
                let tag_len = if stray_close { close.len() } else { open.len() };
                self.buf.drain(..at + tag_len);
                if !stray_close {
                    self.inside = Some(i);
                }
                continue;
            }
            // Hold back a tail that could still grow into a tag. Tags are
            // ASCII, so a matching tail starts on a char boundary.
            let bytes = lower.as_bytes();
            let hold = (1..=bytes.len().min(self.longest_tag()))
                .rev()
                .find(|&k| {
                    let tail = &bytes[bytes.len() - k..];
                    self.pairs.iter().any(|(open, close)| {
                        [open, close]
                            .iter()
                            .any(|tag| tag.len() > k && tag.as_bytes().starts_with(tail))
                    })
                })
                .unwrap_or(0);
            let cut = self.buf.len() - hold;
            out.push_str(&self.buf[..cut]);
            self.buf.drain(..cut);
            return out;
        }
    }

    fn longest_tag(&self) -> usize {
        self.pairs
            .iter()
            .map(|(o, c)| o.len().max(c.len()))
            .max()
            .unwrap_or(0)
    }
}

/// One tool call being reassembled from its streamed fragments.
#[derive(Debug, Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// Folds one `delta.tool_calls[]` fragment into `calls`: `id` is set once,
/// `name` and `arguments` are appended (some endpoints split the name too).
fn merge_call(calls: &mut Vec<PartialCall>, part: &Value) {
    let index = part
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|i| usize::try_from(i).ok())
        .unwrap_or(calls.len());
    if index >= MAX_TOOL_CALLS {
        return;
    }
    if calls.len() <= index {
        calls.resize_with(index + 1, PartialCall::default);
    }
    let call = &mut calls[index];
    if let Some(id) = part.get("id").and_then(Value::as_str)
        && !id.is_empty()
    {
        id.clone_into(&mut call.id);
    }
    if let Some(name) = part.pointer("/function/name").and_then(Value::as_str) {
        call.name.push_str(name);
    }
    if let Some(args) = part.pointer("/function/arguments").and_then(Value::as_str) {
        call.arguments.push_str(args);
    }
}

/// Reads the SSE body to its end (or `[DONE]`, or until `on_text` says
/// stop), handing visible text over as it comes. Returns the raw content
/// and the reassembled tool-call fragments.
async fn read_events<F>(
    resp: &mut reqwest::Response,
    on_text: &mut F,
) -> Result<(String, Vec<PartialCall>), LlmError>
where
    F: FnMut(&str) -> bool,
{
    let mut think = HiddenSpans::new(&[("<think>", "</think>")]);
    let mut content = String::new();
    let mut calls: Vec<PartialCall> = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    'read: while let Some(chunk) = resp.chunk().await? {
        pending.extend_from_slice(&chunk);
        // SSE lines end in `\n`; a line split across chunks waits here.
        while let Some(nl) = pending.iter().position(|b| *b == b'\n') {
            let raw: Vec<u8> = pending.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&raw);
            let Some(data) = line.trim().strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                break 'read;
            }
            let Ok(event) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            if let Some(err) = event.get("error") {
                let text: String = err.to_string().chars().take(200).collect();
                return Err(LlmError::Api(format!("LLM stream: {text}")));
            }
            let Some(delta) = event.pointer("/choices/0/delta") else {
                continue;
            };
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                content.push_str(text);
                let visible = think.push(text);
                if !visible.is_empty() && !on_text(&visible) {
                    break 'read;
                }
            }
            if let Some(parts) = delta.get("tool_calls").and_then(Value::as_array) {
                for part in parts {
                    merge_call(&mut calls, part);
                }
            }
        }
    }

    Ok((content, calls))
}

impl LlmClient {
    /// [`LlmClient::chat_with_tools`] with `stream: true`: `on_text` gets
    /// each piece of visible answer text (never `<think>` content) as it
    /// arrives, and returns `false` to stop reading — the caller's client
    /// has gone. The returned message is built exactly like the
    /// non-streamed one, from everything received.
    ///
    /// Text streamed during a round that ends in tool calls is the model's
    /// preamble, not the answer; the caller decides what to do with it
    /// once it sees the returned `tool_calls`.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::Transport`] on a network-level failure,
    /// [`LlmError::Api`] when the endpoint responds with a non-2xx status
    /// or sends an `error` event mid-stream.
    pub async fn chat_with_tools_streamed<F>(
        &self,
        messages: &[LlmMessage],
        tools: &[Value],
        opts: ChatOptions,
        mut on_text: F,
    ) -> Result<LlmMessage, LlmError>
    where
        F: FnMut(&str) -> bool,
    {
        let body = ChatWithToolsRequest {
            model: &self.model,
            messages,
            tools,
            tool_choice: "auto",
            temperature: opts.temperature.unwrap_or(0.2),
            max_tokens: opts.max_tokens.unwrap_or(1200),
            stream: true,
        };
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.url))
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .await?;
        let mut resp = error_for_status(resp).await?;

        let is_sse = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        if !is_sse {
            let parsed: Value = resp.json().await.unwrap_or_default();
            let msg = reply_from_json(&parsed);
            if let Some(content) = msg.content.as_deref().filter(|c| !c.is_empty()) {
                on_text(content);
            }
            return Ok(msg);
        }

        let (content, calls) = read_events(&mut resp, &mut on_text).await?;
        let tool_calls: Vec<ToolCall> = calls
            .into_iter()
            .enumerate()
            .filter(|(_, c)| !c.name.is_empty())
            .map(|(i, c)| ToolCall {
                id: if c.id.is_empty() {
                    format!("call_{i}")
                } else {
                    c.id
                },
                kind: "function".to_owned(),
                function: ToolCallFunction {
                    name: c.name,
                    arguments: c.arguments,
                },
            })
            .collect();
        let stripped = strip_think_blocks(&content).trim().to_owned();
        Ok(LlmMessage {
            role: LlmMessageRole::Assistant,
            // A tool-only turn carries no content, as in the JSON reply.
            content: if stripped.is_empty() && !tool_calls.is_empty() {
                None
            } else {
                Some(stripped)
            },
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: None,
            name: None,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn user(text: &str) -> LlmMessage {
        LlmMessage {
            role: LlmMessageRole::User,
            content: Some(text.to_owned()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    fn sse(events: &[Value]) -> String {
        use std::fmt::Write as _;
        let mut body = String::new();
        for e in events {
            let _ = write!(body, "data: {e}\n\n");
        }
        body.push_str("data: [DONE]\n\n");
        body
    }

    fn content_delta(text: &str) -> Value {
        json!({ "choices": [ { "delta": { "content": text } } ] })
    }

    #[test]
    fn hidden_spans_hides_a_tag_split_across_pieces() {
        let mut f = HiddenSpans::new(&[("<think>", "</think>")]);
        let mut shown = String::new();
        for piece in ["Hi <th", "ink>secret", " plan</thi", "nk> there"] {
            shown.push_str(&f.push(piece));
        }
        assert_eq!(shown, "Hi  there");
    }

    #[test]
    fn hidden_spans_is_case_insensitive_and_keeps_non_ascii_text() {
        let mut f = HiddenSpans::new(&[("<think>", "</think>")]);
        let shown = f.push("Tarif naik 4% — <THINK>x</Think>ok ✓");
        assert_eq!(shown, "Tarif naik 4% — ok ✓");
    }

    #[test]
    fn hidden_spans_releases_a_lone_angle_bracket_once_it_cannot_be_a_tag() {
        let mut f = HiddenSpans::new(&[("<think>", "</think>")]);
        assert_eq!(f.push("a <"), "a ");
        assert_eq!(f.push("b"), "<b");
    }

    #[test]
    fn hidden_spans_drops_a_closing_tag_that_has_no_opening_tag() {
        let mut f = HiddenSpans::new(&[("<think>", "</think>")]);
        let mut shown = f.push("\n\n</thi");
        shown.push_str(&f.push("nk>Hello"));
        assert_eq!(shown, "\n\nHello");
    }

    #[test]
    fn hidden_spans_hides_several_kinds_of_span() {
        let mut f = HiddenSpans::new(&[
            ("<minimax:tool_call>", "</minimax:tool_call>"),
            ("<invoke ", "</invoke>"),
        ]);
        let shown = f.push(r#"Checking.<invoke name="run_sql"><parameter name="sql">SELECT 1</parameter></invoke> Done."#);
        assert_eq!(shown, "Checking. Done.");
    }

    #[tokio::test]
    async fn streamed_reply_hands_over_visible_text_and_returns_the_same_message() {
        let server = MockServer::start().await;
        let body = sse(&[
            content_delta("<think>plan"),
            content_delta("</think>Hello"),
            content_delta(", **world**"),
        ]);
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(json!({ "stream": true })))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let mut seen = String::new();
        let msg = client
            .chat_with_tools_streamed(&[user("hi")], &[], ChatOptions::default(), |t| {
                seen.push_str(t);
                true
            })
            .await
            .unwrap();
        assert_eq!(seen, "Hello, **world**");
        assert_eq!(msg.content.as_deref(), Some("Hello, **world**"));
        assert!(msg.tool_calls.is_none());
    }

    #[tokio::test]
    async fn streamed_tool_call_fragments_are_reassembled() {
        let server = MockServer::start().await;
        let body = sse(&[
            json!({ "choices": [ { "delta": { "tool_calls": [
                { "index": 0, "id": "call_a", "type": "function",
                  "function": { "name": "run_sql", "arguments": "{\"sql\":" } } ] } } ] }),
            json!({ "choices": [ { "delta": { "tool_calls": [
                { "index": 0, "function": { "arguments": "\"SELECT 1\"}" } } ] } } ] }),
        ]);
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let msg = client
            .chat_with_tools_streamed(&[user("hi")], &[], ChatOptions::default(), |_| true)
            .await
            .unwrap();
        assert!(msg.content.is_none());
        let calls = msg.tool_calls.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_a");
        assert_eq!(calls[0].function.name, "run_sql");
        assert_eq!(calls[0].function.arguments, "{\"sql\":\"SELECT 1\"}");
    }

    #[tokio::test]
    async fn a_json_answer_to_a_stream_request_is_handed_over_whole() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [ { "message": { "role": "assistant", "content": "<think>x</think> Whole answer" } } ]
            })))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let mut seen = Vec::new();
        let msg = client
            .chat_with_tools_streamed(&[user("hi")], &[], ChatOptions::default(), |t| {
                seen.push(t.to_owned());
                true
            })
            .await
            .unwrap();
        assert_eq!(seen, vec!["Whole answer".to_owned()]);
        assert_eq!(msg.content.as_deref(), Some("Whole answer"));
    }

    #[tokio::test]
    async fn an_error_event_mid_stream_is_an_api_error() {
        let server = MockServer::start().await;
        let body = sse(&[
            content_delta("Hel"),
            json!({ "error": { "message": "overloaded" } }),
        ]);
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let err = client
            .chat_with_tools_streamed(&[user("hi")], &[], ChatOptions::default(), |_| true)
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::Api(m) if m.contains("overloaded")));
    }

    #[tokio::test]
    async fn returning_false_from_the_callback_stops_reading() {
        let server = MockServer::start().await;
        let body = sse(&[
            content_delta("one "),
            content_delta("two "),
            content_delta("three"),
        ]);
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;

        let client = LlmClient::new(server.uri(), "m".to_owned(), "k".to_owned());
        let mut calls = 0;
        let msg = client
            .chat_with_tools_streamed(&[user("hi")], &[], ChatOptions::default(), |_| {
                calls += 1;
                false
            })
            .await
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(msg.content.as_deref(), Some("one"));
    }
}
