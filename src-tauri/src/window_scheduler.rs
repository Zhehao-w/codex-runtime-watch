//! Opt-in macOS 5-hour window scheduler. No model calls happen while disabled.
//! Uses the bundled Codex CLI: rate-limit reads do not consume model tokens.
use crate::codex::rate_limits::{read_limits, Limits};
use chrono::{DateTime, Local, Timelike, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const PING_TIMEOUT: Duration = Duration::from_secs(120);
const BACKOFF_SECONDS: i64 = 30 * 60;
const AGENT_LABEL: &str = "dev.codexruntimewatch.window-scheduler";

#[derive(Debug, Serialize)]
pub struct SchedulerReport {
    pub action: String,
    pub detail: String,
    pub next_reset: Option<i64>,
    pub primary_used_percent: Option<f64>,
    pub weekly_used_percent: Option<f64>,
}

impl SchedulerReport {
    fn new(action: &str, detail: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            detail: detail.into(),
            next_reset: None,
            primary_used_percent: None,
            weekly_used_percent: None,
        }
    }
}

fn attach_limits(limits: &Limits, report: &mut SchedulerReport) {
    report.next_reset = Some(limits.primary_reset);
    report.primary_used_percent = Some(limits.primary_used);
    report.weekly_used_percent = Some(limits.weekly_used);
}

/// A Mac on battery should never initiate an automatic model request.
fn on_ac_power() -> bool {
    Command::new("/usr/bin/pmset")
        .args(["-g", "batt"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| String::from_utf8_lossy(&output.stdout).contains("AC Power"))
}

fn cli_path() -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        for name in ["ChatGPT.app", "Codex.app"] {
            candidates.push(
                home.join("Applications")
                    .join(name)
                    .join("Contents/Resources/codex-cli/bin/codex"),
            );
        }
    }
    for name in ["ChatGPT.app", "Codex.app"] {
        candidates.push(
            Path::new("/Applications")
                .join(name)
                .join("Contents/Resources/codex-cli/bin/codex"),
        );
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            candidates.push(dir.join("codex"));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "Codex CLI not found (checked Desktop bundled CLI and PATH)".into())
}

fn ensure_chatgpt_auth(codex_home: &Path) -> Result<(), String> {
    // Do not ever fall through to pay-per-token API key authentication.
    let bytes = fs::read(codex_home.join("auth.json"))
        .map_err(|_| "ChatGPT auth.json unavailable; automatic usage is disabled".to_string())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "Invalid auth.json")?;
    if value.get("auth_mode").and_then(Value::as_str) != Some("chatgpt") {
        return Err("Only ChatGPT-plan authentication is allowed".into());
    }
    Ok(())
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Keep the Mac from IDLE sleeping for the entire check/request lifecycle.
/// Dropping this guard releases the assertion even after an error or timeout.
struct IdleSleepGuard(Child);

impl IdleSleepGuard {
    fn start() -> Result<Self, String> {
        let child = Command::new("/usr/bin/caffeinate")
            .arg("-i")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Cannot hold macOS idle-sleep assertion: {e}"))?;
        Ok(Self(child))
    }
}

impl Drop for IdleSleepGuard {
    fn drop(&mut self) {
        stop_child(&mut self.0);
    }
}

/// User-level code may publish ONLY a reset timestamp, never a privileged
/// command. The root wake helper checks ownership, numerical bounds,
/// power source, and local allowed hours independently before using it.
fn publish_next_reset(app_dir: &Path, reset: i64, now: i64) -> Result<(), String> {
    let path = app_dir.join("window-next-reset");
    let local_minutes = chrono::DateTime::<Utc>::from_timestamp(reset, 0)
        .map(|stamp| stamp.with_timezone(&Local))
        .map(|stamp| stamp.hour() * 60 + stamp.minute());
    let eligible = app_dir.join("window-wake-enabled").is_file()
        && reset > now + 45
        && reset <= now + 6 * 3600
        && local_minutes.is_some_and(|m| (420..=1350).contains(&m));
    if !eligible {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Cannot clear next-reset wake: {e}")),
        }
        return Ok(());
    }
    fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
    let temp = path.with_extension("tmp");
    fs::write(&temp, format!("{reset}\n"))
        .and_then(|_| fs::rename(temp, path))
        .map_err(|e| format!("Cannot publish next-reset wake: {e}"))
}

