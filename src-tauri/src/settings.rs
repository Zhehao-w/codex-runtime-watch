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
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            codex_home: None,
            notify_mismatch: true,
            start_at_login: false,
            theme: "system".into(),
            initial_scan_days: 7,
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
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?
        };
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, path).context("replace settings")
    }
}
