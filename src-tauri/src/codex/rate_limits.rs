//! Upstream Codex app-server rate-limit RPC and its response schema.
//! Keep protocol knowledge here, not in scheduler policy code.
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
const RPC_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct Limits {
    pub primary_reset: i64,
    pub primary_used: f64,
    pub weekly_used: f64,
    pub reset_credits_available: u64,
}

impl Limits {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let limits = value
            .get("result")
            .and_then(|v| v.get("rateLimits"))
            .ok_or("Codex did not return rateLimits")?;
        // Always verify the banked reset-credit state. Without a verified
        // zero count, an automatic ping could redeem a stored reset.
        // This field belongs to result, not to result.rateLimits.
        let reset_credits_available = value
            .get("result")
            .and_then(|result| result.get("rateLimitResetCredits"))
            .and_then(|credits| credits.get("availableCount"))
            .and_then(Value::as_u64)
            .ok_or("Reset-credit availability is missing or invalid; skipping automatic usage")?;
        let primary = limits.get("primary").ok_or("Missing primary window")?;
        let secondary = limits.get("secondary").ok_or("Missing weekly window")?;
        let primary_reset = primary
            .get("resetsAt")
            .and_then(Value::as_i64)
            .ok_or("Missing primary reset timestamp")?;
        let primary_used = primary
            .get("usedPercent")
            .and_then(Value::as_f64)
            .ok_or("Missing primary usage")?;
        let weekly_used = secondary
            .get("usedPercent")
            .and_then(Value::as_f64)
            .ok_or("Missing weekly usage")?;
        if !primary_used.is_finite()
            || !weekly_used.is_finite()
            || !(0.0..=100.0).contains(&primary_used)
            || !(0.0..=100.0).contains(&weekly_used)
        {
            return Err("Invalid usage percentages".into());
        }
        Ok(Self {
            primary_reset,
            primary_used,
            weekly_used,
            reset_credits_available,
        })
    }
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub fn read_limits(cli: &Path, codex_home: &Path) -> Result<Limits, String> {
    let mut child = Command::new(cli)
        .arg("app-server")
        .env("CODEX_HOME", codex_home)
        .env_remove("OPENAI_API_KEY")
        .env_remove("CODEX_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Unable to launch app-server: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("Missing app-server stdin")?;
    let stdout = child.stdout.take().ok_or("Missing app-server stdout")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line.len() > 256 * 1024 {
                        break;
                    }
                    if let Ok(value) = serde_json::from_str::<Value>(&line) {
                        if value.get("id").and_then(Value::as_i64) == Some(2) {
                            let _ = tx.send(value);
                            break;
                        }
                    }
                }
            }
        }
    });
    let request = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"codex-runtime-watch\",\"title\":\"Codex Runtime Watch\",\"version\":\"0.1.1\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"account/rateLimits/read\",\"params\":{\"excludeResetCreditDetails\":false}}\n"
    );
    if let Err(e) = stdin
        .write_all(request.as_bytes())
        .and_then(|_| stdin.flush())
    {
        stop_child(&mut child);
        return Err(format!("app-server request failed: {e}"));
    }
    // Keep stdin open until the asynchronous id=2 response arrives.
    let response = rx.recv_timeout(RPC_TIMEOUT);
    drop(stdin);
    stop_child(&mut child);
    let value = response.map_err(|_| "Timed out reading Codex rate limits")?;
    if let Some(error) = value.get("error") {
        return Err(format!("Codex rate-limit RPC error: {error}"));
    }
    Limits::parse(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rejects_incomplete_or_invalid_limits() {
        let valid = json!({"result":{
            "rateLimits":{"primary":{"resetsAt":123,"usedPercent":0.0},"secondary":{"usedPercent":50.0}},
            "rateLimitResetCredits":{"availableCount":0}
        }});
        let parsed = Limits::parse(&valid).unwrap();
        assert_eq!(parsed.weekly_used, 50.0);
        assert_eq!(parsed.reset_credits_available, 0);
        assert!(Limits::parse(&json!({"result":{"rateLimits":{
            "primary":{"resetsAt":123,"usedPercent":0},
            "secondary":{"usedPercent":50}
        }}})).is_err());
        assert!(Limits::parse(&json!({"result":{
            "rateLimits":{"primary":{"resetsAt":123,"usedPercent":0},"secondary":{"usedPercent":101}},
            "rateLimitResetCredits":{"availableCount":0}
        }})).is_err());
    }

    #[test]
    fn reset_credit_count_must_be_explicit_and_nonnegative() {
        let data = json!({"result":{
            "rateLimits":{"primary":{"resetsAt":123,"usedPercent":0.0},"secondary":{"usedPercent":50.0}},
            "rateLimitResetCredits":{"availableCount":2}
        }});
        assert_eq!(Limits::parse(&data).unwrap().reset_credits_available, 2);
        for invalid in [Value::Null, json!({}), json!({"availableCount":null}),
                        json!({"availableCount":-1}), json!({"availableCount":"0"})] {
            let mut value = data.clone();
            value["result"]["rateLimitResetCredits"] = invalid;
            assert!(Limits::parse(&value).is_err());
        }
    }
}
