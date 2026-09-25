use super::*;
use crate::wallet_agent::{tool_definitions, ToolOutput};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

fn text_response() -> Value {
    json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Read the wallet result."}}]})
}
fn call_response() -> Value {
    json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"wallet_get_assets","arguments":"{}"}}]}}]})
}
#[test]
fn missing_and_invalid_configuration_is_clear_without_exposing_input() {
    for key in ["", "  "] {
        assert!(matches!(
            OpenAiModel::new(key, DEFAULT_MODEL),
            Err(ModelError::MissingApiKey)
        ));
    }
    assert!(matches!(
        OpenAiModel::new("not-a-credential\n", ""),
        Err(ModelError::Configuration)
    ));
    let error = OpenAiModel::new("invalid\nheader", DEFAULT_MODEL)
        .err()
        .unwrap();
    assert!(!format!("{error:?} {error}").contains("header"));
}
#[test]
fn request_preserves_roles_call_ids_results_and_strict_boundaries() {
    let history = vec![
        Message::User("assets".into()),
        Message::Call(ToolCall {
            id: "call_1".into(),
            name: "wallet_get_assets".into(),
            arguments: "{}".into(),
        }),
        Message::Result {
            call_id: "call_1".into(),
            output: ToolOutput::Assets { assets: vec![] },
            task: None,
        },
        Message::Assistant("empty".into()),
    ];
    let definitions = tool_definitions();
    let value =
        serde_json::to_value(request(DEFAULT_MODEL, &history, &definitions).unwrap()).unwrap();
    assert_eq!(value["model"], "gpt-5.6-terra");
    assert_eq!(value["messages"][0]["role"], "system");
    assert_eq!(value["messages"][1]["role"], "user");
    assert_eq!(value["messages"][2]["tool_calls"][0]["id"], "call_1");
    assert_eq!(value["messages"][3]["tool_call_id"], "call_1");
    assert_eq!(
        serde_json::from_str::<Value>(value["messages"][3]["content"].as_str().unwrap()).unwrap(),
        json!({"type":"assets","assets":[]})
    );
    assert_eq!(value["parallel_tool_calls"], false);
    assert_eq!(value["store"], false);
    assert_eq!(value["tools"].as_array().unwrap().len(), 8);
    for tool in value["tools"].as_array().unwrap() {
        assert_eq!(tool["function"]["strict"], true);
        assert_eq!(
            tool["function"]["parameters"]["additionalProperties"],
            false
        );
    }
    assert!(!value.to_string().contains("OPENAI_API_KEY"));
}
#[test]
fn response_mapping_and_malformed_arguments_reach_typed_dispatch() {
    assert!(matches!(
        decode(&serde_json::to_vec(&text_response()).unwrap()).unwrap(),
        ModelResponse::Text(_)
    ));
    let mut value = call_response();
    value["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] = json!("not json");
    assert!(
        matches!(decode(&serde_json::to_vec(&value).unwrap()).unwrap(),ModelResponse::Tool(call) if call.arguments=="not json")
    );
}
#[test]
fn reject_parallel_truncated_refused_and_invalid_responses() {
    let mut parallel = call_response();
    let call = parallel["choices"][0]["message"]["tool_calls"][0].clone();
    parallel["choices"][0]["message"]["tool_calls"]
        .as_array_mut()
        .unwrap()
        .push(call);
    let mut truncated = call_response();
    truncated["choices"][0]["finish_reason"] = json!("length");
    let mut refused = text_response();
    refused["choices"][0]["message"]["refusal"] = json!("refused");
    let mut wrong_role = text_response();
    wrong_role["choices"][0]["message"]["role"] = json!("user");
    for value in [
        parallel,
        truncated,
        refused,
        wrong_role,
        json!({"choices":[]}),
        json!({"choices":[{},{}]}),
    ] {
        assert!(matches!(
            decode(&serde_json::to_vec(&value).unwrap()),
            Err(ModelError::InvalidResponse)
        ));
    }
    assert!(matches!(
        decode(b"not json"),
        Err(ModelError::InvalidResponse)
    ));
}
#[derive(Clone)]
struct Fixture {
    requests: Arc<Mutex<Vec<Value>>>,
    status: StatusCode,
    body: String,
}
async fn handler(
    State(f): State<Fixture>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, String) {
    assert_eq!(
        headers.get("authorization").unwrap(),
        "Bearer not-a-credential"
    );
    assert!(!body.to_string().contains("not-a-credential"));
    f.requests.lock().unwrap().push(body);
    (f.status, f.body)
}
async fn server(
    status: StatusCode,
    body: String,
) -> (OpenAiModel, Fixture, tokio::task::JoinHandle<()>) {
    let fixture = Fixture {
        requests: Arc::new(Mutex::new(vec![])),
        status,
        body,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/v1/chat/completions", post(handler))
        .with_state(fixture.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut model = OpenAiModel::new("not-a-credential", DEFAULT_MODEL).unwrap();
    model.endpoint = format!("http://{address}/v1/chat/completions");
    (model, fixture, task)
}
#[tokio::test]
async fn http_contract_posts_authenticated_typed_request() {
    let (mut model, f, task) = server(StatusCode::OK, call_response().to_string()).await;
    assert!(
        matches!(model.respond(&[Message::User("assets".into())],&tool_definitions()).await.unwrap(),ModelResponse::Tool(call) if call.name=="wallet_get_assets")
    );
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    task.abort();
}
#[tokio::test]
async fn http_errors_are_sanitized_and_not_retried() {
    for status in [
        StatusCode::UNAUTHORIZED,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::TEMPORARY_REDIRECT,
    ] {
        let (mut model, f, task) = server(status, "sensitive remote error".into()).await;
        let error = model.respond(&[], &tool_definitions()).await.err().unwrap();
        assert!(matches!(error,ModelError::Http(code) if code==status.as_u16()));
        assert!(!format!("{error:?} {error}").contains("sensitive"));
        assert_eq!(f.requests.lock().unwrap().len(), 1);
        task.abort();
    }
}
#[tokio::test]
async fn oversized_or_malformed_body_is_rejected() {
    for body in ["x".repeat(MAX_RESPONSE_BYTES + 1), "invalid json".into()] {
        let (mut model, _, task) = server(StatusCode::OK, body).await;
        assert!(matches!(
            model.respond(&[], &tool_definitions()).await,
            Err(ModelError::InvalidResponse)
        ));
        task.abort();
    }
}
