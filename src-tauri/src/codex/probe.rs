use crate::Observation;
use chrono::Utc;
use reqwest::blocking::Client;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::Path,
    time::Duration,
};

const RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const MAX_SSE_BYTES: u64 = 256 * 1024;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ProbeEvidence {
    pub model: Option<String>,
    pub response_id: Option<String>,
}

pub fn normalize_request(model: &str, effort: &str) -> Result<(String, String), &'static str> {
    let model = model.trim();
    if model.is_empty() {
        return Err("Model is required");
    }
    let effort = effort.trim();
    Ok((
        model.to_owned(),
        if effort.is_empty() {
            "low".into()
        } else {
            effort.to_owned()
        },
    ))
}

fn parse_data(data: &str) -> Option<ProbeEvidence> {
    if data == "[DONE]" {
        return None;
    }
    let value = serde_json::from_str::<Value>(data).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("response.created") {
        return None;
    }
    let response = value.get("response")?;
    let model = response
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(ProbeEvidence {
        model,
        response_id: response
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// Reads framed SSE incrementally and returns immediately after structured
/// `response.created.response.model` evidence. All other response content is ignored.
pub fn parse_sse_reader(reader: impl Read, maximum: u64) -> Result<ProbeEvidence, &'static str> {
    let mut reader = BufReader::new(reader.take(maximum + 1));
    let mut consumed = 0_u64;
    let mut data = Vec::<String>::new();
    loop {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|_| "Unable to read probe stream")?;
        if bytes == 0 {
            break;
        }
        consumed += bytes as u64;
        if consumed > maximum {
            return Err("Probe response exceeded the safe streaming limit");
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if !data.is_empty() {
                if let Some(evidence) = parse_data(&data.join("\n")) {
                    if evidence.model.is_some() {
                        return Ok(evidence);
                    }
                }
                data.clear();
            }
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start().to_owned());
        }
    }
    if let Some(evidence) = parse_data(&data.join("\n")) {
        return Ok(evidence);
    }
    Ok(ProbeEvidence::default())
}

pub fn parse_sse(input: &str) -> ProbeEvidence {
    parse_sse_reader(input.as_bytes(), input.len() as u64).unwrap_or_default()
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
        selected_effort: Some(effort),
        runtime_model: None,
        runtime_effort: None,
        provider_model: provider,
        evidence: "manual Responses API probe".into(),
        details: Some(details.to_string()),
    }
}

pub fn run(codex_home: &Path, model: String, effort: String) -> Observation {
    let (model, effort) = match normalize_request(&model, &effort) {
        Ok(values) => values,
        Err(message) => {
            return failed(
                model.trim().to_owned(),
                effort.trim().to_owned(),
                "input",
                message,
            )
        }
    };
    let auth: Value = match fs::read(codex_home.join("auth.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        Some(value) => value,
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
        Ok(client) => client,
        Err(error) => return failed(model, effort, "network", error),
    };
    let mut request = client.post(RESPONSES_URL).bearer_auth(token).header("originator", "codex_cli_rs")
        .header("accept", "text/event-stream").json(&serde_json::json!({
            "model": model, "instructions": "You are a helpful assistant.",
            "input": [{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}],
            "stream": true, "store": false, "reasoning": {"effort": effort}
        }));
    if let Some(id) = account {
        request = request.header("chatgpt-account-id", id);
    }
    let mut response = match request.send() {
        Ok(response) => response,
        Err(error) if error.is_timeout() => {
            return failed(model, effort, "network", "Probe timed out")
        }
        Err(error) => return failed(model, effort, "network", error),
    };
    let status = response.status();
    if !status.is_success() {
        let class = if matches!(status.as_u16(), 401 | 403) {
            "auth"
        } else if matches!(status.as_u16(), 429 | 503) {
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
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let header_model = response
        .headers()
        .get("openai-model")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let parsed = match parse_sse_reader(&mut response, MAX_SSE_BYTES) {
        Ok(parsed) => parsed,
        Err(message) => return failed(model, effort, "protocol", message),
    };
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