fn allowed_time(now: DateTime<Local>, reset: i64) -> bool {
    let minute = now.hour() * 60 + now.minute();
    if !(420..=1350).contains(&minute) {
        return false; // 07:00 through 22:30
    }
    let in_anchor = [420_u32, 720, 1020, 1320]
        .into_iter()
        .any(|anchor| minute >= anchor && minute <= anchor + 30);
    // Follow real (possibly delayed) reset times, without replaying old slots.
    let just_reset = reset > 0 && now.timestamp() >= reset && now.timestamp() - reset <= 15 * 60;
    in_anchor || just_reset
}

fn state_path(app_dir: &Path) -> PathBuf {
    app_dir.join("window-scheduler-state.json")
}

fn recent_attempt(app_dir: &Path, now: i64) -> bool {
    fs::read(state_path(app_dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.get("attempt").and_then(Value::as_i64))
        .is_some_and(|attempt| now >= attempt && now - attempt < BACKOFF_SECONDS)
}

fn save_attempt(app_dir: &Path, action: &str, detail: &str) -> Result<(), String> {
    fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
    let path = state_path(app_dir);
    let tmp = path.with_extension("tmp");
    let bytes = serde_json::to_vec(&json!({
        "attempt": Utc::now().timestamp(),
        "action": action, "detail": detail
    }))
    .map_err(|e| e.to_string())?;
    fs::write(&tmp, bytes)
        .and_then(|_| fs::rename(tmp, path))
        .map_err(|e| format!("Cannot save scheduler state: {e}"))
}

struct TickLock(PathBuf);
impl Drop for TickLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn acquire_lock(app_dir: &Path) -> Result<Option<TickLock>, String> {
    fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
    let path = app_dir.join("window-scheduler.lock");
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{}", std::process::id());
            Ok(Some(TickLock(path)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // A crashed process may leave a lock. Never race another live tick:
            // only reclaim it after 10 minutes, longer than the RPC timeout.
            let stale = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(600));
            if stale {
                let _ = fs::remove_file(path);
            }
            Ok(None)
        }
        Err(e) => Err(format!("Scheduler lock failed: {e}")),
    }
}

fn ping(cli: &Path, model: &str, codex_home: &Path) -> Result<(), String> {
    let mut child = Command::new(cli)
        .args([
            "exec",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--skip-git-repo-check",
            "-s",
            "read-only",
            "-m",
            model,
            "-c",
            "model_reasoning_effort=low",
            "--json",
            "Reply with exactly OK. Do not use tools.",
        ])
        .current_dir(std::env::temp_dir())
        .env("CODEX_HOME", codex_home)
        .env_remove("OPENAI_API_KEY")
        .env_remove("CODEX_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot launch Codex CLI: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("Codex exited with {status}")),
            Ok(None) if start.elapsed() < PING_TIMEOUT => thread::sleep(Duration::from_millis(250)),
            Ok(None) => {
                stop_child(&mut child);
                return Err("Codex ping timed out".into());
            }
            Err(e) => {
                stop_child(&mut child);
                return Err(e.to_string());
            }
        }
    }
}

