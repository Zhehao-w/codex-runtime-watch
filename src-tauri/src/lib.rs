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
    pub fn result(&self) -> &'static str {
        if self.kind == "Probe"
            && self
                .details
                .as_deref()
                .is_some_and(|d| d.contains("probe_error"))
        {
            return "Probe failed";
        }
        if let Some(provider) = &self.provider_model {
            return match &self.selected_model {
                Some(selected) if selected == provider => "Provider match",
                Some(_) => "Provider mismatch",
                None => "Provider reroute",
            };
        }
        match (&self.selected_model, &self.runtime_model) {
            (Some(sm), Some(rm)) => match (sm == rm, self.selected_effort == self.runtime_effort) {
                (true, true) => "Runtime match",
                (false, true) => "Runtime model mismatch",
                (true, false) => "Runtime effort mismatch",
                (false, false) => "Runtime model + effort mismatch",
            },
            _ => {
                if self.kind == "Probe" {
                    "Provider not observed"
                } else {
                    "Incomplete"
                }
            }
        }
    }
}
