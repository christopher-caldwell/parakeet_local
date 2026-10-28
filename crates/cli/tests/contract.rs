use serde_json::{Value, json};
use std::{path::Path, process::Command};
fn run(root: &Path, args: &[&str]) -> (i32, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_parakeet"))
        .arg("--data-root")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    let data: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "invalid stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(data["schema_version"], 1);
    (output.status.code().unwrap(), data)
}
#[test]
fn health_and_import_plan_do_not_initialize_storage() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("absent");
    let (code, result) = run(&root, &["doctor"]);
    assert_eq!(code, 0);
    assert_eq!(result["data"]["database"]["initialized"], false);
    assert!(!root.exists());
    let source = tmp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("manifest.json"),
        json!({"meeting":{"id":"fixture","title":"Source title","createdAt":"2026-01-01"}})
            .to_string(),
    )
    .unwrap();
    let original = std::fs::read(source.join("manifest.json")).unwrap();
    let (code, result) = run(
        &root,
        &["meetings", "import", source.to_str().unwrap(), "--dry-run"],
    );
    assert_eq!(code, 0);
    assert_eq!(result["data"]["recordings"].as_array().unwrap().len(), 1);
    assert!(!root.exists());
    assert_eq!(
        original,
        std::fs::read(source.join("manifest.json")).unwrap()
    );
}
#[test]
fn json_errors_idempotency_and_durable_title_override() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("app");
    assert_eq!(run(&root, &["db", "create"]).0, 0);
    assert_eq!(run(&root, &["db", "migrate"]).0, 0);
    let (code, error) = run(&root, &["meetings", "show", "missing"]);
    assert_eq!(code, 3);
    assert_eq!(error["error"]["kind"], "not_found");
    let source = tmp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("manifest.json"),
        json!({"meeting":{"id":"fixture","title":"Source title","createdAt":"2026-01-01"}})
            .to_string(),
    )
    .unwrap();
    std::fs::write(source.join("transcript.json"),json!({"transcriptSegments":[{"startMs":5000,"endMs":6000,"text":"Later"},{"startMs":1000,"endMs":2000,"text":"Earlier"}]}).to_string()).unwrap();
    let (_, r) = run(&root, &["meetings", "import", source.to_str().unwrap()]);
    assert_eq!(r["data"]["imported"], 1);
    let id = r["data"]["recordings"][0].as_str().unwrap();
    assert_eq!(
        run(&root, &["meetings", "rename", id, "My explicit title"]).0,
        0
    );
    let (_, r) = run(&root, &["meetings", "import", source.to_str().unwrap()]);
    assert_eq!(r["data"]["imported"], 0);
    assert_eq!(r["data"]["existing"], 1);
    let (_, r) = run(&root, &["meetings", "show", id]);
    assert_eq!(
        r["data"]["recording"]["title_override"],
        "My explicit title"
    );
    assert_eq!(r["data"]["segments"][0]["text"], "Earlier");
    let segment = r["data"]["segments"][0]["id"].as_str().unwrap();
    assert_eq!(run(&root, &["speakers", "unknown", segment]).0, 0);
    assert_eq!(
        run(&root, &["speakers", "text", segment, "Corrected earlier"]).0,
        0
    );
    let (code, text) = run(&root, &["meetings", "export", id, "--format", "text"]);
    assert_eq!(code, 0);
    assert_eq!(text["data"]["text"], "Corrected earlier\n\nLater");
    assert_eq!(run(&root, &["speakers", "undo", segment]).0, 0);
    assert_eq!(
        run(&root, &["meetings", "export", id, "--format", "text"]).1["data"]["text"],
        "Earlier\n\nLater"
    );
    let (code, error) = run(&root, &["does-not-exist"]);
    assert_eq!(code, 2);
    assert_eq!(error["error"]["kind"], "invalid_arguments");
}