pub fn tick(
    enabled: bool,
    model: &str,
    app_dir: &Path,
    codex_home: &Path,
    dry_run: bool,
) -> SchedulerReport {
    if !enabled && !dry_run {
        return SchedulerReport::new("disabled", "Scheduler is off; monitoring remains passive");
    }
    let now = Local::now();
    if now.hour() < 7 || now.hour() > 22 || (now.hour() == 22 && now.minute() > 30) {
        return SchedulerReport::new("quiet-hours", "Outside 07:00–22:30 local-time schedule");
    }
    if !dry_run && recent_attempt(app_dir, now.timestamp()) {
        return SchedulerReport::new("backoff", "An attempt occurred in the last 30 minutes");
    }
    if model.trim().is_empty() || model.chars().any(char::is_control) {
        return SchedulerReport::new("error", "Invalid model name");
    }
    if !on_ac_power() {
        return SchedulerReport::new(
            "ac-required",
            "Automatic starts are disabled on battery power",
        );
    }
    if let Err(error) = ensure_chatgpt_auth(codex_home) {
        return SchedulerReport::new("auth-required", error);
    }
    let cli = match cli_path() {
        Ok(path) => path,
        Err(error) => return SchedulerReport::new("cli-missing", error),
    };
    let _sleep_guard = if dry_run {
        None
    } else {
        match IdleSleepGuard::start() {
            Ok(guard) => Some(guard),
            Err(error) => return SchedulerReport::new("sleep-guard-failed", error),
        }
    };
    let limits = match read_limits(&cli, codex_home) {
        Ok(value) => value,
        Err(error) => return SchedulerReport::new("rpc-failed", error),
    };
    if !dry_run {
        if let Err(error) = publish_next_reset(
            app_dir,
            if limits.weekly_used >= 95.0 {
                0
            } else {
                limits.primary_reset
            },
            now.timestamp(),
        ) {
            return SchedulerReport::new("wake-update-failed", error);
        }
    }
    let mut report = if limits.weekly_used >= 95.0 {
        SchedulerReport::new(
            "weekly-guard",
            "Weekly usage reached the 95% safety threshold",
        )
    } else if limits.primary_reset > now.timestamp() {
        SchedulerReport::new("active", "Existing 5-hour window is still active")
    } else if !allowed_time(now, limits.primary_reset) {
        SchedulerReport::new(
            "outside-slot",
            "Missed the 07/12/17/22 start slot; no catch-up",
        )
    } else if dry_run {
        SchedulerReport::new(
            "would-ping",
            "Eligible for a scheduled start; this dry run never sends requests",
        )
    } else {
        SchedulerReport::new("pending", "Ready for minimal Codex request")
    };
    attach_limits(&limits, &mut report);
    if report.action != "pending" {
        return report;
    }
    let _lock = match acquire_lock(app_dir) {
        Ok(Some(lock)) => lock,
        Ok(None) => return SchedulerReport::new("locked", "Another scheduler tick is running"),
        Err(error) => return SchedulerReport::new("error", error),
    };
    if recent_attempt(app_dir, now.timestamp()) {
        return SchedulerReport::new("backoff", "A recent tick has already attempted a request");
    }
    // Save before sending, preventing crash/restart from causing duplicate pings.
    if let Err(error) = save_attempt(app_dir, "attempt", "pending") {
        return SchedulerReport::new("error", error);
    }
    let result = ping(&cli, model, codex_home).and_then(|_| {
        let after = read_limits(&cli, codex_home)?;
        if after.primary_reset <= Utc::now().timestamp() {
            return Err("Codex completed but a new window was not confirmed".into());
        }
        Ok(after)
    });
    match result {
        Ok(after) => {
            if let Err(error) =
                publish_next_reset(app_dir, after.primary_reset, Utc::now().timestamp())
            {
                let _ = save_attempt(
                    app_dir,
                    "started",
                    "Window started; dynamic wake update failed",
                );
                let mut report = SchedulerReport::new("started-wake-warning", error);
                attach_limits(&after, &mut report);
                return report;
            }
            let _ = save_attempt(app_dir, "started", "New 5-hour window confirmed");
            let mut report = SchedulerReport::new("started", "New 5-hour window confirmed");
            attach_limits(&after, &mut report);
            report
        }
        Err(error) => {
            let _ = save_attempt(app_dir, "failed", "Codex request failed; backed off");
            SchedulerReport::new("failed", error)
        }
    }
}

