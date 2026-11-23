//! Each provider against a local stand-in server: what is sent, and how the
//! streamed reply, the token counts and the errors are read.

use std::sync::Mutex;

use dbclient_ai::{
    CancellationToken, ChatMessage, ChatRequest, ProviderConfig, ProviderKind, Role, StopReason,
    StreamEvent, http_client, list_models, stream_chat,
};
use dbclient_core::Error;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider(kind: ProviderKind, server: &MockServer, model: Option<&str>) -> ProviderConfig {
    ProviderConfig {
        id: "p".into(),
        kind,
        name: "Test provider".into(),
        base_url: Some(server.uri()),
        model: model.map(str::to_owned),
    }
}

fn request() -> ChatRequest {
    ChatRequest {
        system: "You help with SQL.".into(),
        messages: vec![
            ChatMessage {
                role: Role::User,
                content: "first".into(),
            },
            ChatMessage {
                role: Role::Assistant,
                content: "answer".into(),
            },
            ChatMessage {
                role: Role::User,
                content: "second".into(),
            },
        ],
    }
}

fn sse(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.to_owned(), "text/event-stream")
}

async fn run(
    provider: &ProviderConfig,
    key: Option<&str>,
) -> (Result<dbclient_ai::Completion, Error>, String) {
    let streamed = Mutex::new(String::new());
    let result = stream_chat(
        &http_client().unwrap(),
        provider,
        key,
        &request(),
        &CancellationToken::new(),
        |event| {
            let StreamEvent::Text { text } = event;
            streamed.lock().unwrap().push_str(&text);
        },
    )
    .await;
    (result, streamed.into_inner().unwrap())
}

fn reason(error: &Error) -> &str {
    match error {
        Error::Ai { reason, .. } => reason,
        other => panic!("not an AI error: {other:?}"),
    }
}

#[tokio::test]
async fn anthropic_streams_text_and_reports_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-test"))
        .and(header("anthropic-version", "2026-10-21"))
        .and(header("anthropic-beta", "server-side-fallback-2026-07-01"))
        .and(body_partial_json(json!({
            "model": "claude-opus-5-5",
            "stream": true,
            "fallbacks": "default",
            "system": "You help with SQL.",
            "messages": [
                { "role": "user", "content": "first" },
                { "role": "assistant", "content": "answer" },
                { "role": "user", "content": "second" },
            ],
        })))
        .respond_with(sse(concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-opus-5-5\",\"usage\":{\"input_tokens\":42}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hidden\"}}\n\n",
            "event: ping\ndata: {\"type\":\"ping\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"SELECT \"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"1;\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        )))
        .expect(1)
        .mount(&server)
        .await;

    // No model chosen: the default is used.
    let (result, streamed) = run(
        &provider(ProviderKind::Anthropic, &server, None),
        Some("sk-test"),
    )
    .await;
    let completion = result.unwrap();
    assert_eq!(completion.text, "SELECT 1;");
    assert_eq!(streamed, "SELECT 1;");
    assert_eq!(completion.model, "claude-opus-5-5");
    assert_eq!(completion.stop, StopReason::End);
    assert_eq!(
        (
            completion.usage.input_tokens,
            completion.usage.output_tokens
        ),
        (Some(42), Some(7))
    );
}

#[tokio::test]
async fn anthropic_other_models_send_no_fallback_and_refusals_are_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(sse(concat!(
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"I\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\",\"stop_details\":{\"type\":\"refusal\",\"category\":\"cyber\"}},\"usage\":{\"output_tokens\":1}}\n\n",
        )))
        .mount(&server)
        .await;

    let (result, _) = run(
        &provider(ProviderKind::Anthropic, &server, Some("claude-haiku-5-5")),
        Some("k"),
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(reason(&error), "refused");
    assert!(error.to_string().contains("cyber"));

    let sent = &server.received_requests().await.unwrap()[0];
    assert!(sent.headers.get("anthropic-beta").is_none());
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert!(body.get("fallbacks").is_none());
    // Sampling parameters are rejected by this API version; none may be sent.
    assert!(body.get("temperature").is_none());
}

#[tokio::test]
async fn openai_streams_text_and_reports_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer sk-openai"))
        .and(body_partial_json(json!({
            "model": "some-model",
            "stream": true,
            "stream_options": { "include_usage": true },
            "messages": [
                { "role": "system", "content": "You help with SQL." },
                { "role": "user", "content": "first" },
                { "role": "assistant", "content": "answer" },
                { "role": "user", "content": "second" },
            ],
        })))
        .respond_with(sse(concat!(
            "data: {\"model\":\"some-model-0001\",\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Mer\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"haba\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":30,\"completion_tokens\":4}}\n\n",
            "data: [DONE]\n\n",
        )))
        .expect(1)
        .mount(&server)
        .await;

    let (result, streamed) = run(
        &provider(ProviderKind::OpenAi, &server, Some("some-model")),
        Some("sk-openai"),
    )
    .await;
    let completion = result.unwrap();
    assert_eq!(streamed, "Merhaba");
    assert_eq!(completion.model, "some-model-0001");
    assert_eq!(completion.stop, StopReason::End);
    assert_eq!(
        (
            completion.usage.input_tokens,
            completion.usage.output_tokens
        ),
        (Some(30), Some(4))
    );
}

