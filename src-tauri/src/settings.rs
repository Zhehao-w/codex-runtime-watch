use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub codex_home: Option<String>,
    pub notify_mismatch: bool,
    pub start_at_login: bool,
    pub theme: String,
    pub initial_scan_days: u32,
    pub scheduler_enabled: bool,
    pub scheduler_wake_enabled: bool,
    pub scheduler_model: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            codex_home: None,
            notify_mismatch: true,
            start_at_login: false,
            theme: "system".into(),
            initial_scan_days: 7,
            scheduler_enabled: false,
            scheduler_wake_enabled: false,
            scheduler_model: "gpt-5.6-luna".into(),
        }
    }
}
impl Settings {
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if !["system", "light", "dark"].contains(&self.theme.as_str()) {
            anyhow::bail!("theme must be system, light, or dark")
        };
        if !(1..=365).contains(&self.initial_scan_days) {
            anyhow::bail!("initial_scan_days must be between 1 and 365")
        };
        if self.scheduler_model.trim().is_empty()
            || self.scheduler_model.chars().any(char::is_control)
        {
            anyhow::bail!("scheduler model must be a nonempty model identifier");
        }
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?
        };
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, path).context("replace settings")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_open_ended_model_identifiers() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            scheduler_model: format!("org/vendor:experimental/{}", "v".repeat(120)),
            ..Settings::default()
        };
        settings.save(&dir.path().join("settings.json")).unwrap();
        assert_eq!(Settings::load(&dir.path().join("settings.json")), settings);
    }
    #[test]
    fn rejects_blank_or_control_character_model_identifiers() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings {
            scheduler_model: "  ".into(),
            ..Settings::default()
        };
        assert!(settings.save(&dir.path().join("settings.json")).is_err());
        settings.scheduler_model = "valid\\ninvalid".replace("\\n", "\n");
        assert!(settings.save(&dir.path().join("settings.json")).is_err());
    }
}