fn agent_plist_path() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|h| {
            h.join("Library/LaunchAgents")
                .join(format!("{AGENT_LABEL}.plist"))
        })
        .ok_or("Unable to find home directory".into())
}

fn launchctl_uid() -> Result<String, String> {
    let output = Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map_err(|e| format!("Cannot determine UID: {e}"))?;
    if !output.status.success() {
        return Err("Unable to determine launchd user domain".into());
    }
    let uid = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let uid = uid.trim().to_owned();
    if uid.is_empty() || !uid.chars().all(|c| c.is_ascii_digit()) {
        return Err("Invalid launchd UID".into());
    }
    Ok(uid)
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Manage only our own unprivileged per-user LaunchAgent. This does NOT
/// schedule macOS wake events or change power-management settings.
fn launchctl_loaded(service: &str) -> bool {
    Command::new("/bin/launchctl")
        .args(["print", service])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn launchctl_bootstrap(domain: &str, plist: &Path) -> Result<(), String> {
    let output = Command::new("/bin/launchctl")
        .arg("bootstrap")
        .arg(domain)
        .arg(plist)
        .output()
        .map_err(|e| format!("Cannot launch scheduler LaunchAgent: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "launchctl bootstrap failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// Stage and validate updates before stopping the previous agent.
/// Restores the old plist and running service on replacement failure.
pub fn sync_launch_agent(enabled: bool) -> Result<(), String> {
    let plist = agent_plist_path()?;
    let domain = format!("gui/{}", launchctl_uid()?);
    let service = format!("{domain}/{AGENT_LABEL}");
    let old_content = match fs::read(&plist) {
        Ok(content) => Some(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Cannot inspect scheduler LaunchAgent: {e}")),
    };
    let loaded = launchctl_loaded(&service);
    if !enabled {
        if loaded {
            let output = Command::new("/bin/launchctl")
                .args(["bootout", &service])
                .output()
                .map_err(|e| format!("Cannot unload scheduler LaunchAgent: {e}"))?;
            if !output.status.success() {
                return Err(format!(
                    "Cannot unload scheduler LaunchAgent: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
        if let Err(e) = fs::remove_file(&plist) {
            if e.kind() != std::io::ErrorKind::NotFound {
                if loaded {
                    let _ = launchctl_bootstrap(&domain, &plist);
                }
                return Err(format!("Cannot remove scheduler LaunchAgent: {e}"));
            }
        }
        return Ok(());
    }

    let exe = std::env::current_exe().map_err(|e| format!("Cannot locate app executable: {e}"))?;
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{AGENT_LABEL}</string>
<key>ProgramArguments</key><array><string>{}</string><string>--window-scheduler-tick</string></array>
<key>StartInterval</key><integer>60</integer>
<key>RunAtLoad</key><true/>
<key>StandardOutPath</key><string>/dev/null</string>
<key>StandardErrorPath</key><string>/dev/null</string>
</dict></plist>"#,
        xml_escape(&exe.to_string_lossy())
    );
    if old_content.as_deref() == Some(content.as_bytes()) {
        return if loaded { Ok(()) } else { launchctl_bootstrap(&domain, &plist) };
    }
    if let Some(parent) = plist.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temp = plist.with_extension("tmp");
    fs::write(&temp, content).map_err(|e| format!("Cannot stage scheduler LaunchAgent: {e}"))?;
    let validated = Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(&temp)
        .output()
        .map_err(|e| format!("Cannot validate scheduler LaunchAgent: {e}"))?;
    if !validated.status.success() {
        let _ = fs::remove_file(&temp);
        return Err(format!(
            "Invalid scheduler LaunchAgent: {}",
            String::from_utf8_lossy(&validated.stderr)
        ));
    }
    if loaded {
        let output = Command::new("/bin/launchctl")
            .args(["bootout", &service])
            .output()
            .map_err(|e| format!("Cannot unload scheduler LaunchAgent: {e}"))?;
        if !output.status.success() {
            let _ = fs::remove_file(&temp);
            return Err(format!("Cannot unload scheduler LaunchAgent: {}",
                String::from_utf8_lossy(&output.stderr)));
        }
    }
    if let Err(e) = fs::rename(&temp, &plist) {
        if loaded {
            let _ = launchctl_bootstrap(&domain, &plist);
        }
        return Err(format!("Cannot activate staged scheduler LaunchAgent: {e}"));
    }
    if let Err(start_error) = launchctl_bootstrap(&domain, &plist) {
        let restored = if let Some(previous) = old_content {
            fs::write(&temp, previous)
                .and_then(|_| fs::rename(&temp, &plist))
                .map_err(|e| format!("Cannot restore previous LaunchAgent: {e}"))
        } else {
            fs::remove_file(&plist)
                .map_err(|e| format!("Cannot remove failed LaunchAgent: {e}"))
        };
        let restart = if loaded && restored.is_ok() {
            launchctl_bootstrap(&domain, &plist)
        } else {
            Ok(())
        };
        return Err(format!(
            "{start_error}; restore: {restored:?}; previous agent restart: {restart:?}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn slots_and_night_pause() {
        let date = Local
            .with_ymd_and_hms(2026, 10, 13, 7, 0, 0)
            .single()
            .unwrap();
        assert!(allowed_time(date, 0));
        let noon = Local
            .with_ymd_and_hms(2026, 10, 13, 12, 10, 0)
            .single()
            .unwrap();
        assert!(allowed_time(noon, 0));
        let late = Local
            .with_ymd_and_hms(2026, 10, 13, 9, 0, 0)
            .single()
            .unwrap();
        assert!(!allowed_time(late, 0));
        let night = Local
            .with_ymd_and_hms(2026, 10, 14, 3, 0, 0)
            .single()
            .unwrap();
        assert!(!allowed_time(night, night.timestamp()));
        let reset = Local
            .with_ymd_and_hms(2026, 10, 13, 12, 34, 0)
            .single()
            .unwrap();
        assert!(allowed_time(reset, reset.timestamp()));
    }

    #[test]
    fn disabled_never_requests() {
        let report = tick(false, "gpt-5.6-luna", Path::new("/unused"), Path::new("/unused"), false);
        assert_eq!(report.action, "disabled");
    }

    #[test]
    fn dynamic_wake_follows_server_reset_and_enforces_daytime() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("window-wake-enabled"), b"enabled").unwrap();

        let noon = Local
            .with_ymd_and_hms(2026, 10, 13, 12, 3, 0)
            .single()
            .unwrap();
        publish_next_reset(dir.path(), noon.timestamp(), noon.timestamp() - 3600).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("window-next-reset")).unwrap(),
            format!("{}\n", noon.timestamp())
        );

        let night = Local
            .with_ymd_and_hms(2026, 10, 13, 23, 0, 0)
            .single()
            .unwrap();
        publish_next_reset(dir.path(), night.timestamp(), night.timestamp() - 3600).unwrap();
        assert!(!dir.path().join("window-next-reset").exists());
    }

    #[test]
    fn dynamic_wake_requires_explicit_marker_and_recent_future_reset() {
        let dir = tempfile::tempdir().unwrap();
        let reset = Local
            .with_ymd_and_hms(2026, 10, 13, 17, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        publish_next_reset(dir.path(), reset, reset - 3600).unwrap();
        assert!(!dir.path().join("window-next-reset").exists());
        fs::write(dir.path().join("window-wake-enabled"), b"enabled").unwrap();
        publish_next_reset(dir.path(), reset, reset - 7 * 3600).unwrap();
        assert!(!dir.path().join("window-next-reset").exists());
        publish_next_reset(dir.path(), reset, reset - 45).unwrap();
        assert!(!dir.path().join("window-next-reset").exists());
    }
}
