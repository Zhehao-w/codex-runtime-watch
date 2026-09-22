use codex_runtime_watch::{
    codex::{
        probe::parse_events,
        rollout::{adapt, Correlator},
        watcher::scan_file,
    },
    db::Database,
    Observation,
};
use serde_json::json;
use std::io::Write;
fn event(t: &str, p: serde_json::Value) -> serde_json::Value {
    json!({"timestamp":"2026-01-01T00:00:00Z","type":t,"thread_id":"a","payload":p})
}
#[test]
fn correlation_matrix_and_settings_changes() {
    let mut c = Correlator::default();
    assert!(c
        .push(
            adapt(&event(
                "thread_settings_applied",
                json!({"model":"gpt-a","reasoning_effort":"high"})
            ))
            .unwrap()
        )
        .is_none());
    let o = c
        .push(
            adapt(&event(
                "turn_context",
                json!({"id":"1","model":"gpt-a","effort":"high"}),
            ))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(o.result(), "Runtime match");
    c.push(
        adapt(&event(
            "thread_settings_applied",
            json!({"model":"gpt-b","effort":"low"}),
        ))
        .unwrap(),
    );
    let o = c
        .push(
            adapt(&event(
                "turn_context",
                json!({"id":"2","model":"gpt-c","effort":"medium","unknown":1}),
            ))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(o.result(), "Runtime model + effort mismatch")
}
#[test]
fn missing_unknown_subagent_and_provider() {
    let mut c = Correlator::default();
    assert!(adapt(&event("future_event", json!({"model":"secret"}))).is_none());
    let o = c
        .push(
            adapt(&event(
                "turn_context",
                json!({"id":"3","model":"future-model","effort":"ultra","parent_thread_id":"root"}),
            ))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(o.selected_model, None);
    assert!(o.details.unwrap().contains("subagent"));
    let p = adapt(&event(
        "model/rerouted",
        json!({"turn_id":"3","model":"served-x","reason":"safety"}),
    ))
    .unwrap();
    assert_eq!(p.provider_model.as_deref(), Some("served-x"));
    assert!(adapt(&event("model/rerouted", json!({"reason":"missing"}))).is_none())
}
#[test]
fn statuses() {
    let base = Observation {
        id: None,
        time: "x".into(),
        kind: "Runtime".into(),
        session_id: None,
        turn_id: None,
        selected_model: Some("a".into()),
        selected_effort: Some("high".into()),
        runtime_model: Some("b".into()),
        runtime_effort: Some("high".into()),
        provider_model: None,
        evidence: "x".into(),
        details: None,
    };
    assert_eq!(base.result(), "Runtime model mismatch");
    let mut x = base.clone();
    x.runtime_model = Some("a".into());
    x.runtime_effort = Some("low".into());
    assert_eq!(x.result(), "Runtime effort mismatch");
    x.provider_model = Some("a".into());
    assert_eq!(x.result(), "Provider match")
}
#[test]
fn scanner_partial_restart_truncate_and_malformed() {
    let d = tempfile::tempdir().unwrap();
    let db = Database::open(&d.path().join("x.db")).unwrap();
    let p = d.path().join("rollout.jsonl");
    let mut f = std::fs::File::create(&p).unwrap();
    writeln!(f, "not json").unwrap();
    write!(
        f,
        "{}",
        event("turn_context", json!({"id":"1","model":"a"}))
    )
    .unwrap();
    let mut c = Correlator::default();
    assert_eq!(scan_file(&db, &p, &mut c).unwrap(), 0);
    writeln!(f).unwrap();
    assert_eq!(scan_file(&db, &p, &mut c).unwrap(), 1);
    assert_eq!(scan_file(&db, &p, &mut c).unwrap(), 0);
    std::fs::write(
        &p,
        format!("{}\n", event("turn_context", json!({"id":"2","model":"b"}))),
    )
    .unwrap();
    assert_eq!(scan_file(&db, &p, &mut c).unwrap(), 1)
}
#[test]
fn structured_probe_parser() {
    let (m, r) = parse_events(vec![
        "bad".into(),
        json!({"type":"noise","model":"wrong"}).to_string(),
        json!({"type":"model/rerouted","payload":{"server_model":"served","reason":"capacity"}})
            .to_string(),
    ]);
    assert_eq!(m.as_deref(), Some("served"));
    assert!(r.unwrap().to_string().contains("capacity"));
    assert_eq!(
        parse_events(vec![json!({"type":"done"}).to_string()]).0,
        None
    )
}
