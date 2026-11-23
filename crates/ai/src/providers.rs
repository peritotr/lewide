//! The providers and the one request they all serve: a streamed chat reply.

use std::time::Duration;

use dbclient_core::{Error, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::ai_error;
use crate::sse::{SseEvent, SseParser};

const ANTHROPIC_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2026-10-21";
/// The model used for Anthropic when the user has not chosen one.
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-opus-5-5";
/// Room for the reply. A ceiling, not a target: only what is generated is billed.
const ANTHROPIC_MAX_TOKENS: u32 = 64_000;
/// Models on which a request declined by a safety classifier can be re-run
/// on another model by the API itself.
const ANTHROPIC_SERVER_FALLBACK: [&str; 5] = [
    "claude-fable-5-2",
    "claude-fable-5-1",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-sonnet-5-5",
];
const OPENAI_URL: &str = "https://api.openai.com/v1";
const GEMINI_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Gemini,
    /// Any service that speaks the OpenAI chat completions protocol at its own
    /// address: Ollama, OpenRouter, Groq, Mistral, a company gateway.
    OpenAiCompatible,
}

/// A provider as the user set it up. The API key is stored separately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub name: String,
    /// Required for `OpenAiCompatible`; overrides the standard address otherwise.
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

impl ProviderConfig {
    fn base(&self) -> Result<String> {
        let standard = match self.kind {
            ProviderKind::Anthropic => Some(ANTHROPIC_URL),
            ProviderKind::OpenAi => Some(OPENAI_URL),
            ProviderKind::Gemini => Some(GEMINI_URL),
            ProviderKind::OpenAiCompatible => None,
        };
        self.base_url
            .as_deref()
            .filter(|u| !u.trim().is_empty())
            .or(standard)
            .map(|u| u.trim().trim_end_matches('/').to_owned())
            .ok_or_else(|| {
                ai_error(
                    "not_configured",
                    format!("{} has no address set", self.name),
                )
            })
    }

    fn model(&self) -> Result<String> {
        self.model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| {
                (self.kind == ProviderKind::Anthropic).then(|| ANTHROPIC_DEFAULT_MODEL.to_owned())
            })
            .ok_or_else(|| {
                ai_error(
                    "not_configured",
                    format!("choose a model for {}", self.name),
                )
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub system: String,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model finished its reply.
    End,
    /// The reply was cut off at the output limit.
    Length,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// The next piece of the reply.
    Text { text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Completion {
    pub text: String,
    pub model: String,
    pub stop: StopReason,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

/// The HTTP client for provider requests. Build one and reuse it.
pub fn http_client() -> Result<reqwest::Client> {
    // rustls needs a crypto provider chosen once per process; an earlier
    // choice by another part of the application is kept.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // No overall timeout: a long reply streams for as long as it needs.
        .read_timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| Error::Internal(format!("HTTP client: {e}")))
}

/// Sends the conversation and streams the reply through `on_event`. Returns
/// the whole reply at the end. Stops early, with a `cancelled` error, when
/// `cancel` is triggered.
pub async fn stream_chat(
    http: &reqwest::Client,
    provider: &ProviderConfig,
    api_key: Option<&str>,
    request: &ChatRequest,
    cancel: &CancellationToken,
    mut on_event: impl FnMut(StreamEvent) + Send,
) -> Result<Completion> {
    let base = provider.base()?;
    let model = provider.model()?;
    let key = api_key.filter(|k| !k.is_empty());
    if key.is_none() && provider.kind != ProviderKind::OpenAiCompatible {
        return Err(ai_error(
            "not_configured",
            format!("{} has no API key", provider.name),
        ));
    }

    let builder = match provider.kind {
        ProviderKind::Anthropic => anthropic_request(http, &base, &model, key, request),
        ProviderKind::Gemini => gemini_request(http, &base, &model, key, request),
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
            openai_request(http, &base, &model, key, request, provider.kind)
        }
    };

    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(ai_error("cancelled", "stopped")),
        sent = builder.send() => sent.map_err(network_error)?,
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(status_error(status.as_u16(), &body, &provider.name, &model));
    }

    let mut completion = Completion {
        text: String::new(),
        model,
        stop: StopReason::Other,
        usage: Usage::default(),
    };
    let mut parser = SseParser::default();
    let mut body = response.bytes_stream();
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err(ai_error("cancelled", "stopped")),
            next = body.next() => next,
        };
        let Some(chunk) = chunk else { break };
        for event in parser.push(&chunk.map_err(network_error)?) {
            let text = match provider.kind {
                ProviderKind::Anthropic => anthropic_event(&event, &mut completion)?,
                ProviderKind::Gemini => gemini_event(&event, &mut completion)?,
                _ => openai_event(&event, &mut completion)?,
            };
            if let Some(text) = text.filter(|t| !t.is_empty()) {
                completion.text.push_str(&text);
                on_event(StreamEvent::Text { text });
            }
        }
    }
    Ok(completion)
}

