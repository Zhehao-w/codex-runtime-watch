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
fn scanner_restores_rollout_context_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("watch.db")).unwrap();
    let path = dir.path().join("sessions-resumed.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(file, "{}", json!({"type":"session_meta","payload":{"id":"canonical-thread","session_id":"root-session","parent_thread_id":"parent","agent_path":"/root/child"}})).unwrap();
    writeln!(file, "{}", json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"canonical-thread","thread_settings":{"model":"selected-model","reasoning_effort":"high"}}})).unwrap();
    writeln!(file, "{}", json!({"type":"turn_context","payload":{"turn_id":"before-restart","model":"runtime-model","effort":"high"}})).unwrap();
    let mut before = Correlator::default();
    assert_eq!(scan_file(&db, &path, &mut before).unwrap(), 1);

    writeln!(file, "{}", json!({"type":"turn_context","payload":{"turn_id":"after-restart","model":"runtime-model","effort":"high"}})).unwrap();
    let mut after = Correlator::default();
    assert_eq!(scan_file(&db, &path, &mut after).unwrap(), 1);
    let rows = db.history("runtime", 10, 0).unwrap();
    let resumed = rows
        .iter()
        .find(|row| row.turn_id.as_deref() == Some("after-restart"))
        .unwrap();
    assert_eq!(resumed.session_id.as_deref(), Some("canonical-thread"));
    assert_eq!(resumed.selected_model.as_deref(), Some("selected-model"));
    assert_eq!(resumed.selected_effort.as_deref(), Some("high"));
    assert!(resumed.details.as_deref().unwrap().contains("parent"));
}

#[test]
fn later_provider_evidence_reports_one_material_update() {
    use codex_runtime_watch::codex::watcher::scan_file_observations;
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("watch.db")).unwrap();
    let path = dir.path().join("sessions-provider.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(file, "{}", json!({"type":"session_meta","payload":{"id":"thread-provider","session_id":"thread-provider"}})).unwrap();
    writeln!(file, "{}", json!({"timestamp":"2026-09-24T12:00:00Z","type":"turn_context","payload":{"turn_id":"turn-provider","model":"requested","effort":"high","collaboration_mode":{"settings":{"model":"requested","reasoning_effort":"high"}}}})).unwrap();
    let mut correlator = Correlator::default();
    assert_eq!(
        scan_file_observations(&db, &path, &mut correlator)
            .unwrap()
            .len(),
        1
    );

    let reroute = json!({"timestamp":"2026-09-24T12:01:00Z","method":"model/rerouted","params":{"threadId":"thread-provider","turnId":"turn-provider","fromModel":"requested","toModel":"served","reason":"capacity"}});
    writeln!(file, "{reroute}").unwrap();
    let changed = scan_file_observations(&db, &path, &mut correlator).unwrap();
    assert_eq!(changed.len(), 1);
    assert!(changed[0].notify);
    assert_eq!(
        changed[0].observation.provider_model.as_deref(),
        Some("served")
    );
    let rows = db.history("runtime", 10, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].time, "2026-09-24T12:00:00Z");
    let details = rows[0].details.as_deref().unwrap();
    assert!(details.contains("turn_context"));
    assert!(details.contains("model/rerouted"));
    assert!(details.contains("capacity"));

    writeln!(file, "{reroute}").unwrap();
    assert!(scan_file_observations(&db, &path, &mut correlator)
        .unwrap()
        .is_empty());
}

