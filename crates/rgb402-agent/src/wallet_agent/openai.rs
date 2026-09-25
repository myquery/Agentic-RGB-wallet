//! Minimal Chat Completions transport. Credentials never enter the conversation.
use super::{
    AgentModel, Message, ModelError, ModelResponse, ToolCall, ToolDefinition, SYSTEM_INSTRUCTIONS,
};
use async_trait::async_trait;
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
    Client,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
pub const DEFAULT_MODEL: &str = "gpt-5.6-terra";
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

// No Debug or Serialize: the HTTP client holds a sensitive authorization header.
pub struct OpenAiModel {
    http: Client,
    model: String,
    endpoint: String,
}
impl OpenAiModel {
    /// Read process environment only. Never load, create, or update an env file.
    pub fn from_env() -> Result<Self, ModelError> {
        let key = std::env::var("OPENAI_API_KEY").map_err(|_| ModelError::MissingApiKey)?;
        let model = std::env::var("AGENT_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        Self::new(&key, &model)
    }
    fn new(key: &str, model: &str) -> Result<Self, ModelError> {
        if key.trim().is_empty() {
            return Err(ModelError::MissingApiKey);
        }
        if model.is_empty()
            || model.len() > 128
            || !model
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._".contains(&c))
        {
            return Err(ModelError::Configuration);
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", key.trim()))
            .map_err(|_| ModelError::Configuration)?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        let http = Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(45))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| ModelError::Transport)?;
        Ok(Self {
            http,
            model: model.into(),
            endpoint: ENDPOINT.into(),
        })
    }
}
#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    messages: Vec<WireMessage>,
    tools: Vec<FunctionTool<'a>>,
    tool_choice: &'static str,
    parallel_tool_calls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
    max_completion_tokens: u32,
    store: bool,
}
#[derive(Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
enum WireMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<WireCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}
#[derive(Serialize)]
struct FunctionTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: FunctionDefinition<'a>,
}
#[derive(Serialize)]
struct FunctionDefinition<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
    strict: bool,
}
#[derive(Serialize, Deserialize)]
struct WireCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: FunctionCall,
}
#[derive(Serialize, Deserialize)]
struct FunctionCall {
    name: String,
    arguments: String,
}
#[derive(Deserialize)]
struct Response {
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
struct Choice {
    finish_reason: String,
    message: AssistantMessage,
}
#[derive(Deserialize)]
struct AssistantMessage {
    role: String,
    content: Option<String>,
    refusal: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireCall>,
}
fn request<'a>(
    model: &'a str,
    conversation: &[Message],
    tools: &'a [ToolDefinition],
) -> Result<Request<'a>, ModelError> {
    let mut messages = vec![WireMessage::System {
        content: SYSTEM_INSTRUCTIONS.into(),
    }];
    for message in conversation {
        messages.push(match message {
            Message::User(content) => WireMessage::User {
                content: content.clone(),
            },
            Message::Assistant(content) => WireMessage::Assistant {
                content: Some(content.clone()),
                tool_calls: vec![],
            },
            Message::Call(call) => WireMessage::Assistant {
                content: None,
                tool_calls: vec![WireCall {
                    id: call.id.clone(),
                    kind: "function".into(),
                    function: FunctionCall {
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    },
                }],
            },
            Message::Result {
                call_id,
                output,
                task,
            } => WireMessage::Tool {
                tool_call_id: call_id.clone(),
                content: serde_json::to_string(&super::observation::with_task(
                    output,
                    task.as_deref(),
                ))
                .map_err(|_| ModelError::InvalidResponse)?,
            },
        });
    }
    Ok(Request {
        model,
        messages,
        tools: tools
            .iter()
            .map(|t| FunctionTool {
                kind: "function",
                function: FunctionDefinition {
                    name: t.name,
                    description: t.description,
                    parameters: &t.parameters,
                    strict: true,
                },
            })
            .collect(),
        tool_choice: "auto",
        parallel_tool_calls: false,
        // Terra's Chat Completions endpoint requires reasoning to be disabled
        // when function tools are present. Other model overrides keep their
        // provider defaults and receive no Terra-specific parameter.
        reasoning_effort: (model == "gpt-5.6-terra").then_some("none"),
        max_completion_tokens: 2048,
        store: false,
    })
}
fn decode(bytes: &[u8]) -> Result<ModelResponse, ModelError> {
    let response: Response =
        serde_json::from_slice(bytes).map_err(|_| ModelError::InvalidResponse)?;
    if response.choices.len() != 1 {
        return Err(ModelError::InvalidResponse);
    }
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or(ModelError::InvalidResponse)?;
    let message = choice.message;
    if message.role != "assistant" || message.refusal.is_some() {
        return Err(ModelError::InvalidResponse);
    }
    match (choice.finish_reason.as_str(), message.tool_calls.len()) {
        ("tool_calls", 1) => {
            let call = message
                .tool_calls
                .into_iter()
                .next()
                .ok_or(ModelError::InvalidResponse)?;
            if call.kind != "function"
                || call.id.is_empty()
                || call.id.len() > 128
                || call.function.name.is_empty()
                || call.function.name.len() > 128
            {
                return Err(ModelError::InvalidResponse);
            }
            // Argument validation belongs to the typed wallet dispatcher, including malformed JSON.
            Ok(ModelResponse::Tool(ToolCall {
                id: call.id,
                name: call.function.name,
                arguments: call.function.arguments,
            }))
        }
        ("stop", 0) => {
            let text = message
                .content
                .filter(|s| !s.trim().is_empty())
                .ok_or(ModelError::InvalidResponse)?;
            Ok(ModelResponse::Text(text))
        }
        _ => Err(ModelError::InvalidResponse),
    }
}
#[async_trait]
impl AgentModel for OpenAiModel {
    async fn respond(
        &mut self,
        conversation: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<ModelResponse, ModelError> {
        let body = request(&self.model, conversation, tools)?;
        let mut response = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|_| ModelError::Transport)?;
        if !response.status().is_success() {
            return Err(ModelError::Http(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err(ModelError::InvalidResponse);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ModelError::Transport)? {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(ModelError::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        decode(&bytes)
    }
}
#[cfg(test)]
mod tests;