/// The models the key can use, for the model picker.
pub async fn list_models(
    http: &reqwest::Client,
    provider: &ProviderConfig,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>> {
    let base = provider.base()?;
    let key = api_key.unwrap_or_default();
    let request = match provider.kind {
        ProviderKind::Anthropic => http
            .get(format!("{base}/v1/models?limit=1000"))
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION),
        ProviderKind::Gemini => http
            .get(format!("{base}/models?pageSize=1000"))
            .header("x-goog-api-key", key),
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
            let request = http.get(format!("{base}/models"));
            if key.is_empty() {
                request
            } else {
                request.bearer_auth(key)
            }
        }
    };
    let response = request.send().await.map_err(network_error)?;
    let status = response.status();
    let body = response.text().await.map_err(network_error)?;
    if !status.is_success() {
        return Err(status_error(status.as_u16(), &body, &provider.name, ""));
    }
    let json: Value = serde_json::from_str(&body).map_err(|e| {
        ai_error(
            "server",
            format!("unexpected reply from {}: {e}", provider.name),
        )
    })?;

    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    let mut models: Vec<ModelInfo> = match provider.kind {
        ProviderKind::Gemini => json["models"]
            .as_array()
            .into_iter()
            .flatten()
            // Only models that can answer a chat request.
            .filter(|m| {
                m["supportedGenerationMethods"]
                    .as_array()
                    .is_some_and(|methods| methods.iter().any(|x| x == "generateContent"))
            })
            .filter_map(|m| {
                let id = text(m, "name")?.trim_start_matches("models/").to_owned();
                Some(ModelInfo {
                    name: text(m, "displayName").unwrap_or_else(|| id.clone()),
                    id,
                })
            })
            .collect(),
        _ => json["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                let id = text(m, "id")?;
                Some(ModelInfo {
                    name: text(m, "display_name").unwrap_or_else(|| id.clone()),
                    id,
                })
            })
            .collect(),
    };
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

// ---- Anthropic: the Messages API ----

fn anthropic_request(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    key: Option<&str>,
    request: &ChatRequest,
) -> reqwest::RequestBuilder {
    let mut body = json!({
        "model": model,
        "max_tokens": ANTHROPIC_MAX_TOKENS,
        "stream": true,
        "system": request.system,
        "messages": request.messages.iter().map(|m| json!({
            "role": if m.role == Role::User { "user" } else { "assistant" },
            "content": m.content,
        })).collect::<Vec<_>>(),
    });
    let mut builder = http
        .post(format!("{base}/v1/messages"))
        .header("x-api-key", key.unwrap_or_default())
        .header("anthropic-version", ANTHROPIC_VERSION);
    // Where the API offers it, let it re-run a request a safety classifier
    // declined on the fallback model it picks, inside the same call.
    if ANTHROPIC_SERVER_FALLBACK.contains(&model) {
        body["fallbacks"] = json!("default");
        builder = builder.header("anthropic-beta", "server-side-fallback-2026-07-01");
    }
    builder.json(&body)
}

fn anthropic_event(event: &SseEvent, completion: &mut Completion) -> Result<Option<String>> {
    let Ok(data) = serde_json::from_str::<Value>(&event.data) else {
        return Ok(None);
    };
    match data["type"].as_str() {
        Some("message_start") => {
            let message = &data["message"];
            if let Some(model) = message["model"].as_str() {
                completion.model = model.to_owned();
            }
            completion.usage.input_tokens = message["usage"]["input_tokens"].as_u64();
        }
        // Only text is shown; thinking and other block types are skipped.
        Some("content_block_delta") if data["delta"]["type"] == "text_delta" => {
            return Ok(data["delta"]["text"].as_str().map(str::to_owned));
        }
        Some("message_delta") => {
            if let Some(tokens) = data["usage"]["output_tokens"].as_u64() {
                completion.usage.output_tokens = Some(tokens);
            }
            match data["delta"]["stop_reason"].as_str() {
                Some("end_turn" | "stop_sequence") => completion.stop = StopReason::End,
                Some("max_tokens") => completion.stop = StopReason::Length,
                Some("refusal") => {
                    let category = data["delta"]["stop_details"]["category"]
                        .as_str()
                        .unwrap_or("policy");
                    return Err(ai_error(
                        "refused",
                        format!("The model declined this request ({category})."),
                    ));
                }
                _ => {}
            }
        }
        Some("error") => {
            let kind = data["error"]["type"].as_str().unwrap_or("error");
            let message = data["error"]["message"]
                .as_str()
                .unwrap_or("the provider reported an error");
            let reason = if kind == "overloaded_error" || kind == "rate_limit_error" {
                "rate_limit"
            } else {
                "server"
            };
            return Err(ai_error(reason, message));
        }
        _ => {}
    }
    Ok(None)
}

// ---- OpenAI and compatible services: chat completions ----

fn openai_request(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    key: Option<&str>,
    request: &ChatRequest,
    kind: ProviderKind,
) -> reqwest::RequestBuilder {
    let mut messages = vec![json!({ "role": "system", "content": request.system })];
    messages.extend(request.messages.iter().map(|m| {
        json!({
            "role": if m.role == Role::User { "user" } else { "assistant" },
            "content": m.content,
        })
    }));
    let mut body = json!({ "model": model, "stream": true, "messages": messages });
    // Token counts in the stream are an OpenAI extension some compatible servers reject.
    if kind == ProviderKind::OpenAi {
        body["stream_options"] = json!({ "include_usage": true });
    }
    let builder = http.post(format!("{base}/chat/completions")).json(&body);
    match key {
        Some(key) => builder.bearer_auth(key),
        None => builder,
    }
}

