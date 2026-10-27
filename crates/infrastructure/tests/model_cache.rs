use parakeet_application::{LocalModels, RecordingSource, processing::*};
use parakeet_domain::ProcessingRun;
use parakeet_infrastructure::{
    artifacts::MacParakeetArtifacts, models::ModelFiles, speech::LocalSpeech,
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

/// Explicit opt-in; uses installed local model copies and public audio, never downloads.
#[tokio::test]
#[ignore = "requires explicit local model probe root and public audio; includes 75s idle expiry"]
async fn real_model_reuse_verification_and_idle_expiry() {
    let root = PathBuf::from(
        std::env::var_os("PARAKEET_MODEL_PROBE_ROOT").expect("explicit model copies root"),
    );
    let audio = PathBuf::from(
        std::env::var_os("PARAKEET_MODEL_PROBE_AUDIO").expect("explicit public audio"),
    );
    let models = Arc::new(ModelFiles::new(root.clone()));
    assert!(models.verify("whisper-base.en").await.unwrap().verified);
    assert!(
        models
            .verify("speakrs-community-1-cpu")
            .await
            .unwrap()
            .verified
    );
    let bundle = MacParakeetArtifacts.import_file(&audio).await.unwrap();
    let track = &bundle.tracks[0];
    let engine = LocalSpeech::new(models.clone(), PathBuf::from("unused-native-helper"));
    let request = ProcessRequest {
        recording_id: bundle.recording.id.clone(),
        track_id: track.id.clone(),
        model: "whisper-base.en".into(),
        start_ms: 0,
        duration_ms: Some(track.duration_ms.unwrap().min(20_000)),
        word_timestamps: false,
    };
    let run = ProcessingRun {
        id: parakeet_application::new_id(),
        recording_id: request.recording_id.clone(),
        kind: "transcription".into(),
        model: request.model.clone(),
        model_revision: "probe".into(),
        status: "running".into(),
        started_at: parakeet_application::now(),
        completed_at: None,
        configuration: json!({}),
        result: json!({}),
        error: None,
    };
    let cold = engine.process(&request, track, &run).await.unwrap();
    let warm = engine.process(&request, track, &run).await.unwrap();
    assert_eq!(cold.metrics["model_context_reused"], false);
    assert_eq!(warm.metrics["model_context_reused"], true);
    assert_eq!(
        cold.segments
            .iter()
            .map(|s| (&s.text, s.start_ms, s.end_ms))
            .collect::<Vec<_>>(),
        warm.segments
            .iter()
            .map(|s| (&s.text, s.start_ms, s.end_ms))
            .collect::<Vec<_>>()
    );
    let mut word_request = request.clone();
    word_request.word_timestamps = true;
    let word_first = engine.process(&word_request, track, &run).await.unwrap();
    let word_second = engine.process(&word_request, track, &run).await.unwrap();
    assert_eq!(word_first.metrics["model_context_reused"], true);
    assert_eq!(word_second.metrics["model_context_reused"], true);
    let word_evidence = |output: &EngineOutput| {
        output
            .segments
            .iter()
            .map(|s| s.evidence["words"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(word_evidence(&word_first), word_evidence(&word_second));
    assert!(word_first.segments.iter().any(|s| {
        s.evidence["words"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    }));
    let segment_again = engine.process(&request, track, &run).await.unwrap();
    assert_eq!(
        cold.segments
            .iter()
            .map(|s| (&s.text, s.start_ms, s.end_ms))
            .collect::<Vec<_>>(),
        segment_again
            .segments
            .iter()
            .map(|s| (&s.text, s.start_ms, s.end_ms))
            .collect::<Vec<_>>()
    );
    let embedding = EmbeddingRequest {
        track: track.clone(),
        start_ms: 0,
        end_ms: track.duration_ms.unwrap().min(10_000),
        model: "speakrs-community-1-cpu".into(),
    };
    let ecold = engine.embed(&embedding).await.unwrap();
    let ewarm = engine.embed(&embedding).await.unwrap();
    assert_eq!(ecold.metrics["model_context_reused"], false);
    assert_eq!(ewarm.metrics["model_context_reused"], true);
    assert!(parakeet_domain::recognition::cosine(&ecold.vector, &ewarm.vector).unwrap() > 0.999999);
    let receipt = root.join("whisper-base.en/verified.json");
    let saved = std::fs::read(&receipt).unwrap();
    std::fs::remove_file(&receipt).unwrap();
    assert!(engine.process(&request, track, &run).await.is_err());
    std::fs::write(&receipt, saved).unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(75)).await;
    let expired = engine.process(&request, track, &run).await.unwrap();
    let eexpired = engine.embed(&embedding).await.unwrap();
    assert_eq!(expired.metrics["model_context_reused"], false);
    assert_eq!(eexpired.metrics["model_context_reused"], false);
    println!(
        "{}",
        json!({"asr_cold":cold.metrics,"asr_warm":warm.metrics,"embedding_cold":ecold.metrics,"embedding_warm":ewarm.metrics,"asr_identical_text_and_times":true,"word_timing_repeat_stable":true,"switching_back_preserves_segment_output":true,"embedding_cosine_above_0_999999":true,"invalid_verification_refused_while_cached":true,"idle_expiry_tested_seconds":75,"both_models_reloaded_after_expiry":true})
    );
}