#[test]
fn provider_accepts_current_envelope_not_unstructured_text() {
    let v = json!({"timestamp":"2026-09-24T13:00:00Z","method":"model/rerouted","params":{"threadId":"thread","turnId":"turn","fromModel":"gpt-a","toModel":"gpt-b","reason":"highRiskCyberActivity"}});
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
    assert_eq!(parse_sse("data: {\"type\":\"response.created\",\"response\":{\"headers\":{\"OpenAI-Model\":\"gpt-header\"}}}\n").model, None);
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

#[test]
fn provider_status_never_hides_runtime_facts() {
    let mut row = runtime("model-a", "model-b");
    row.runtime_effort = Some("low".into());
    row.provider_model = Some("model-a".into());
    assert_eq!(
        row.result(),
        "Provider match · Runtime model + effort mismatch"
    );
    assert!(row.has_runtime_mismatch());

    row.runtime_model = Some("model-a".into());
    row.runtime_effort = None;
    assert_eq!(
        row.result(),
        "Provider match · Runtime model match · effort not observed"
    );
    assert!(!row.runtime_effort_comparable());
}

#[test]
fn mismatch_filter_includes_runtime_mismatch_with_provider_match() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("filter.db")).unwrap();
    let mut row = runtime("selected", "different");
    row.provider_model = Some("selected".into());
    db.insert(&row).unwrap();
    let rows = db.history("mismatches", 10, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].has_runtime_mismatch());
}

#[test]
fn provider_before_turn_context_merges_and_preserves_evidence() {
    use codex_runtime_watch::codex::watcher::scan_file_observations;
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("merge.db")).unwrap();
    let path = dir.path().join("rollout.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"session_meta","payload":{"id":"thread"}})
    )
    .unwrap();
    writeln!(file, "{}", json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"thread","thread_settings":{"model":"fallback","reasoning_effort":"medium"}}})).unwrap();
    writeln!(file, "{}", json!({"timestamp":"2026-09-24T13:00:00Z","method":"model/rerouted","params":{"threadId":"thread","turnId":"turn","toModel":"provider","reason":"capacity"}})).unwrap();
    writeln!(file, "{}", json!({"timestamp":"2026-09-24T13:01:00Z","type":"turn_context","payload":{"turn_id":"turn","model":"runtime","effort":"high","parent_thread_id":"parent","collaboration_mode":{"settings":{"model":"per-turn","reasoning_effort":"high"}}}})).unwrap();
    let changed = scan_file_observations(&db, &path, &mut Correlator::default()).unwrap();
    assert_eq!(changed.len(), 2);
    let row = &db.history("runtime", 10, 0).unwrap()[0];
    assert_eq!(row.time, "2026-09-24T13:01:00Z");
    assert_eq!(row.selected_model.as_deref(), Some("per-turn"));
    assert_eq!(row.provider_model.as_deref(), Some("provider"));
    assert_eq!(row.runtime_model.as_deref(), Some("runtime"));
    let details = row.details.as_deref().unwrap();
    assert!(details.contains("parent"));
    assert!(details.contains("capacity"));
    assert!(details.contains("turn_context"));
    assert!(details.contains("model/rerouted"));
}

#[test]
fn correlator_does_not_retain_completed_turns() {
    let mut correlator = Correlator::default();
    for turn in 0..10_000 {
        let evidence = adapt(&json!({"type":"turn_context","payload":{"turn_id":turn.to_string(),"model":"runtime","effort":"low"}})).unwrap();
        assert!(correlator.push_scoped("one-file", evidence).is_some());
    }
    assert_eq!(correlator.retained_entries(), 0);
}

#[test]
fn scanner_resets_after_shrink_and_replace_regrow() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("replacement.db")).unwrap();
    let path = dir.path().join("rollout.jsonl");
    let old = format!(
        "{}\n{}\n",
        json!({"type":"session_meta","payload":{"id":"old-thread"}}),
        json!({"type":"turn_context","payload":{"turn_id":"old-turn","model":"old-runtime","effort":"low","collaboration_mode":{"settings":{"model":"old-selected","reasoning_effort":"low"}}}})
    );
    std::fs::write(&path, &old).unwrap();
    let mut correlator = Correlator::default();
    assert_eq!(scan_file(&db, &path, &mut correlator).unwrap(), 1);

    std::fs::write(&path, "{}\n").unwrap();
    assert_eq!(scan_file(&db, &path, &mut correlator).unwrap(), 0);

    let mut replacement = format!(
        "{}\n{}\n",
        json!({"type":"session_meta","payload":{"id":"new-thread"}}),
        json!({"type":"turn_context","payload":{"turn_id":"new-turn","model":"new-runtime","effort":"high","collaboration_mode":{"settings":{"model":"new-selected","reasoning_effort":"high"}}}})
    );
    while replacement.len() < old.len() + 100 {
        replacement.push_str("{\"type\":\"ignored\"}\n");
    }
    std::fs::write(&path, replacement).unwrap();
    assert_eq!(scan_file(&db, &path, &mut correlator).unwrap(), 1);
    let row = db.history("runtime", 1, 0).unwrap().remove(0);
    assert_eq!(row.session_id.as_deref(), Some("new-thread"));
    assert_eq!(row.selected_model.as_deref(), Some("new-selected"));
}

