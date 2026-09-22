#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evidence {
    pub time: Option<String>,
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub selected_model: Option<String>,
    pub selected_effort: Option<String>,
    pub runtime_model: Option<String>,
    pub runtime_effort: Option<String>,
    pub provider_model: Option<String>,
    pub source: String,
    pub parent_thread: Option<String>,
    pub is_subagent: bool,
    pub provider_reason: Option<String>,
}
