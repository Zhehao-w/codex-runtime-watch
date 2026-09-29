pub mod codex;
pub mod db;
pub mod settings;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Observation {
    pub id: Option<i64>,
    pub time: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub selected_model: Option<String>,
    pub selected_effort: Option<String>,
    pub runtime_model: Option<String>,
    pub runtime_effort: Option<String>,
    pub provider_model: Option<String>,
    pub evidence: String,
    pub details: Option<String>,
}

impl Observation {
    pub fn runtime_model_comparable(&self) -> bool {
        self.selected_model.is_some() && self.runtime_model.is_some()
    }

    pub fn runtime_effort_comparable(&self) -> bool {
        self.selected_effort.is_some() && self.runtime_effort.is_some()
    }

    pub fn runtime_model_mismatch(&self) -> bool {
        matches!((&self.selected_model, &self.runtime_model), (Some(a), Some(b)) if a != b)
    }

    pub fn runtime_effort_mismatch(&self) -> bool {
        matches!((&self.selected_effort, &self.runtime_effort), (Some(a), Some(b)) if a != b)
    }

    pub fn provider_model_mismatch(&self) -> bool {
        matches!((&self.selected_model, &self.provider_model), (Some(a), Some(b)) if a != b)
    }

    fn detail_is(&self, key: &str, expected: &str) -> bool {
        self.details
            .as_deref()
            .and_then(|details| serde_json::from_str::<serde_json::Value>(details).ok())
            .and_then(|details| {
                details
                    .get(key)
                    .and_then(|value| value.as_str())
                    .map(|value| value == expected)
            })
            .unwrap_or(false)
    }

    fn detail_bool(&self, key: &str) -> bool {
        self.details
            .as_deref()
            .and_then(|details| serde_json::from_str::<serde_json::Value>(details).ok())
            .and_then(|details| details.get(key).and_then(|value| value.as_bool()))
            .unwrap_or(false)
    }

    pub fn has_turn_context(&self) -> bool {
        self.detail_is("runtime_evidence", "turn_context")
            || self
                .evidence
                .split(" + ")
                .any(|evidence| evidence == "turn_context")
    }

    pub fn provider_model_comparable(&self) -> bool {
        self.has_turn_context() && self.selected_model.is_some() && self.provider_model.is_some()
    }

    pub fn provider_model_mismatch_authoritative(&self) -> bool {
        self.provider_model_comparable() && self.provider_model_mismatch()
    }

    pub fn provider_was_rerouted(&self) -> bool {
        self.provider_model.is_some()
            && (self.detail_bool("provider_rerouted")
                || self.detail_is("provider_evidence", "model/rerouted")
                || self
                    .evidence
                    .split(" + ")
                    .any(|evidence| evidence == "model/rerouted"))
    }

    fn has_provider_alert(&self) -> bool {
        self.provider_model_mismatch_authoritative() || self.provider_was_rerouted()
    }

    pub fn has_runtime_mismatch(&self) -> bool {
        self.runtime_model_mismatch() || self.runtime_effort_mismatch()
    }

    pub fn has_any_mismatch_or_reroute(&self) -> bool {
        self.has_runtime_mismatch() || self.has_provider_alert()
    }

    pub fn has_new_alert_since(&self, before: &Observation) -> bool {
        (!before.runtime_model_mismatch() && self.runtime_model_mismatch())
            || (!before.runtime_effort_mismatch() && self.runtime_effort_mismatch())
            || (!before.has_provider_alert() && self.has_provider_alert())
    }

    fn probe_failed(&self) -> bool {
        self.kind == "Probe"
            && self
                .details
                .as_deref()
                .and_then(|details| serde_json::from_str::<serde_json::Value>(details).ok())
                .and_then(|details| {
                    details
                        .get("status")
                        .and_then(|status| status.as_str())
                        .map(str::to_owned)
                })
                .as_deref()
                == Some("failed")
    }

    fn runtime_result(&self) -> Option<&'static str> {
        let model = self
            .runtime_model_comparable()
            .then(|| !self.runtime_model_mismatch());
        let effort = self
            .runtime_effort_comparable()
            .then(|| !self.runtime_effort_mismatch());
        match (model, effort) {
            (Some(true), Some(true)) => Some("Runtime match"),
            (Some(false), Some(false)) => Some("Runtime model + effort mismatch"),
            (Some(false), Some(true)) => Some("Runtime model mismatch"),
            (Some(true), Some(false)) => Some("Runtime effort mismatch"),
            (Some(true), None) => Some("Runtime model match · effort not observed"),
            (Some(false), None) => Some("Runtime model mismatch · effort not observed"),
            (None, Some(true)) => Some("Runtime effort match · model not observed"),
            (None, Some(false)) => Some("Runtime effort mismatch · model not observed"),
            (None, None) => None,
        }
    }

    pub fn result(&self) -> String {
        if self.probe_failed() {
            return "Probe failed".into();
        }
        if self.kind == "Probe" {
            return match (&self.selected_model, &self.provider_model) {
                (Some(a), Some(b)) if a == b => "Provider match",
                (Some(_), Some(_)) => "Provider mismatch",
                _ => "Provider not observed",
            }
            .into();
        }
        let provider = self.provider_model.as_ref().map(|provider| {
            if !self.provider_model_comparable() {
                if self.provider_was_rerouted() {
                    "Provider reroute"
                } else {
                    "Provider observed"
                }
            } else {
                match (&self.selected_model, self.provider_was_rerouted()) {
                    (Some(selected), true) if selected == provider => {
                        "Provider match · reroute observed"
                    }
                    (Some(_), true) => "Provider mismatch · reroute observed",
                    (Some(selected), false) if selected == provider => "Provider match",
                    (Some(_), false) => "Provider mismatch",
                    (None, _) => "Provider observed",
                }
            }
        });
        match (provider, self.runtime_result()) {
            (Some(provider), Some(runtime)) => format!("{provider} · {runtime}"),
            (Some(provider), None) => provider.into(),
            (None, Some(runtime)) => runtime.into(),
            (None, None) => "Incomplete".into(),
        }
    }
}
