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

    pub fn has_runtime_mismatch(&self) -> bool {
        self.runtime_model_mismatch() || self.runtime_effort_mismatch()
    }

    pub fn has_any_mismatch_or_reroute(&self) -> bool {
        self.has_runtime_mismatch()
            || self.provider_model_mismatch()
            || (self.provider_model.is_some() && self.selected_model.is_none())
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
        if !self.runtime_model_comparable() {
            return None;
        }
        Some(
            match (
                self.runtime_model_mismatch(),
                self.runtime_effort_mismatch(),
                self.runtime_effort_comparable(),
            ) {
                (true, true, _) => "Runtime model + effort mismatch",
                (true, false, _) => "Runtime model mismatch",
                (false, true, _) => "Runtime effort mismatch",
                (false, false, true) => "Runtime match",
                (false, false, false) => "Runtime model match · effort not observed",
            },
        )
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
        let provider = self
            .provider_model
            .as_ref()
            .map(|provider| match &self.selected_model {
                Some(selected) if selected == provider => "Provider match",
                Some(_) => "Provider mismatch",
                None => "Provider reroute",
            });
        match (provider, self.runtime_result()) {
            (Some(provider), Some(runtime)) => format!("{provider} · {runtime}"),
            (Some(provider), None) => provider.into(),
            (None, Some(runtime)) => runtime.into(),
            (None, None) => "Incomplete".into(),
        }
    }
}