#[test]
fn unverified_personal_labels_cannot_be_scored_as_ground_truth() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("app");
    assert_eq!(run(&root, &["db", "create"]).0, 0);
    assert_eq!(run(&root, &["db", "migrate"]).0, 0);
    let suite = temp.path().join("suite.json");
    std::fs::write(&suite,json!({"id":"fixture","dataset":"unverified fixture","split":"diagnostic","labels":"unverified_personal","entries":[{"id":"r","audio":"/not/read","reference":"/not/read"}],"models":["whisper-base.en"],"collar_ms":0,"score_overlap":true}).to_string()).unwrap();
    let (code, error) = run(&root, &["benchmark", "run", suite.to_str().unwrap()]);
    assert_eq!(code, 2);
    assert_eq!(error["error"]["kind"], "invalid_input");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unverified")
    );
}

#[test]
fn audio_settings_inspection_preserves_explicit_device_selection() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("app");
    assert_eq!(run(&root, &["db", "create"]).0, 0);
    assert_eq!(run(&root, &["db", "migrate"]).0, 0);
    assert_eq!(
        run(
            &root,
            &[
                "dictation",
                "audio-settings",
                "--microphone-uid",
                "fixture-input",
                "--system",
                "true"
            ]
        )
        .0,
        0
    );
    let (_, read) = run(&root, &["dictation", "audio-settings"]);
    assert_eq!(read["data"]["microphone_uid"], "fixture-input");
    assert_eq!(read["data"]["system_audio"], true);
    let (_, updated) = run(&root, &["dictation", "audio-settings", "--system", "false"]);
    assert_eq!(updated["data"]["microphone_uid"], "fixture-input");
    assert_eq!(updated["data"]["system_audio"], false);
    let (_, cleared) = run(
        &root,
        &["dictation", "audio-settings", "--clear-microphone"],
    );
    assert!(cleared["data"]["microphone_uid"].is_null());
}

#[test]
fn pending_jobs_are_read_only_and_recovery_does_not_initialize_storage() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("library");
    assert_ne!(run(&root, &["jobs", "pending"]).0, 0);
    assert!(!root.exists());
    run(&root, &["db", "create"]);
    run(&root, &["db", "migrate"]);
    assert_eq!(run(&root, &["jobs", "pending"]).1["data"], json!([]));
    assert!(!root.join("parakeet.db.processing-leases").exists());
    let (code, r) = run(&root, &["jobs", "recover"]);
    assert_eq!(code, 0);
    assert_eq!(r["data"]["interrupted"], json!([]));
    assert_eq!(run(&root, &["jobs", "retry", "missing"]).0, 3);
}

#[test]
fn clipboard_recovery_without_a_journal_does_not_initialize_storage() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("absent");
    let (code, result) = run(&root, &["dictation", "recover-clipboard"]);
    assert_eq!(code, 0);
    assert_eq!(result["data"]["restored"], 0);
    assert!(!root.exists());
}

#[test]
fn speaker_word_scoring_uses_verified_evidence_without_initializing_storage() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("absent");
    let reference = tmp.path().join("reference.json");
    let prediction = tmp.path().join("prediction.json");
    let mut truth = json!({"dataset":"synthetic fixture","labels":"synthetic","audio_sha256":"ab".repeat(32),"streams":[{"speaker":"a","text":"one three"},{"speaker":"b","text":"two four"}]});
    let mut hypothesis = json!({"dataset":"synthetic fixture","labels":"model_prediction","audio_sha256":"ab".repeat(32),"streams":[{"speaker":"x","text":"one two"},{"speaker":"y","text":"three four"}]});
    std::fs::write(&reference, serde_json::to_vec(&truth).unwrap()).unwrap();
    std::fs::write(&prediction, serde_json::to_vec(&hypothesis).unwrap()).unwrap();
    let command = [
        "benchmark",
        "score-speakers",
        reference.to_str().unwrap(),
        prediction.to_str().unwrap(),
    ];
    let (code, result) = run(&root, &command);
    assert_eq!(code, 0);
    assert_eq!(result["data"]["score"]["cpwer"], 0.5);
    assert_eq!(result["data"]["score"]["errors"], 2);
    assert_eq!(
        result["data"]["reference_sha256"].as_str().unwrap().len(),
        64
    );
    assert!(!root.exists());
    hypothesis["audio_sha256"] = json!("cd".repeat(32));
    std::fs::write(&prediction, serde_json::to_vec(&hypothesis).unwrap()).unwrap();
    assert_eq!(run(&root, &command).0, 2);
    hypothesis["audio_sha256"] = truth["audio_sha256"].clone();
    std::fs::write(&prediction, serde_json::to_vec(&hypothesis).unwrap()).unwrap();
    truth["labels"] = json!("unverified_personal");
    std::fs::write(&reference, serde_json::to_vec(&truth).unwrap()).unwrap();
    assert_eq!(run(&root, &command).0, 2);
    assert!(!root.exists());
}

