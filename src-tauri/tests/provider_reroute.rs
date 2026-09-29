use codex_runtime_watch::{
    codex::{probe::parse_sse_reader, rollout::Correlator, watcher::scan_file_observations},
    db::Database,
    Observation,
};
use serde_json::json;
use std::io::Write;

#[test]
fn explicit_reroute_remains_visible_when_provider_matches_selected() {
    let row = Observation {
        id: None,
        time: "x".into(),
        kind: "Runtime".into(),
        session_id: Some("s".into()),
        turn_id: Some("t".into()),
        selected_model: Some("same".into()),
        selected_effort: Some("high".into()),
        runtime_model: Some("same".into()),
        runtime_effort: Some("high".into()),
        provider_model: Some("same".into()),
        evidence: "turn_context + server_model".into(),
        details: Some(
            json!({
                "runtime_evidence": "turn_context",
                "provider_evidence": "server_model",
                "provider_rerouted": true
            })
            .to_string(),
        ),
    };

    assert!(row.provider_was_rerouted());
    assert!(row.has_any_mismatch_or_reroute());
    assert_eq!(
        row.result(),
        "Provider match · reroute observed · Runtime match"
    );
}

#[test]
fn latest_explicit_server_model_wins_within_probe_stream() {
    let body = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp\",\"model\":\"fallback\",\"headers\":{\"X-OpenAI-Model\":\"model-a\"}}}\n\n",
        "data: {\"type\":\"response.metadata\",\"headers\":{\"openai-model\":\"model-b\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp\"}}\n\n"
    );
    let parsed = parse_sse_reader(body.as_bytes(), 4096).unwrap();
    assert_eq!(parsed.model.as_deref(), Some("model-b"));
    assert_eq!(parsed.source.as_deref(), Some("sse headers OpenAI-Model"));
    assert!(parsed.explicit);
    assert_eq!(parsed.response_id.as_deref(), Some("resp"));
}

#[test]
fn later_provider_evidence_keeps_reroute_fact_but_replaces_reason() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("reroute.db")).unwrap();
    let path = dir.path().join("rollout.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"session_meta","payload":{"id":"thread"}})
    )
    .unwrap();
    writeln!(file, "{}", json!({"type":"turn_context","payload":{"turn_id":"turn","model":"selected","effort":"high","collaboration_mode":{"settings":{"model":"selected","reasoning_effort":"high"}}}})).unwrap();
    writeln!(file, "{}", json!({"method":"model/rerouted","params":{"threadId":"thread","turnId":"turn","fromModel":"selected","toModel":"routed","reason":"capacity"}})).unwrap();

    let mut correlator = Correlator::default();
    scan_file_observations(&db, &path, &mut correlator).unwrap();

    writeln!(
        file,
        "{}",
        json!({"type":"server_model","thread_id":"thread","turn_id":"turn","model":"selected"})
    )
    .unwrap();
    let changed = scan_file_observations(&db, &path, &mut correlator).unwrap();
    assert_eq!(changed.len(), 1);
    assert!(!changed[0].notify);

    let row = db.history("runtime", 1, 0).unwrap().remove(0);
    assert!(row.provider_was_rerouted());
    assert_eq!(
        row.result(),
        "Provider match · reroute observed · Runtime match"
    );
    let details: serde_json::Value = serde_json::from_str(row.details.as_deref().unwrap()).unwrap();
    assert_eq!(details["provider_evidence"], "server_model");
    assert_eq!(details["provider_rerouted"], true);
    assert!(details.get("provider_reason").is_none());
    assert_eq!(db.history("mismatches", 10, 0).unwrap().len(), 1);
}

#[test]
fn reroute_before_turn_context_notifies_provider_anomaly_only_once() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("dedupe.db")).unwrap();
    let path = dir.path().join("rollout.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"session_meta","payload":{"id":"thread"}})
    )
    .unwrap();
    writeln!(file, "{}", json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"thread","thread_settings":{"model":"fallback","reasoning_effort":"high"}}})).unwrap();
    writeln!(file, "{}", json!({"method":"model/rerouted","params":{"threadId":"thread","turnId":"turn","fromModel":"fallback","toModel":"served","reason":"capacity"}})).unwrap();

    let mut correlator = Correlator::default();
    let first = scan_file_observations(&db, &path, &mut correlator).unwrap();
    assert_eq!(first.len(), 1);
    assert!(first[0].notify);
    assert_eq!(first[0].observation.result(), "Provider reroute");

    writeln!(file, "{}", json!({"type":"turn_context","payload":{"turn_id":"turn","model":"different-selected","effort":"high","collaboration_mode":{"settings":{"model":"different-selected","reasoning_effort":"high"}}}})).unwrap();
    let second = scan_file_observations(&db, &path, &mut correlator).unwrap();
    assert_eq!(second.len(), 1);
    assert!(!second[0].notify);
    assert!(second[0].observation.provider_was_rerouted());
    assert!(second[0]
        .observation
        .provider_model_mismatch_authoritative());
}

#[test]
fn generic_provider_before_turn_context_is_provisional_and_silent() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("provisional.db")).unwrap();
    let path = dir.path().join("rollout.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"session_meta","payload":{"id":"thread"}})
    )
    .unwrap();
    writeln!(file, "{}", json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"thread","thread_settings":{"model":"fallback-a","reasoning_effort":"high"}}})).unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"server_model","thread_id":"thread","turn_id":"turn","model":"per-turn-b"})
    )
    .unwrap();

    let mut correlator = Correlator::default();
    let provisional = scan_file_observations(&db, &path, &mut correlator).unwrap();
    assert_eq!(provisional.len(), 1);
    assert!(!provisional[0].notify);
    assert_eq!(provisional[0].observation.result(), "Provider observed");
    assert!(db.history("mismatches", 10, 0).unwrap().is_empty());

    writeln!(file, "{}", json!({"type":"turn_context","payload":{"turn_id":"turn","model":"per-turn-b","effort":"high","collaboration_mode":{"settings":{"model":"per-turn-b","reasoning_effort":"high"}}}})).unwrap();
    let final_change = scan_file_observations(&db, &path, &mut correlator).unwrap();
    assert_eq!(final_change.len(), 1);
    assert!(!final_change[0].notify);
    assert_eq!(
        final_change[0].observation.result(),
        "Provider match · Runtime match"
    );
}

#[test]
fn provider_match_does_not_repeat_existing_runtime_mismatch_alert() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("runtime-dedupe.db")).unwrap();
    let path = dir.path().join("rollout.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"session_meta","payload":{"id":"thread"}})
    )
    .unwrap();
    writeln!(file, "{}", json!({"type":"turn_context","payload":{"turn_id":"turn","model":"runtime-b","effort":"high","collaboration_mode":{"settings":{"model":"selected-a","reasoning_effort":"high"}}}})).unwrap();

    let mut correlator = Correlator::default();
    let initial = scan_file_observations(&db, &path, &mut correlator).unwrap();
    assert_eq!(initial.len(), 1);
    assert!(initial[0].notify);

    writeln!(
        file,
        "{}",
        json!({"type":"server_model","thread_id":"thread","turn_id":"turn","model":"selected-a"})
    )
    .unwrap();
    let provider = scan_file_observations(&db, &path, &mut correlator).unwrap();
    assert_eq!(provider.len(), 1);
    assert!(!provider[0].notify);
    assert!(provider[0].observation.has_runtime_mismatch());
    assert_eq!(
        provider[0].observation.result(),
        "Provider match · Runtime model mismatch"
    );
}
