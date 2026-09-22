use super::provider::explicit_provider;
use crate::Observation;
use chrono::Utc;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};
pub fn parse_events<I: IntoIterator<Item = String>>(lines: I) -> (Option<String>, Option<Value>) {
    let mut model = None;
    let mut routing = None;
    for line in lines {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some((m, _, reason)) = explicit_provider(&v) {
            model = Some(m);
            routing = Some(serde_json::json!({"reason":reason}))
        }
    }
    (model, routing)
}
pub fn run(model: String, effort: String) -> Observation {
    let mut cmd = Command::new("codex");
    cmd.args(["exec", "--json", "--model", &model]);
    if !effort.is_empty() {
        cmd.args([
            "-c",
            &format!("model_reasoning_effort=\"{}\"", effort.replace('"', "")),
        ]);
    }
    cmd.arg("hi")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let (provider, details) = match cmd.spawn() {
        Ok(mut child) => {
            let lines = child
                .stdout
                .take()
                .map(|x| {
                    BufReader::new(x)
                        .lines()
                        .map_while(Result::ok)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let parsed = parse_events(lines);
            match child.wait() {
                Ok(s) if s.success() => (
                    parsed.0,
                    Some(serde_json::json!({"routing":parsed.1}).to_string()),
                ),
                Ok(s) => (
                    None,
                    Some(
                        serde_json::json!({"probe_error":format!("Codex exited with {s}")})
                            .to_string(),
                    ),
                ),
                Err(e) => (
                    None,
                    Some(serde_json::json!({"probe_error":e.to_string()}).to_string()),
                ),
            }
        }
        Err(e) => (
            None,
            Some(
                serde_json::json!({"probe_error":format!("Unable to start Codex: {e}")})
                    .to_string(),
            ),
        ),
    };
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
        evidence: "manual codex exec --json".into(),
        details,
    }
}
