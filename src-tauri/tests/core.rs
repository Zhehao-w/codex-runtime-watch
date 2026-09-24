use codex_runtime_watch::{
    codex::{
        probe::parse_sse,
        provider::explicit_provider,
        rollout::{adapt, Correlator},
        watcher::scan_file,
    },
    db::Database,
    Observation,
};
use serde_json::json;
use std::io::Write;

fn runtime(selected: &str, runtime: &str) -> Observation {
    Observation {
        id: None,
        time: "x".into(),
        kind: "Runtime".into(),
        session_id: Some("s".into()),
        turn_id: Some("t".into()),
        selected_model: Some(selected.into()),
        selected_effort: Some("high".into()),
        runtime_model: Some(runtime.into()),
        runtime_effort: Some("high".into()),
        provider_model: None,
        evidence: "turn_context".into(),
        details: None,
    }
}

#[test]
fn real_current_rollout_lines_produce_and_deduplicate_turns() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("watch.db")).unwrap();
    let path = dir.path().join("sessions-root.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    let lines = [
        json!({"timestamp":"2026-09-24T10:00:00Z","type":"session_meta","payload":{"id":"thread-root","session_id":"session-root","parent_thread_id":null,"source":"cli","agent_path":"/root"}}),
        json!({"timestamp":"2026-09-24T10:00:01Z","type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"thread-root","thread_settings":{"model":"gpt-selected-fallback","reasoning_effort":"medium"}}}),
        json!({"timestamp":"2026-09-24T10:00:02Z","type":"turn_context","payload":{"turn_id":"turn-1","model":"gpt-runtime","effort":"high","collaboration_mode":{"mode":"default","settings":{"model":"gpt-selected","reasoning_effort":"high"}}}}),
        json!({"timestamp":"2026-09-24T10:01:00Z","type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"thread-root","thread_settings":{"model":"gpt-later","reasoning_effort":"low"}}}),
        json!({"timestamp":"2026-09-24T10:01:01Z","type":"turn_context","payload":{"turn_id":"turn-2","model":"gpt-later","effort":"low","collaboration_mode":{"mode":"default","settings":{}}}}),
    ];
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
    let mut correlator = Correlator::default();
    assert_eq!(scan_file(&db, &path, &mut correlator).unwrap(), 2);
    assert_eq!(scan_file(&db, &path, &mut correlator).unwrap(), 0);
    let rows = db.history("runtime", 10, 0).unwrap();
    assert_eq!(rows.len(), 2);
    let first = &rows[1];
    assert_eq!(first.session_id.as_deref(), Some("thread-root"));
    assert_eq!(first.selected_model.as_deref(), Some("gpt-selected"));
    assert_eq!(first.runtime_model.as_deref(), Some("gpt-runtime"));
    assert_eq!(first.result(), "Runtime model mismatch");
    assert_eq!(rows[0].selected_model.as_deref(), Some("gpt-later"));
}

#[test]
fn file_scope_is_fallback_and_subagents_do_not_mix() {
    let mut c = Correlator::default();
    let turn = |id: &str| {
        adapt(&json!({"type":"turn_context","payload":{"turn_id":id,"model":"m","effort":"low"}}))
            .unwrap()
    };
    let a = c.push_scoped("rollout:a", turn("same")).unwrap();
    let b = c.push_scoped("rollout:b", turn("same")).unwrap();
    assert_ne!(a.session_id, b.session_id);
    c.push_scoped("rollout:child",adapt(&json!({"type":"session_meta","payload":{"id":"child","session_id":"root","parent_thread_id":"root","agent_path":"/root/worker"}})).unwrap());
    let child = c.push_scoped("rollout:child", turn("child-turn")).unwrap();
    assert_eq!(child.session_id.as_deref(), Some("child"));
    assert!(child.details.unwrap().contains("subagent"));
}

#[test]
fn provider_accepts_current_envelope_not_unstructured_text() {
    let v = json!({"method":"model/rerouted","params":{"threadId":"thread","turnId":"turn","fromModel":"gpt-a","toModel":"gpt-b","reason":"highRiskCyberActivity"}});
    assert_eq!(explicit_provider(&v).unwrap().0, "gpt-b");
    let e = adapt(&v).unwrap();
    assert_eq!(e.thread_id.as_deref(), Some("thread"));
    assert_eq!(e.turn_id.as_deref(), Some("turn"));
    assert!(explicit_provider(&json!({"message":"model/rerouted to gpt-secret"})).is_none());
}

#[test]
fn realistic_probe_sse_is_structured() {
    let body="event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"model\":\"gpt-provider\"}}\n\ndata: [DONE]\n";
    let parsed = parse_sse(body);
    assert_eq!(parsed.model.as_deref(), Some("gpt-provider"));
    assert_eq!(
        parse_sse("data: {\"type\":\"output_text\",\"text\":\"model: fake\"}\n").model,
        None
    );
    assert_eq!(parse_sse("data: {\"type\":\"response.created\",\"response\":{\"headers\":{\"OpenAI-Model\":\"gpt-header\"}}}\n").model.as_deref(), Some("gpt-header"));
}

#[test]
fn failed_probe_is_not_a_mismatch_and_filter_precedes_limit() {
    let mut failed = runtime("a", "b");
    failed.kind = "Probe".into();
    failed.runtime_model = None;
    failed.details = Some("{\"status\":\"failed\",\"error_class\":\"auth\"}".into());
    assert_eq!(failed.result(), "Probe failed");
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("x.db")).unwrap();
    db.insert(&runtime("selected", "different")).unwrap();
    for n in 0..120 {
        let mut row = runtime("same", "same");
        row.turn_id = Some(format!("turn-{n}"));
        row.time = format!("2027-{n:03}");
        db.insert(&row).unwrap();
    }
    let rows = db.history("mismatches", 1, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].result(), "Runtime model mismatch");
}