#[test]
fn probe_normalization_matches_sent_facts_and_stream_is_bounded() {
    use codex_runtime_watch::codex::probe::{normalize_request, parse_sse_reader};
    assert_eq!(
        normalize_request(" future-model ", " ").unwrap(),
        ("future-model".into(), "low".into())
    );
    assert_eq!(
        normalize_request("m", " future-effort ").unwrap().1,
        "future-effort"
    );
    let body = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"model: fake\"}\n\nevent: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"id\",\"model\":\"real\"}}\n\ndata: {\"type\":\"response.created\",\"response\":{\"model\":\"later\"}}\n\n";
    let parsed = parse_sse_reader(body.as_bytes(), 4096).unwrap();
    assert_eq!(parsed.model.as_deref(), Some("real"));
    assert!(parse_sse_reader("data: ignored forever\n".repeat(100).as_bytes(), 10).is_err());
}

#[test]
fn effort_comparison_is_independent_when_model_is_unavailable() {
    let mut row = runtime("unused", "runtime");
    row.selected_model = None;
    row.selected_effort = Some("high".into());
    row.runtime_effort = Some("low".into());
    assert_eq!(row.result(), "Runtime effort mismatch · model not observed");
    assert!(row.has_runtime_mismatch());

    row.runtime_effort = Some("high".into());
    assert_eq!(row.result(), "Runtime effort match · model not observed");
    assert!(!row.has_runtime_mismatch());
}

#[test]
fn provider_observed_is_not_inferred_to_be_a_reroute() {
    let mut observed = runtime("unused", "unused");
    observed.selected_model = None;
    observed.runtime_model = None;
    observed.selected_effort = None;
    observed.runtime_effort = None;
    observed.provider_model = Some("served".into());
    observed.evidence = "server_model".into();
    observed.details = Some(json!({"provider_evidence":"server_model"}).to_string());
    assert_eq!(observed.result(), "Provider observed");
    assert!(!observed.has_any_mismatch_or_reroute());

    observed.evidence = "model/rerouted".into();
    observed.details = Some(json!({"provider_evidence":"model/rerouted"}).to_string());
    assert_eq!(observed.result(), "Provider reroute");
    assert!(observed.has_any_mismatch_or_reroute());

    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("provider-status.db")).unwrap();
    observed.turn_id = Some("rerouted".into());
    db.insert(&observed).unwrap();
    observed.turn_id = Some("observed".into());
    observed.evidence = "server_model".into();
    observed.details = Some(json!({"provider_evidence":"server_model"}).to_string());
    db.insert(&observed).unwrap();
    let mismatches = db.history("mismatches", 10, 0).unwrap();
    assert_eq!(mismatches.len(), 1);
    assert_eq!(mismatches[0].turn_id.as_deref(), Some("rerouted"));
}

#[test]
fn completed_file_scopes_do_not_accumulate_in_correlator() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("many-files.db")).unwrap();
    let mut correlator = Correlator::default();
    for index in 0..250 {
        let path = dir.path().join(format!("rollout-{index}.jsonl"));
        let contents = format!(
            "{}\n{}\n{}\n",
            json!({"type":"session_meta","payload":{"id":format!("thread-{index}")}}),
            json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":format!("thread-{index}"),"thread_settings":{"model":"selected","reasoning_effort":"high"}}}),
            json!({"type":"turn_context","payload":{"turn_id":format!("turn-{index}"),"model":"runtime","effort":"high"}})
        );
        std::fs::write(&path, contents).unwrap();
        assert_eq!(scan_file(&db, &path, &mut correlator).unwrap(), 1);
        assert_eq!(correlator.retained_entries(), 0);
    }
}
