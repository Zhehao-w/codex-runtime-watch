use super::{models::Evidence, provider::explicit_provider};
use crate::Observation;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;

fn string(v: &Value, paths: &[&str]) -> Option<String> {
    paths
        .iter()
        .find_map(|p| v.pointer(p).and_then(Value::as_str).map(str::to_owned))
}
pub fn adapt(v: &Value) -> Option<Evidence> {
    let kind = string(v, &["/type", "/payload/type"])?;
    let mut e = Evidence {
        time: string(v, &["/timestamp", "/time", "/payload/timestamp"]),
        session_id: string(
            v,
            &[
                "/session_id",
                "/payload/session_id",
                "/payload/conversation_id",
            ],
        ),
        thread_id: string(v, &["/thread_id", "/payload/thread_id"]),
        turn_id: string(v, &["/turn_id", "/payload/turn_id", "/payload/id"]),
        source: kind.clone(),
        ..Default::default()
    };
    match kind.as_str() {
        "thread_settings_applied" | "thread/configured" | "thread_configured" => {
            e.selected_model = string(v, &["/payload/model", "/payload/model_name", "/model"]);
            e.selected_effort = string(
                v,
                &[
                    "/payload/reasoning_effort",
                    "/payload/effort",
                    "/payload/config/reasoning_effort",
                ],
            );
        }
        "turn_context" => {
            e.runtime_model = string(v, &["/payload/model"]);
            e.runtime_effort = string(v, &["/payload/effort", "/payload/reasoning_effort"]);
            e.parent_thread = string(v, &["/payload/parent_thread_id", "/payload/parent_thread"]);
            e.is_subagent = e.parent_thread.is_some()
                || v.pointer("/payload/subagent")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
        }
        _ => {
            if let Some((m, s, r)) = explicit_provider(v) {
                e.provider_model = Some(m);
                e.source = s;
                e.provider_reason = r
            } else {
                return None;
            }
        }
    }
    Some(e)
}

#[derive(Default)]
pub struct Correlator {
    selected: HashMap<String, (Option<String>, Option<String>)>,
    pending: HashMap<String, Observation>,
}
impl Correlator {
    fn key(e: &Evidence) -> Option<String> {
        e.thread_id.clone().or_else(|| e.session_id.clone())
    }
    pub fn push(&mut self, e: Evidence) -> Option<Observation> {
        let key = Self::key(&e)?;
        if e.selected_model.is_some() || e.selected_effort.is_some() {
            self.selected
                .insert(key.clone(), (e.selected_model, e.selected_effort));
            return None;
        }
        let turn = e.turn_id.clone()?;
        let identity = format!("{key}:{turn}");
        let o = self.pending.entry(identity.clone()).or_insert_with(|| {
            let s = self.selected.get(&key).cloned().unwrap_or_default();
            Observation {
                id: None,
                time: e.time.clone().unwrap_or_else(|| Utc::now().to_rfc3339()),
                kind: "Runtime".into(),
                session_id: e.session_id.clone().or_else(|| e.thread_id.clone()),
                turn_id: Some(turn),
                selected_model: s.0,
                selected_effort: s.1,
                runtime_model: None,
                runtime_effort: None,
                provider_model: None,
                evidence: e.source.clone(),
                details: None,
            }
        });
        if e.runtime_model.is_some() {
            o.runtime_model = e.runtime_model;
            o.runtime_effort = e.runtime_effort;
            o.evidence = "turn_context".into();
            o.details = Some(
                serde_json::json!({"parent_thread":e.parent_thread,"subagent":e.is_subagent})
                    .to_string(),
            )
        }
        if e.provider_model.is_some() {
            o.provider_model = e.provider_model;
            o.evidence = e.source;
            o.details = Some(serde_json::json!({"provider_reason":e.provider_reason}).to_string())
        }
        Some(o.clone())
    }
}