#[test]
fn meeting_finalization_commands_keep_provisioning_and_consent_explicit() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("app");
    let (code, _) = run(&root, &["meetings", "pending-transcriptions"]);
    assert_eq!(code, 1);
    assert!(!root.exists());
    assert_eq!(run(&root, &["db", "create"]).0, 0);
    assert_eq!(run(&root, &["db", "migrate"]).0, 0);
    let (code, settings) = run(&root, &["capture", "settings"]);
    assert_eq!(code, 0);
    assert_eq!(settings["data"]["transcribe_meetings_after_stop"], true);
    assert_eq!(
        run(
            &root,
            &["capture", "settings", "--transcribe-after-stop", "true"]
        )
        .0,
        0
    );
    assert_eq!(
        run(&root, &["capture", "settings"]).1["data"]["transcribe_meetings_after_stop"],
        true
    );
    let (code, pending) = run(&root, &["meetings", "pending-transcriptions"]);
    assert_eq!(code, 0);
    assert_eq!(pending["data"], json!([]));
    assert!(!root.join("parakeet.db.processing-leases").exists());
    let (code, missing) = run(&root, &["meetings", "finalize", "missing"]);
    assert_eq!(code, 3);
    assert_eq!(missing["error"]["kind"], "not_found");
    let (code, invalid) = run(
        &root,
        &[
            "capture",
            "record",
            "--system",
            "--seconds",
            "0",
            "--transcribe",
        ],
    );
    assert_eq!(code, 2);
    assert_eq!(invalid["error"]["kind"], "invalid_input");
    let (code, _) = run(
        &root,
        &[
            "capture",
            "record",
            "--system",
            "--seconds",
            "1",
            "--transcribe",
        ],
    );
    assert_ne!(code, 0);
    assert!(!root.join("captures").exists());
    assert!(!root.join("capture.lock").exists());
}

#[test]
fn job_inspection_and_control_keep_read_only_and_mutating_commands_distinct() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("app");
    assert_eq!(run(&root, &["jobs", "show", "missing"]).0, 1);
    assert!(!root.exists());
    assert_eq!(run(&root, &["db", "create"]).0, 0);
    assert_eq!(run(&root, &["db", "migrate"]).0, 0);
    let (code, data) = run(&root, &["jobs", "show", "missing"]);
    assert_eq!(code, 3);
    assert_eq!(data["error"]["kind"], "not_found");
    assert_eq!(run(&root, &["jobs", "cancel", "missing"]).0, 3);
    assert!(!root.join("parakeet.db.processing-leases").exists());
}

#[test]
fn storage_inspection_is_read_only_and_policy_saves_require_an_explicit_revision() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("absent");
    for args in [
        &["storage", "usage"][..],
        &["storage", "preview"][..],
        &["storage", "policy"][..],
    ] {
        assert_ne!(run(&root, args).0, 0);
        assert!(!root.exists());
    }
    assert_eq!(run(&root, &["db", "create"]).0, 0);
    assert_eq!(run(&root, &["db", "migrate"]).0, 0);
    let (code, p) = run(&root, &["storage", "policy"]);
    assert_eq!(code, 0);
    assert_eq!(p["data"]["revision"], 0);
    assert!(p["data"]["policy"]["completed_scratch_after_days"].is_null());
    let (code, p) = run(
        &root,
        &["storage", "preview", "--completed-scratch-days", "30"],
    );
    assert_eq!(code, 0);
    assert_eq!(p["data"]["hypothetical"], true);
    assert_eq!(p["data"]["automatic_cleanup"], false);
    assert_eq!(p["data"]["execution_available"], false);
    assert!(!root.join("models").exists());
    assert_eq!(
        run(
            &root,
            &[
                "storage",
                "set-policy",
                "--completed-scratch-days",
                "30",
                "--expected-revision",
                "0"
            ]
        )
        .0,
        0
    );
    assert_eq!(
        run(
            &root,
            &[
                "storage",
                "set-policy",
                "--keep-all",
                "--expected-revision",
                "0"
            ]
        )
        .0,
        5
    );
    assert_eq!(
        run(
            &root,
            &[
                "storage",
                "set-policy",
                "--completed-scratch-days",
                "0",
                "--expected-revision",
                "1"
            ]
        )
        .0,
        2
    );
    let p = run(&root, &["storage", "policy"]).1;
    assert_eq!(p["data"]["revision"], 1);
    assert_eq!(p["data"]["policy"]["completed_scratch_after_days"], 30);
    assert_eq!(
        run(
            &root,
            &[
                "storage",
                "set-policy",
                "--keep-all",
                "--expected-revision",
                "1"
            ]
        )
        .0,
        0
    );
    assert_eq!(
        run(&root, &["storage", "preview"]).1["data"]["effective_policy"]["completed_scratch_after_days"],
        Value::Null
    );
}

