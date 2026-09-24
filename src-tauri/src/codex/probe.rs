use crate::Observation;
use chrono::Utc;
use reqwest::blocking::Client;
use serde_json::Value;
use std::{fs, path::Path, time::Duration};

const RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ProbeEvidence {
    pub model: Option<String>,
    pub response_id: Option<String>,
}

/// Parse only the typed SSE payload used by the Responses API. Conversation
/// content is deliberately ignored and never returned to callers.
pub fn parse_sse(input: &str) -> ProbeEvidence {
    let mut out = ProbeEvidence::default();
    for line in input.lines() {
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("response.created") {
            continue;
        }
        let Some(response) = v.get("response") else {
            continue;
        };
        out.model = response
            .get("model")
            .and_then(Value::as_str)
            .or_else(|| {
                response
                    .pointer("/headers/OpenAI-Model")
                    .and_then(Value::as_str)
            })
            .or_else(|| {
                response
                    .pointer("/headers/openai-model")
                    .and_then(Value::as_str)
            })
            .map(str::to_owned);
        out.response_id = response
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if out.model.is_some() {
            break;
        }
    }
    out
}

fn failed(model: String, effort: String, class: &str, message: impl ToString) -> Observation {
    observation(
        model,
        effort,
        None,
        serde_json::json!({"status":"failed","error_class":class,"message":message.to_string()}),
    )
}

fn observation(
    model: String,
    effort: String,
    provider: Option<String>,
    details: Value,
) -> Observation {
    Observation {
        id: None,
        time: Utc::now().to_rfc3339(),
        kind: "Probe".into(),
        session_id: None,
        turn_id: None,
        selected_model: Some(model),
        selected_effort: (!effort.is_empty()).then_some(effort),
        runtime_model: None,
        runtime_effort: None,
        provider_model: provider,
        evidence: "manual Responses API probe".into(),
        details: Some(details.to_string()),
    }
}

pub fn run(codex_home: &Path, model: String, effort: String) -> Observation {
    let auth: Value = match fs::read(codex_home.join("auth.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
    {
        Some(v) => v,
        None => {
            return failed(
                model,
                effort,
                "auth",
                "Codex authentication was not found; run codex login",
            )
        }
    };
    let Some(token) = auth.pointer("/tokens/access_token").and_then(Value::as_str) else {
        return failed(
            model,
            effort,
            "auth",
            "Codex access token is unavailable; run codex login",
        );
    };
    let account = auth.pointer("/tokens/account_id").and_then(Value::as_str);
    let client = match Client::builder()
        .timeout(Duration::from_secs(90))
        .user_agent("codex_cli_rs")
        .build()
    {
        Ok(c) => c,
        Err(e) => return failed(model, effort, "network", e),
    };
    let mut request = client.post(RESPONSES_URL).bearer_auth(token).header("originator", "codex_cli_rs")
        .header("accept", "text/event-stream").json(&serde_json::json!({
            "model": model, "instructions": "You are a helpful assistant.",
            "input": [{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}],
            "stream": true, "store": false,
            "reasoning": {"effort": if effort.is_empty() { "low" } else { effort.as_str() }}
        }));
    if let Some(id) = account {
        request = request.header("chatgpt-account-id", id);
    }
    let response = match request.send() {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return failed(model, effort, "network", "Probe timed out"),
        Err(e) => return failed(model, effort, "network", e),
    };
    let status = response.status();
    if !status.is_success() {
        let class = if status.as_u16() == 401 || status.as_u16() == 403 {
            "auth"
        } else if status.as_u16() == 429 || status.as_u16() == 503 {
            "capacity"
        } else {
            "protocol"
        };
        return failed(
            model,
            effort,
            class,
            format!("Backend returned HTTP {}", status.as_u16()),
        );
    }
    let safety = response
        .headers()
        .get("x-codex-safety-buffering-enabled")
        .and_then(|x| x.to_str().ok())
        .map(str::to_owned);
    let header_model = response
        .headers()
        .get("openai-model")
        .and_then(|x| x.to_str().ok())
        .map(str::to_owned);
    let body = match response.text() {
        Ok(x) => x,
        Err(e) => return failed(model, effort, "protocol", e),
    };
    let parsed = parse_sse(&body);
    let provider = parsed.model.or(header_model);
    let Some(provider) = provider else {
        return failed(
            model,
            effort,
            "protocol",
            "No explicit provider model in response.created or OpenAI-Model header",
        );
    };
    observation(
        model,
        effort,
        Some(provider),
        serde_json::json!({"status":"succeeded","response_id":parsed.response_id,"safety_buffering":safety}),
    )
}