#[tokio::test]
async fn compatible_servers_work_without_a_key_and_without_openai_extensions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(sse("data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n"))
        .mount(&server)
        .await;

    let (result, _) = run(
        &provider(ProviderKind::OpenAiCompatible, &server, Some("llama")),
        None,
    )
    .await;
    let completion = result.unwrap();
    assert_eq!(completion.text, "ok");
    assert_eq!(completion.stop, StopReason::Length);

    let sent = &server.received_requests().await.unwrap()[0];
    assert!(sent.headers.get("authorization").is_none());
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert!(body.get("stream_options").is_none());
}

#[tokio::test]
async fn gemini_streams_text_and_reports_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-test:streamGenerateContent"))
        .and(query_param("alt", "sse"))
        .and(header("x-goog-api-key", "g-key"))
        .and(body_partial_json(json!({
            "system_instruction": { "parts": [{ "text": "You help with SQL." }] },
            "contents": [
                { "role": "user", "parts": [{ "text": "first" }] },
                { "role": "model", "parts": [{ "text": "answer" }] },
                { "role": "user", "parts": [{ "text": "second" }] },
            ],
        })))
        .respond_with(sse(concat!(
            "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"reasoning\",\"thought\":true},{\"text\":\"SELECT\"}]}}]}\r\n\r\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" 1\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":12,\"candidatesTokenCount\":3},\"modelVersion\":\"gemini-test-001\"}\r\n\r\n",
        )))
        .expect(1)
        .mount(&server)
        .await;

    let (result, streamed) = run(
        &provider(ProviderKind::Gemini, &server, Some("gemini-test")),
        Some("g-key"),
    )
    .await;
    let completion = result.unwrap();
    assert_eq!(streamed, "SELECT 1");
    assert_eq!(completion.model, "gemini-test-001");
    assert_eq!(completion.stop, StopReason::End);
    assert_eq!(
        (
            completion.usage.input_tokens,
            completion.usage.output_tokens
        ),
        (Some(12), Some(3))
    );
}

#[tokio::test]
async fn http_errors_become_reasons_the_interface_can_act_on() {
    for (status, body, expected, fragment) in [
        (
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
            "auth",
            "invalid x-api-key",
        ),
        (
            429,
            r#"{"error":{"message":"You exceeded your current quota"}}"#,
            "rate_limit",
            "quota",
        ),
        (
            404,
            r#"{"error":{"message":"model not found"}}"#,
            "model",
            "no model named",
        ),
        (
            400,
            r#"{"error":{"message":"messages: at least one message is required"}}"#,
            "request",
            "at least one message",
        ),
        (529, "", "server", "529"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_raw(body, "application/json"))
            .mount(&server)
            .await;
        let (result, _) = run(
            &provider(ProviderKind::Anthropic, &server, Some("claude-opus-5-5")),
            Some("k"),
        )
        .await;
        let error = result.unwrap_err();
        assert_eq!(reason(&error), expected, "status {status}");
        assert!(error.to_string().contains(fragment), "{error}");
    }
}

#[tokio::test]
async fn a_missing_key_or_model_is_reported_before_anything_is_sent() {
    let server = MockServer::start().await;
    let (no_key, _) = run(&provider(ProviderKind::OpenAi, &server, Some("m")), None).await;
    assert_eq!(reason(&no_key.unwrap_err()), "not_configured");
    let (no_model, _) = run(&provider(ProviderKind::Gemini, &server, None), Some("k")).await;
    assert_eq!(reason(&no_model.unwrap_err()), "not_configured");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn cancelling_stops_a_reply_in_progress() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse("data: {}\n\n").set_delay(std::time::Duration::from_secs(30)))
        .mount(&server)
        .await;
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        stopper.cancel();
    });
    let started = std::time::Instant::now();
    let result = stream_chat(
        &http_client().unwrap(),
        &provider(ProviderKind::OpenAi, &server, Some("m")),
        Some("k"),
        &request(),
        &cancel,
        |_| {},
    )
    .await;
    assert_eq!(reason(&result.unwrap_err()), "cancelled");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn lists_models_for_each_provider() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("x-api-key", "k"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [
            { "id": "claude-sonnet-5-5", "display_name": "Claude Sonnet 5.5" },
            { "id": "claude-opus-5-5", "display_name": "Claude Opus 5.5" },
        ]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", "Bearer k"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "data": [{ "id": "b-model" }, { "id": "a-model" }] })),
        )
        .mount(&server)
        .await;
    let http = http_client().unwrap();

    let anthropic = list_models(
        &http,
        &provider(ProviderKind::Anthropic, &server, None),
        Some("k"),
    )
    .await
    .unwrap();
    assert_eq!(anthropic[0].id, "claude-opus-5-5");
    assert_eq!(anthropic[0].name, "Claude Opus 5.5");
    let openai = list_models(
        &http,
        &provider(ProviderKind::OpenAi, &server, None),
        Some("k"),
    )
    .await
    .unwrap();
    assert_eq!(
        openai.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["a-model", "b-model"]
    );

    let gemini_server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/models")).and(header("x-goog-api-key", "k"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [
            { "name": "models/gemini-chat", "displayName": "Gemini Chat", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/embedding", "displayName": "Embedding", "supportedGenerationMethods": ["embedContent"] },
        ]})))
        .mount(&gemini_server).await;
    let gemini = list_models(
        &http,
        &provider(ProviderKind::Gemini, &gemini_server, None),
        Some("k"),
    )
    .await
    .unwrap();
    assert_eq!(gemini.len(), 1);
    assert_eq!(
        (gemini[0].id.as_str(), gemini[0].name.as_str()),
        ("gemini-chat", "Gemini Chat")
    );

    let bad_key = list_models(
        &http,
        &provider(ProviderKind::OpenAi, &server, None),
        Some("wrong"),
    )
    .await;
    assert!(bad_key.is_err());
}
