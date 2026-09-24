use super::{models::Evidence, provider::explicit_provider};
use crate::Observation;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

fn string(v: &Value, paths: &[&str]) -> Option<String> {
    paths
        .iter()
        .find_map(|p| v.pointer(p).and_then(Value::as_str).map(str::to_owned))
}
pub fn adapt(v: &Value) -> Option<Evidence> {
    let outer = string(v, &["/type", "/method"])?;
    let kind = if outer == "event_msg" {
        string(v, &["/payload/type"])?
    } else {
        outer
    };
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
        thread_id: string(v, &["/thread_id", "/payload/thread_id", "/params/threadId"]),
        turn_id: string(
            v,
            &[
                "/turn_id",
                "/payload/turn_id",
                "/payload/id",
                "/params/turnId",
            ],
        ),
        source: kind.clone(),
        ..Default::default()
    };
    match kind.as_str() {
        "session_meta" => {
            e.session_id = string(v, &["/payload/id", "/payload/session_id"]);
            e.thread_id = string(v, &["/payload/id"]);
            e.parent_thread = string(v, &["/payload/parent_thread_id"]);
            e.is_subagent = e.parent_thread.is_some()
                || v.pointer("/payload/agent_path")
                    .and_then(Value::as_str)
                    .is_some();
        }
        "thread_settings_applied" | "thread/configured" | "thread_configured" => {
            e.selected_model = string(
                v,
                &[
                    "/payload/thread_settings/model",
                    "/payload/model",
                    "/payload/model_name",
                    "/model",
                ],
            );
            e.selected_effort = string(
                v,
                &[
                    "/payload/reasoning_effort",
                    "/payload/thread_settings/reasoning_effort",
                    "/payload/effort",
                    "/payload/config/reasoning_effort",
                ],
            );
        }
        "turn_context" => {
            e.selected_model = string(v, &["/payload/collaboration_mode/settings/model"]);
            e.selected_effort = string(
                v,
                &["/payload/collaboration_mode/settings/reasoning_effort"],
            );
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
    sessions: HashMap<String, (String, Option<String>, bool)>,
    pending: HashMap<String, Observation>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PersistedScope {
    session: Option<String>,
    parent_thread: Option<String>,
    is_subagent: bool,
    selected_model: Option<String>,
    selected_effort: Option<String>,
}

impl Correlator {
    pub fn push(&mut self, e: Evidence) -> Option<Observation> {
        self.push_scoped("default", e)
    }
    pub fn push_scoped(&mut self, scope: &str, e: Evidence) -> Option<Observation> {
        if e.source == "session_meta" {
            if let Some(id) = e.thread_id.or(e.session_id) {
                self.sessions
                    .insert(scope.to_owned(), (id, e.parent_thread, e.is_subagent));
            }
            return None;
        }
        let key = e
            .thread_id
            .clone()
            .or_else(|| e.session_id.clone())
            .or_else(|| self.sessions.get(scope).map(|x| x.0.clone()))
            .unwrap_or_else(|| scope.to_owned());
        if e.source != "turn_context" && (e.selected_model.is_some() || e.selected_effort.is_some())
        {
            let selected = (e.selected_model, e.selected_effort);
            self.selected.insert(key.clone(), selected.clone());
            self.selected.insert(scope.to_owned(), selected);
            return None;
        }
        let turn = e.turn_id.clone()?;
        let identity = format!("{key}:{turn}");
        let o = self.pending.entry(identity.clone()).or_insert_with(|| {
            let fallback = self
                .selected
                .get(&key)
                .or_else(|| self.selected.get(scope))
                .cloned()
                .unwrap_or_default();
            let selected_model = e.selected_model.clone().or(fallback.0);
            let selected_effort = e.selected_effort.clone().or(fallback.1);
            let session = e
                .session_id
                .clone()
                .or_else(|| e.thread_id.clone())
                .or_else(|| self.sessions.get(scope).map(|x| x.0.clone()))
                .or_else(|| Some(scope.to_owned()));
            Observation {
                id: None,
                time: e.time.clone().unwrap_or_else(|| Utc::now().to_rfc3339()),
                kind: "Runtime".into(),
                session_id: session,
                turn_id: Some(turn),
                selected_model,
                selected_effort,
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
                serde_json::json!({"parent_thread":e.parent_thread.clone().or_else(|| self.sessions.get(scope).and_then(|x| x.1.clone())),"subagent":e.is_subagent || self.sessions.get(scope).is_some_and(|x| x.2)})
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

    pub fn restore_scope(&mut self, scope: &str, metadata: &str) {
        let Ok(state) = serde_json::from_str::<PersistedScope>(metadata) else {
            return;
        };
        if let Some(session) = state.session {
            self.sessions.insert(
                scope.to_owned(),
                (session.clone(), state.parent_thread, state.is_subagent),
            );
            self.selected.insert(
                session,
                (state.selected_model.clone(), state.selected_effort.clone()),
            );
        }
        self.selected.insert(
            scope.to_owned(),
            (state.selected_model, state.selected_effort),
        );
    }

    pub fn reset_scope(&mut self, scope: &str) {
        if let Some((session, _, _)) = self.sessions.remove(scope) {
            self.selected.remove(&session);
        }
        self.selected.remove(scope);
        self.pending
            .retain(|identity, _| !identity.starts_with(&format!("{scope}:")));
    }

    pub fn persist_scope(&self, scope: &str) -> String {
        let session = self.sessions.get(scope);
        let selected = session
            .and_then(|value| self.selected.get(&value.0))
            .or_else(|| self.selected.get(scope));
        serde_json::to_string(&PersistedScope {
            session: session.map(|value| value.0.clone()),
            parent_thread: session.and_then(|value| value.1.clone()),
            is_subagent: session.is_some_and(|value| value.2),
            selected_model: selected.and_then(|value| value.0.clone()),
            selected_effort: selected.and_then(|value| value.1.clone()),
        })
        .expect("persisted rollout scope is serializable")
    }
}
