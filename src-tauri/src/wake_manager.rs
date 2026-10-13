//! A separately approved, narrowly-scoped root wake helper for macOS.
//! The root process ONLY runs pmset for fixed wake times. It never reads Codex
//! credentials, calls OpenAI, or executes commands from user-writable files.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const LABEL: &str = "dev.codexruntimewatch.wake-helper";
const ROOT_BIN: &str = "/Library/PrivilegedHelperTools/dev.codexruntimewatch.wake-helper.sh";
const ROOT_PLIST: &str = "/Library/LaunchDaemons/dev.codexruntimewatch.wake-helper.plist";
const WAKE_OWNER: &str = "dev.codexruntimewatch.wake-helper";
const HELPER_CONTENT: &str = include_str!("../resources/wake-helper.sh");

#[derive(Debug, Serialize)]
pub struct WakeStatus {
    pub supported: bool,
    pub installed: bool,
    pub enabled: bool,
    pub owned_events: usize,
    pub detail: String,
}

pub fn marker_path(app_dir: &Path) -> PathBuf {
    app_dir.join("window-wake-enabled")
}

pub fn set_marker(app_dir: &Path, enabled: bool) -> Result<(), String> {
    let marker = marker_path(app_dir);
    if enabled {
        fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
        fs::write(marker, b"enabled\n").map_err(|e| e.to_string())
    } else {
        match fs::remove_file(marker) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

pub fn status(app_dir: &Path) -> WakeStatus {
    let installed = Path::new(ROOT_BIN).is_file() && Path::new(ROOT_PLIST).is_file();
    let output = Command::new("/usr/bin/pmset").args(["-g", "sched"]).output();
    let (owned_events, detail) = match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            (text.lines().filter(|line| line.contains(WAKE_OWNER) && line.contains("wake at")).count(),
             if installed { "Wake helper installed; fixed daily wake times are 06:58, 11:58, 16:58 and 21:58" }
             else { "Wake helper is not installed; scheduled Codex checks cannot wake a sleeping Mac" }.to_owned())
        }
        _ => (0, "Unable to read macOS wake schedule".into()),
    };
    WakeStatus { supported: true, installed, enabled: marker_path(app_dir).is_file(), owned_events, detail }
}

fn xml_escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;")
        .replace('>', "&gt;").replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn applescript_quote(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn validated_marker(app_dir: &Path) -> Result<String, String> {
    let marker = marker_path(app_dir).to_string_lossy().into_owned();
    if !marker.starts_with("/Users/")
        || !marker.ends_with("/Library/Application Support/dev.codexruntimewatch.app/window-wake-enabled")
    {
        return Err("Unexpected macOS application data path".into());
    }
    Ok(marker)
}

fn plist(marker: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array>
<string>/bin/sh</string><string>{ROOT_BIN}</string>
<string>run</string><string>{}</string>
</array>
<key>RunAtLoad</key><true/>
<key>StartInterval</key><integer>60</integer>
<key>StandardOutPath</key><string>/dev/null</string>
<key>StandardErrorPath</key><string>/dev/null</string>
</dict></plist>"#,
        xml_escape(marker)
    )
}

/// A one-time OS administrator prompt, ONLY when the user clicks Install.
/// The installed root-owned code and launchd configuration are fixed strings,
/// not mutable scripts or commands supplied by the frontend.
pub fn install(app_dir: &Path) -> Result<(), String> {
    let marker = validated_marker(app_dir)?;
    let helper_b64 = STANDARD.encode(HELPER_CONTENT);
    let plist_b64 = STANDARD.encode(plist(&marker));
    let service = format!("system/{LABEL}");
    let command = format!(
        "/bin/launchctl bootout {} >/dev/null 2>&1 || true; \
         /usr/bin/printf %s {} | /usr/bin/base64 -D > {}; \
         /usr/sbin/chown root:wheel {}; /bin/chmod 0755 {}; \
         /usr/bin/printf %s {} | /usr/bin/base64 -D > {}; \
         /usr/sbin/chown root:wheel {}; /bin/chmod 0644 {}; \
         /bin/launchctl bootstrap system {}",
        shell_quote(&service),
        shell_quote(&helper_b64), shell_quote(ROOT_BIN),
        shell_quote(ROOT_BIN), shell_quote(ROOT_BIN),
        shell_quote(&plist_b64), shell_quote(ROOT_PLIST),
        shell_quote(ROOT_PLIST), shell_quote(ROOT_PLIST),
        shell_quote(ROOT_PLIST)
    );
    run_admin(&command)
}

/// Removes only our daemon and events; other pmset schedules remain intact.
pub fn uninstall(app_dir: &Path) -> Result<(), String> {
    let marker = validated_marker(app_dir)?;
    let command = format!(
        "/bin/launchctl bootout {} >/dev/null 2>&1 || true; \
         if test -f {}; then /bin/sh {} remove {}; fi; \
         /bin/rm -f {} {}",
        shell_quote(&format!("system/{LABEL}")),
        shell_quote(ROOT_BIN), shell_quote(ROOT_BIN), shell_quote(&marker),
        shell_quote(ROOT_BIN), shell_quote(ROOT_PLIST)
    );
    run_admin(&command)?;
    set_marker(app_dir, false)
}

fn run_admin(command: &str) -> Result<(), String> {
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        applescript_quote(command)
    );
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output().map_err(|e| format!("Cannot request macOS administrator approval: {e}"))?;
    if !output.status.success() {
        return Err(format!("Administrator authorization cancelled or failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_inputs_do_not_break_shell_or_xml() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(xml_escape("a&<b>"), "a&amp;&lt;b&gt;");
    }

    #[test]
    fn wake_plist_has_no_model_or_network_execution() {
        let xml = plist("/Users/test/Library/Application Support/dev.codexruntimewatch.app/window-wake-enabled");
        assert!(xml.contains("<key>StartInterval</key><integer>60</integer>"));
        assert!(!xml.contains("codex-cli"));
        assert!(!xml.contains("OPENAI"));
        assert!(xml.contains(ROOT_BIN));
    }

    #[test]
    fn unexpected_root_marker_rejected() {
        assert!(validated_marker(Path::new("/tmp/elsewhere")).is_err());
        assert!(validated_marker(Path::new("/Users/test/Library/Application Support/dev.codexruntimewatch.app")).is_ok());
    }
}
