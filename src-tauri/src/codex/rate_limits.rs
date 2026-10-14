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

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub primary_reset: i64,
    pub primary_used: f64,
    pub weekly_used: f64,
    pub ordinary_usage_allowed: Option<bool>,
}

impl Limits {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let limits = value
            .get("result")
            .and_then(|v| v.get("rateLimits"))
            .ok_or("Codex did not return rateLimits")?;
        // The backend explicitly states whether ordinary, included usage is
        // allowed for this account. Null, absent or false must NEVER be
        // interpreted as a normal reset by inspecting percentages/timestamps.
        let ordinary_usage_allowed = value
            .get("result")
            .and_then(|result| result.get("ordinaryUsageAllowed"))
            .and_then(Value::as_bool);
        // Banked Reset Credits are never redeemed by this adapter. Their
        // existence or availability is irrelevant to ordinary quota use.
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
            ordinary_usage_allowed,
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
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"account/rateLimits/read\",\"params\":{\"supportsLunaReserve\":false,\"excludeResetCreditDetails\":true}}\n"
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

    fn ordinary_response(credits: Option<Value>) -> Value {
        let mut response = json!({"result":{
            "ordinaryUsageAllowed": true,
            "rateLimits":{
                "primary":{"resetsAt":123,"usedPercent":0.0},
                "secondary":{"usedPercent":50.0}
            }
        }});
        if let Some(credits) = credits {
            response["result"]["rateLimitResetCredits"] = credits;
        }
        response
    }

    #[test]
    fn ordinary_limits_parse_regardless_of_banked_reset_credits() {
        for credits in [
            None,
            Some(json!({"availableCount":0})),
            Some(json!({"availableCount":2})),
            Some(json!({"availableCount":100})),
            Some(Value::Null),
            Some(json!({"availableCount":"unknown"})),
        ] {
            let limits = Limits::parse(&ordinary_response(credits)).unwrap();
            assert_eq!(limits.weekly_used, 50.0);
            assert_eq!(limits.ordinary_usage_allowed, Some(true));
        }
    }

    #[test]
    fn limits_still_fail_closed_on_missing_primary_or_weekly_data() {
        let mut data = ordinary_response(Some(json!({"availableCount":5})));
        data["result"]["rateLimits"]["secondary"] = Value::Null;
        assert!(Limits::parse(&data).is_err());
        data["result"]["rateLimits"]["secondary"] = json!({"usedPercent":101});
        assert!(Limits::parse(&data).is_err());
    }

    #[test]
    fn backend_ordinary_usage_permission_must_be_explicit() {
        let mut value = ordinary_response(Some(json!({"availableCount":10})));
        assert_eq!(
            Limits::parse(&value).unwrap().ordinary_usage_allowed,
            Some(true)
        );
        value["result"]["ordinaryUsageAllowed"] = json!(false);
        assert_eq!(
            Limits::parse(&value).unwrap().ordinary_usage_allowed,
            Some(false)
        );
        value["result"]["ordinaryUsageAllowed"] = Value::Null;
        assert_eq!(Limits::parse(&value).unwrap().ordinary_usage_allowed, None);
        value["result"]
            .as_object_mut()
            .unwrap()
            .remove("ordinaryUsageAllowed");
        assert_eq!(Limits::parse(&value).unwrap().ordinary_usage_allowed, None);
    }
}