fn openai_event(event: &SseEvent, completion: &mut Completion) -> Result<Option<String>> {
    if event.data == "[DONE]" {
        return Ok(None);
    }
    let Ok(data) = serde_json::from_str::<Value>(&event.data) else {
        return Ok(None);
    };
    if let Some(message) = data["error"]["message"].as_str() {
        return Err(ai_error("server", message));
    }
    if let Some(model) = data["model"].as_str().filter(|m| !m.is_empty()) {
        completion.model = model.to_owned();
    }
    if let Some(tokens) = data["usage"]["prompt_tokens"].as_u64() {
        completion.usage.input_tokens = Some(tokens);
        completion.usage.output_tokens = data["usage"]["completion_tokens"].as_u64();
    }
    let choice = &data["choices"][0];
    match choice["finish_reason"].as_str() {
        Some("stop") => completion.stop = StopReason::End,
        Some("length") => completion.stop = StopReason::Length,
        Some("content_filter") => {
            return Err(ai_error(
                "refused",
                "The provider's content filter stopped this reply.",
            ));
        }
        _ => {}
    }
    Ok(choice["delta"]["content"].as_str().map(str::to_owned))
}

// ---- Google Gemini: streamGenerateContent ----

fn gemini_request(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    key: Option<&str>,
    request: &ChatRequest,
) -> reqwest::RequestBuilder {
    let body = json!({
        "system_instruction": { "parts": [{ "text": request.system }] },
        "contents": request.messages.iter().map(|m| json!({
            "role": if m.role == Role::User { "user" } else { "model" },
            "parts": [{ "text": m.content }],
        })).collect::<Vec<_>>(),
    });
    http.post(format!(
        "{base}/models/{model}:streamGenerateContent?alt=sse"
    ))
    .header("x-goog-api-key", key.unwrap_or_default())
    .json(&body)
}

fn gemini_event(event: &SseEvent, completion: &mut Completion) -> Result<Option<String>> {
    let Ok(data) = serde_json::from_str::<Value>(&event.data) else {
        return Ok(None);
    };
    if let Some(message) = data["error"]["message"].as_str() {
        return Err(ai_error("server", message));
    }
    if let Some(reason) = data["promptFeedback"]["blockReason"].as_str() {
        return Err(ai_error(
            "refused",
            format!("The provider blocked this request ({reason})."),
        ));
    }
    if let Some(tokens) = data["usageMetadata"]["promptTokenCount"].as_u64() {
        completion.usage.input_tokens = Some(tokens);
        completion.usage.output_tokens = data["usageMetadata"]["candidatesTokenCount"].as_u64();
    }
    if let Some(model) = data["modelVersion"].as_str() {
        completion.model = model.to_owned();
    }
    let candidate = &data["candidates"][0];
    match candidate["finishReason"].as_str() {
        Some("STOP") => completion.stop = StopReason::End,
        Some("MAX_TOKENS") => completion.stop = StopReason::Length,
        Some("SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT") => {
            return Err(ai_error(
                "refused",
                "The provider's safety filter stopped this reply.",
            ));
        }
        _ => {}
    }
    let text: String = candidate["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
        // Parts marked as the model's reasoning are not part of the reply.
        .filter(|part| part["thought"] != true)
        .filter_map(|part| part["text"].as_str())
        .collect();
    Ok(Some(text))
}

// ---- errors ----

fn network_error(e: reqwest::Error) -> Error {
    let what = if e.is_timeout() {
        "the provider did not respond in time"
    } else if e.is_connect() {
        "could not connect to the provider"
    } else {
        "the connection to the provider failed"
    };
    // The URL can carry nothing secret here, but the text is shown to the user: keep it short.
    ai_error("network", format!("{what}: {}", e.without_url()))
}

/// Maps an HTTP error to a reason the interface can act on. The provider's own
/// message is kept: it is usually the most useful part.
fn status_error(status: u16, body: &str, provider: &str, model: &str) -> Error {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            // Gemini answers with a one-element array; the others with an object.
            let v = if v.is_array() { v[0].clone() } else { v };
            v["error"]["message"]
                .as_str()
                .or(v["error"].as_str())
                .or(v["message"].as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    let (reason, summary) = match status {
        401 | 403 => ("auth", format!("{provider} rejected the API key")),
        404 if !model.is_empty() => ("model", format!("{provider} has no model named {model}")),
        429 => (
            "rate_limit",
            format!("{provider} is rate limiting this key or its quota is used up"),
        ),
        400 | 404 | 413 | 422 => ("request", format!("{provider} did not accept the request")),
        _ => ("server", format!("{provider} returned an error ({status})")),
    };
    let message = if detail.trim().is_empty() {
        summary
    } else {
        format!("{summary}: {}", detail.trim())
    };
    ai_error(reason, message)
}
