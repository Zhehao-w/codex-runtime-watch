//! The Codex CLI invocation contract for one minimal ordinary-plan request.
//! Scheduler policy must not embed CLI flags or prompts.
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const REQUEST_TEXT: &str = "Reply with exactly OK. Do not use tools.";

fn ordinary_window_args(model: &str) -> [&str; 13] {
    [
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
        REQUEST_TEXT,
    ]
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Send one minimal request using ordinary ChatGPT-plan authentication.
/// The caller must explicitly verify ordinaryUsageAllowed and check power
/// immediately before invoking this function. No reset-credit redemption
/// method, fallback opt-in or API key is ever supplied here.
pub fn start_ordinary_window(cli: &Path, model: &str, codex_home: &Path) -> Result<(), String> {
    let mut child = Command::new(cli)
        .args(ordinary_window_args(model))
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
            Ok(None) if start.elapsed() < REQUEST_TIMEOUT => {
                thread::sleep(Duration::from_millis(250))
            }
            Ok(None) => {
                stop_child(&mut child);
                return Err("Codex request timed out".into());
            }
            Err(error) => {
                stop_child(&mut child);
                return Err(error.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_request_is_minimal_and_has_no_credit_redemption() {
        let args = ordinary_window_args("org/model:future");
        assert_eq!(args[0], "exec");
        assert_eq!(args[8], "org/model:future");
        assert!(args.contains(&"--ephemeral"));
        assert!(args.contains(&"read-only"));
        assert!(args.contains(&REQUEST_TEXT));
        assert!(!args.iter().any(|part| {
            part.contains("reset-credit")
                || part.contains("redeem")
                || part.contains("luna_reserve")
        }));
    }
}