#[test]
fn benchmark_import_respects_the_same_exclusive_storage_gate_as_library_import() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("app");
    assert_eq!(run(&root, &["db", "create"]).0, 0);
    assert_eq!(run(&root, &["db", "migrate"]).0, 0);
    // Valid minimal PCM WAV, generated locally; no model install or inference.
    let audio = tmp.path().join("synthetic.wav");
    let mut wave = Vec::new();
    wave.extend_from_slice(b"RIFF");
    wave.extend_from_slice(&32036u32.to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt ");
    wave.extend_from_slice(&16u32.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes());
    wave.extend_from_slice(&16000u32.to_le_bytes());
    wave.extend_from_slice(&32000u32.to_le_bytes());
    wave.extend_from_slice(&2u16.to_le_bytes());
    wave.extend_from_slice(&16u16.to_le_bytes());
    wave.extend_from_slice(b"data");
    wave.extend_from_slice(&32000u32.to_le_bytes());
    wave.extend_from_slice(&[0; 32000]);
    std::fs::write(&audio, wave).unwrap();
    let suite = tmp.path().join("suite.json");
    std::fs::write(&suite,json!({"id":"gate-regression","dataset":"synthetic","split":"test","labels":"synthetic","entries":[{"id":"silence","audio":audio}],"models":["whisper-base.en"],"collar_ms":0,"score_overlap":true}).to_string()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let gate =
        parakeet_infrastructure::storage_gate::StorageGate::for_database(&root.join("parakeet.db"))
            .unwrap();
    let guard = runtime.block_on(gate.acquire(true)).unwrap();
    assert_ne!(
        run(&root, &["benchmark", "run", suite.to_str().unwrap()]).0,
        0
    );
    assert!(
        run(&root, &["meetings", "list"]).1["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    drop(guard);
}
#[test]
fn analysis_is_explicit_and_all_matching_thresholds_are_required() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("absent");
    let (code, _) = run(&root, &["meetings", "analysis", "missing"]);
    assert_ne!(code, 0);
    assert!(!root.exists());
    let (code, error) = run(
        &root,
        &["meetings", "analyze", "missing", "--minimum-score", "0.9"],
    );
    assert_eq!(code, 2);
    assert_eq!(error["error"]["kind"], "invalid_arguments");
    assert!(!root.exists());
    assert_eq!(run(&root, &["db", "create"]).0, 0);
    assert_eq!(run(&root, &["db", "migrate"]).0, 0);
    let (code, preferences) = run(&root, &["capture", "settings"]);
    assert_eq!(code, 0);
    assert_eq!(preferences["data"]["analyze_meetings_after_stop"], false);
    let (code, preferences) = run(
        &root,
        &["capture", "settings", "--analyze-after-stop", "true"],
    );
    assert_eq!(code, 0);
    assert_eq!(preferences["data"]["transcribe_meetings_after_stop"], true);
    assert_eq!(preferences["data"]["analyze_meetings_after_stop"], true);
    let (code, preferences) = run(
        &root,
        &["capture", "settings", "--transcribe-after-stop", "false"],
    );
    assert_eq!(code, 0);
    assert_eq!(preferences["data"]["analyze_meetings_after_stop"], false);
}
