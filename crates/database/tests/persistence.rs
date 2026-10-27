use parakeet_application::{
    LibraryQueries, LibraryWrites,
    processing::RunWrites,
    speakers::{SpeakerQueries, SpeakerWrites},
};
use parakeet_database::*;
use parakeet_domain::*;
use serde_json::json;
async fn database() -> (tempfile::TempDir, SqliteStore) {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("test.db");
    create_empty(&p).await.unwrap();
    migrate(&p).await.unwrap();
    let store = SqliteStore::new(open_existing(&p).await.unwrap());
    (tmp, store)
}
fn bundle() -> ImportBundle {
    ImportBundle {
        recording: Recording {
            id: "r".into(),
            source_key: "original:r".into(),
            source_kind: "meeting".into(),
            source_title: "Original title".into(),
            title_override: None,
            created_at: "2026-01-01".into(),
            duration_ms: Some(10000),
            metadata: json!({"read_only":true}),
        },
        tracks: vec![AudioTrack {
            id: "track".into(),
            recording_id: "r".into(),
            kind: TrackKind::System,
            path: "/missing/source.m4a".into(),
            start_offset_ms: Some(2800),
            sample_rate: Some(48000),
            duration_ms: Some(10000),
            available: false,
            size_bytes: None,
            modified_unix_ms: None,
            provenance: json!({}),
        }],
        segments: vec![TranscriptSegment {
            id: "segment".into(),
            recording_id: "r".into(),
            run_id: "import:r".into(),
            track_id: None,
            ordinal: 0,
            start_ms: Some(2000),
            end_ms: Some(3000),
            text: "Original evidence".into(),
            speaker_claim: Some("Me".into()),
            evidence: json!({"verified":false}),
        }],
        original: json!({"raw":"retained"}),
        warnings: vec![],
    }
}
#[tokio::test]
async fn import_idempotency_title_search_and_read_only_runtime() {
    let (tmp, s) = database().await;
    assert!(s.import(&bundle()).await.unwrap());
    assert!(!s.import(&bundle()).await.unwrap());
    assert!(s.rename("r", "My title").await.unwrap());
    assert!(!s.rename("absent", "x").await.unwrap());
    s.import(&bundle()).await.unwrap();
    assert_eq!(
        s.detail("r").await.unwrap().unwrap().recording.title(),
        "My title"
    );
    assert_eq!(s.list(Some("evidence")).await.unwrap().len(), 1);
    let ro = SqliteStore::new(open_read_only(&tmp.path().join("test.db")).await.unwrap());
    assert!(ro.rename("r", "no").await.is_err());
    assert_eq!(ro.list(None).await.unwrap().len(), 1);
}
#[tokio::test]
async fn bad_import_rolls_back_and_concurrent_import_is_atomic() {
    let (_tmp, s) = database().await;
    let mut bad = bundle();
    bad.tracks[0].recording_id = "missing".into();
    assert!(s.import(&bad).await.is_err());
    assert!(s.list(None).await.unwrap().is_empty());
    let b = bundle();
    let (a, c) = tokio::join!(s.import(&b), s.import(&b));
    assert_ne!(a.unwrap(), c.unwrap());
}
#[tokio::test]
async fn completed_run_replaces_view_without_destroying_imported_evidence_or_corrections() {
    let (_tmp, s) = database().await;
    s.import(&bundle()).await.unwrap();
    s.create_profile(&SpeakerProfile {
        id: "person".into(),
        name: "Synthetic".into(),
        created_at: "now".into(),
    })
    .await
    .unwrap();
    s.correct(
        &Correction {
            id: "c".into(),
            segment_id: "segment".into(),
            profile_id: Some("person".into()),
            replacement_text: Some("Human correction".into()),
            action: "assign".into(),
            created_at: "now".into(),
        },
        0,
    )
    .await
    .unwrap();
    let mut run = ProcessingRun {
        id: "run".into(),
        recording_id: "r".into(),
        kind: "transcription".into(),
        model: "model".into(),
        model_revision: "1".into(),
        status: "running".into(),
        started_at: "now".into(),
        completed_at: None,
        configuration: json!({}),
        result: json!({}),
        error: None,
    };
    s.start(&run).await.unwrap();
    assert_eq!(
        s.detail("r").await.unwrap().unwrap().segments[0].id,
        "segment"
    );
    run.status = "completed".into();
    let mut segment = bundle().segments.remove(0);
    segment.id = "new".into();
    segment.run_id = "run".into();
    s.finish(&run, &[segment]).await.unwrap();
    let d = s.detail("r").await.unwrap().unwrap();
    assert_eq!(d.segments.len(), 1);
    assert_eq!(d.segments[0].id, "new");
    assert_eq!(d.corrections.len(), 1);
    assert_eq!(
        s.segment("segment").await.unwrap().unwrap().text,
        "Original evidence"
    );
    assert!(s.finish(&run, &[]).await.is_err());
    s.delete_profile("person").await.unwrap();
    assert!(
        s.corrections("segment").await.unwrap()[0]
            .profile_id
            .is_none()
    );
}
#[tokio::test]
async fn doctor_does_not_create_storage() {
    let t = tempfile::tempdir().unwrap();
    let absent = t.path().join("absent.db");
    assert!(verify(&absent).await.is_err());
    assert!(!absent.exists());
    assert!(open_read_only(&absent).await.is_err());
    assert!(!absent.exists());
}

#[tokio::test]
async fn foreign_database_is_refused_before_wal_or_migration() {
    use sqlx::sqlite::SqlitePoolOptions;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("foreign.db");
    let pool = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::query("CREATE TABLE original_evidence(content TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO original_evidence VALUES('unchanged')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let original = std::fs::read(&path).unwrap();
    assert!(open_existing(&path).await.is_err());
    assert!(migrate(&path).await.is_err());
    assert!(open_read_only(&path).await.is_err());
    assert_eq!(original, std::fs::read(&path).unwrap());
    assert!(!dir.path().join("foreign.db-wal").exists());
}
#[tokio::test]
async fn concurrent_corrections_compare_and_append() {
    let (_tmp, store) = database().await;
    store.import(&bundle()).await.unwrap();
    let c = Correction {
        id: "a".into(),
        segment_id: "segment".into(),
        profile_id: None,
        replacement_text: Some("one".into()),
        action: "text".into(),
        created_at: "now".into(),
    };
    let mut other = c.clone();
    other.id = "b".into();
    other.replacement_text = Some("two".into());
    let (a, b) = tokio::join!(store.correct(&c, 0), store.correct(&other, 0));
    assert_ne!(a.unwrap(), b.unwrap());
    assert_eq!(store.corrections("segment").await.unwrap().len(), 1);
}

struct FixtureModels;
#[async_trait::async_trait]
impl parakeet_application::LocalModels for FixtureModels {
    async fn selection(
        &self,
    ) -> Result<parakeet_application::ModelSelection, parakeet_application::CapabilityError> {
        Ok(Default::default())
    }
    async fn list(
        &self,
    ) -> Result<Vec<parakeet_application::ModelInfo>, parakeet_application::CapabilityError> {
        Ok(vec![])
    }
    async fn install(
        &self,
        _: &str,
    ) -> Result<parakeet_application::ModelInfo, parakeet_application::CapabilityError> {
        Err(parakeet_application::CapabilityError::message(
            "test does not install",
        ))
    }
    async fn verify(
        &self,
        id: &str,
    ) -> Result<parakeet_application::ModelInfo, parakeet_application::CapabilityError> {
        Ok(parakeet_application::ModelInfo {
            experimental_word_timestamps: false,
            id: id.into(),
            category: "asr".into(),
            revision: "test".into(),
            installed: true,
            verified: true,
            bytes: Some(1),
            license: "test".into(),
            path: None,
            detail: json!({}),
        })
    }
    async fn select(&self, _: &str) -> Result<(), parakeet_application::CapabilityError> {
        Ok(())
    }
    async fn select_embedding(&self, _: &str) -> Result<(), parakeet_application::CapabilityError> {
        Ok(())
    }
    async fn remove(&self, _: &str) -> Result<(), parakeet_application::CapabilityError> {
        Ok(())
    }
}
struct FixtureEngine;
#[async_trait::async_trait]
impl parakeet_application::processing::VoiceEmbeddings for FixtureEngine {
    async fn embed(
        &self,
        _: &parakeet_application::processing::EmbeddingRequest,
    ) -> Result<
        parakeet_application::processing::EmbeddingOutput,
        parakeet_application::CapabilityError,
    > {
        Ok(parakeet_application::processing::EmbeddingOutput {
            model: "test".into(),
            revision: "1".into(),
            vector: vec![1., 0.],
            metrics: json!({"mock":true}),
        })
    }
}
#[async_trait::async_trait]
impl parakeet_application::processing::SpeechProcessing for FixtureEngine {
    async fn process(
        &self,
        _: &parakeet_application::processing::ProcessRequest,
        _: &AudioTrack,
        _: &ProcessingRun,
    ) -> Result<parakeet_application::processing::EngineOutput, parakeet_application::CapabilityError>
    {
        let mut segment = bundle().segments.remove(0);
        segment.recording_id = "wrong-recording".into();
        Ok(parakeet_application::processing::EngineOutput {
            segments: vec![segment],
            intervals: vec![],
            raw: json!({"mock":true}),
            metrics: json!({}),
        })
    }
}
#[tokio::test]
async fn invalid_model_output_settles_failed_without_publishing_segments() {
    use parakeet_application::processing::*;
    use std::sync::Arc;
    let (_tmp, store) = database().await;
    let mut b = bundle();
    b.tracks[0].available = true;
    store.import(&b).await.unwrap();
    let store = Arc::new(store);
    let service = ProcessingService::new(
        store.clone(),
        store.clone(),
        Arc::new(FixtureModels),
        Arc::new(FixtureEngine),
        store.clone(),
        std::sync::Arc::new(
            parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
                &_tmp.path().join("test.db"),
            )
            .unwrap(),
        ),
    );
    let unsupported = service
        .run(ProcessRequest {
            recording_id: "r".into(),
            track_id: "track".into(),
            model: "fixture".into(),
            start_ms: 0,
            duration_ms: Some(5000),
            word_timestamps: true,
        })
        .await
        .unwrap_err();
    assert!(unsupported.to_string().contains("does not support"));
    assert!(store.detail("r").await.unwrap().unwrap().runs.is_empty());
    assert!(
        service
            .run(ProcessRequest {
                recording_id: "r".into(),
                track_id: "track".into(),
                model: "fixture".into(),
                start_ms: 0,
                duration_ms: Some(5000),
                word_timestamps: false,
            })
            .await
            .is_err()
    );
    let d = store.detail("r").await.unwrap().unwrap();
    assert_eq!(d.runs.len(), 1);
    assert_eq!(d.runs[0].status, "failed");
    assert!(d.runs[0].error.as_ref().unwrap().contains("outside"));
    assert_eq!(d.segments[0].id, "segment");
}
#[tokio::test]
async fn suggestion_resolution_is_durable_atomic_and_never_enrolls_predictions() {
    use parakeet_application::speakers::*;
    use parakeet_domain::recognition::MatchPolicy;
    use std::sync::Arc;
    let (_tmp, store) = database().await;
    let mut b = bundle();
    b.segments[0].track_id = Some("track".into());
    b.segments[0].start_ms = Some(3000);
    b.segments[0].end_ms = Some(4000);
    store.import(&b).await.unwrap();
    store
        .create_profile(&SpeakerProfile {
            id: "person".into(),
            name: "Synthetic fixture".into(),
            created_at: "now".into(),
        })
        .await
        .unwrap();
    for id in ["e1", "e2"] {
        store
            .enroll(&VoiceExample {
                id: id.into(),
                profile_id: "person".into(),
                recording_id: "r".into(),
                track_id: "track".into(),
                start_ms: if id == "e1" { 0 } else { 5000 },
                end_ms: if id == "e1" { 5000 } else { 10000 },
                model: "test".into(),
                revision: "1".into(),
                capture_domain: "system".into(),
                vector: vec![1., 0.],
                verified_at: "fixture-confirmation".into(),
            })
            .await
            .unwrap();
    }
    let store = Arc::new(store);
    let service = SpeakerService::new(
        store.clone(),
        store.clone(),
        store.clone(),
        Arc::new(FixtureEngine),
        Arc::new(FixtureModels),
    );
    let s = service
        .suggest(SuggestSpeakerInput {
            recording_id: "r".into(),
            track_id: "track".into(),
            start_ms: 0,
            end_ms: 5000,
            policy: MatchPolicy {
                minimum_score: 0.9,
                minimum_margin: 0.1,
                minimum_examples: 2,
            },
        })
        .await
        .unwrap();
    assert_eq!(s.decision.suggested_profile.as_deref(), Some("person"));
    assert_eq!(
        service
            .review_suggestion(&s.id)
            .await
            .unwrap()
            .eligible_segments
            .len(),
        1
    );
    let mut narrow = s.clone();
    narrow.id = "narrow-comparison".into();
    narrow.start_ms = 300;
    narrow.end_ms = 3500;
    store.save_suggestion(&narrow).await.unwrap();
    // The segment is 200–1200 ms in track time. Partial coverage must not be offered.
    assert!(
        service
            .review_suggestion(&narrow.id)
            .await
            .unwrap()
            .eligible_segments
            .is_empty()
    );
    assert!(
        service
            .resolve_suggestion(ResolveSuggestionInput {
                id: narrow.id,
                action: "accept".into(),
                segment_id: Some("segment".into()),
                profile_id: None
            })
            .await
            .is_err()
    );
    assert_eq!(service.suggestions(Some("r")).await.unwrap().len(), 2);
    service
        .resolve_suggestion(ResolveSuggestionInput {
            id: s.id.clone(),
            action: "accept".into(),
            segment_id: Some("segment".into()),
            profile_id: None,
        })
        .await
        .unwrap();
    assert_eq!(
        store.corrections("segment").await.unwrap()[0]
            .profile_id
            .as_deref(),
        Some("person")
    );
    assert_eq!(store.examples(None).await.unwrap().len(), 2);
    service
        .resolve_suggestion(ResolveSuggestionInput {
            id: s.id.clone(),
            action: "undo".into(),
            segment_id: None,
            profile_id: None,
        })
        .await
        .unwrap();
    assert!(
        store
            .corrections("segment")
            .await
            .unwrap()
            .last()
            .unwrap()
            .profile_id
            .is_none()
    );
    assert_eq!(
        store
            .suggestion(&s.id)
            .await
            .unwrap()
            .unwrap()
            .resolutions
            .len(),
        2
    );
    let stale = SuggestionResolution {
        id: "stale".into(),
        suggestion_id: s.id.clone(),
        action: "reject".into(),
        profile_id: None,
        segment_id: None,
        correction_id: None,
        created_at: "now".into(),
    };
    assert!(!store.resolve_suggestion(&stale, 0, None, 0).await.unwrap());
    assert_eq!(
        store
            .suggestion(&s.id)
            .await
            .unwrap()
            .unwrap()
            .resolutions
            .len(),
        2
    );
    service
        .resolve_suggestion(ResolveSuggestionInput {
            id: s.id,
            action: "reject".into(),
            segment_id: None,
            profile_id: None,
        })
        .await
        .unwrap();
    assert_eq!(store.examples(None).await.unwrap().len(), 2);
}

#[tokio::test]
async fn cluster_edits_and_repeated_undo_preserve_original_runs() {
    use parakeet_application::cluster_edits::*;
    use std::sync::Arc;
    let (_tmp, store) = database().await;
    store.import(&bundle()).await.unwrap();
    let intervals = vec![
        SpeakerInterval {
            start_ms: 0,
            end_ms: 1000,
            speaker: "a".into(),
            track_id: Some("track".into()),
        },
        SpeakerInterval {
            start_ms: 200,
            end_ms: 700,
            speaker: "b".into(),
            track_id: Some("track".into()),
        },
    ];
    let mut raw = ProcessingRun {
        id: "raw".into(),
        recording_id: "r".into(),
        kind: "diarization".into(),
        model: "fixture".into(),
        model_revision: "1".into(),
        status: "running".into(),
        started_at: "now".into(),
        completed_at: None,
        configuration: json!({"track":{"id":"track"}}),
        result: json!({}),
        error: None,
    };
    store.start(&raw).await.unwrap();
    raw.status = "completed".into();
    raw.result = json!({"intervals":intervals,"raw":{"untouched":true}});
    store.finish(&raw, &[]).await.unwrap();
    let store = Arc::new(store);
    let service = ClusterEditingService::new(
        store.clone(),
        store.clone(),
        Arc::new(
            parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
                &_tmp.path().join("test.db"),
            )
            .unwrap(),
        ),
    );
    let reviewed = service
        .inspect(InspectClustersInput {
            recording_id: "r".into(),
            track_id: "track".into(),
            run_id: Some(raw.id.clone()),
        })
        .await
        .unwrap();
    assert_eq!(reviewed.clusters.len(), 2);
    assert!(!reviewed.can_undo);
    assert!(
        service
            .inspect(InspectClustersInput {
                recording_id: "r".into(),
                track_id: "other".into(),
                run_id: None
            })
            .await
            .is_err()
    );
    let split = service
        .edit(EditClustersInput {
            recording_id: "r".into(),
            run_id: raw.id.clone(),
            edit: ClusterEdit::Split {
                speaker: "a".into(),
                at_ms: 500,
            },
        })
        .await
        .unwrap();
    assert_eq!(split.result["intervals"].as_array().unwrap().len(), 3);
    let new = split.result["intervals"][2]["speaker"]
        .as_str()
        .unwrap()
        .to_string();
    let merged = service
        .edit(EditClustersInput {
            recording_id: "r".into(),
            run_id: split.id.clone(),
            edit: ClusterEdit::Merge {
                speakers: vec!["a".into(), new],
            },
        })
        .await
        .unwrap();
    assert_eq!(merged.result["intervals"].as_array().unwrap().len(), 2);
    let undo = service
        .edit(EditClustersInput {
            recording_id: "r".into(),
            run_id: merged.id,
            edit: ClusterEdit::Undo,
        })
        .await
        .unwrap();
    assert_eq!(undo.result["intervals"], split.result["intervals"]);
    let undo_again = service
        .edit(EditClustersInput {
            recording_id: "r".into(),
            run_id: undo.id,
            edit: ClusterEdit::Undo,
        })
        .await
        .unwrap();
    assert_eq!(undo_again.result["intervals"], raw.result["intervals"]);
    assert!(
        service
            .edit(EditClustersInput {
                recording_id: "r".into(),
                run_id: undo_again.id,
                edit: ClusterEdit::Undo
            })
            .await
            .is_err()
    );
    let d = store.detail("r").await.unwrap().unwrap();
    assert_eq!(
        d.runs.iter().find(|r| r.id == "raw").unwrap().result,
        raw.result
    );
}

struct FixtureCapture;
#[async_trait::async_trait]
impl parakeet_application::capture::AudioCapture for FixtureCapture {
    fn current_status(&self) -> serde_json::Value {
        json!({"mock":true})
    }
    async fn devices(&self) -> Result<serde_json::Value, parakeet_application::CapabilityError> {
        Ok(json!({"mock":true}))
    }
    async fn permissions(
        &self,
    ) -> Result<serde_json::Value, parakeet_application::CapabilityError> {
        Ok(json!({"mock":true}))
    }
    async fn record(
        &self,
        _: &parakeet_application::capture::CaptureRequest,
        _: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<ImportBundle, parakeet_application::CapabilityError> {
        let mut b = bundle();
        b.tracks[0].available = true;
        Ok(b)
    }
    async fn recoverable(
        &self,
    ) -> Result<serde_json::Value, parakeet_application::CapabilityError> {
        Ok(json!([]))
    }
    async fn recover(
        &self,
        _: &std::path::Path,
    ) -> Result<ImportBundle, parakeet_application::CapabilityError> {
        Err(parakeet_application::CapabilityError::message(
            "no test recovery",
        ))
    }
}
#[derive(Default)]
struct InsertionSpy(std::sync::atomic::AtomicUsize);
#[async_trait::async_trait]
impl parakeet_application::dictation::TextInsertion for InsertionSpy {
    async fn insert(
        &self,
        _: &parakeet_domain::gestures::InsertionTarget,
        _: &str,
    ) -> Result<
        parakeet_application::dictation::InsertionReport,
        parakeet_application::CapabilityError,
    > {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(parakeet_application::dictation::InsertionReport {
            method: "fixture".into(),
            verification: "fixture".into(),
            clipboard_status: None,
        })
    }
}
#[async_trait::async_trait]
impl parakeet_application::settings::TextTransformation for InsertionSpy {
    async fn transform(
        &self,
        _: &PromptProfile,
        _: &str,
    ) -> Result<String, parakeet_application::CapabilityError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok("should not be reached".into())
    }
}
struct UncertainEngine;
#[async_trait::async_trait]
impl parakeet_application::processing::SpeechProcessing for UncertainEngine {
    async fn process(
        &self,
        _: &parakeet_application::processing::ProcessRequest,
        _: &AudioTrack,
        _: &ProcessingRun,
    ) -> Result<parakeet_application::processing::EngineOutput, parakeet_application::CapabilityError>
    {
        Ok(parakeet_application::processing::EngineOutput {
            segments: vec![],
            intervals: vec![],
            raw: json!([{"text":"Potentially fabricated speech","evidence":{"uncertain_speech":true}}]),
            metrics: json!({"mock":true}),
        })
    }
}
async fn dictate_fixture(
    cancel: bool,
) -> (
    tempfile::TempDir,
    SqliteStore,
    parakeet_application::dictation::DictationReceipt,
    usize,
) {
    use parakeet_application::{
        capture::CaptureService, dictation::*, processing::ProcessingService, settings::*,
    };
    use std::sync::Arc;
    let (tmp, store) = database().await;
    let store = Arc::new(store);
    let spy = Arc::new(InsertionSpy::default());
    let settings = Arc::new(SettingsService::new(
        store.clone(),
        store.clone(),
        spy.clone(),
    ));
    settings
        .save_prompt(&PromptProfile {
            id: "fixture".into(),
            bundle_id: "com.example.fixture".into(),
            application_name: "Test only".into(),
            enabled: true,
            prompt: "test".into(),
            executable: "/test/mock-not-executed".into(),
            arguments: vec![],
            route: "local".into(),
            authorized: true,
            timeout_seconds: 5,
        })
        .await
        .unwrap();
    let capture = Arc::new(CaptureService::new(Arc::new(FixtureCapture), store.clone()));
    let processing = Arc::new(ProcessingService::new(
        store.clone(),
        store.clone(),
        Arc::new(FixtureModels),
        Arc::new(UncertainEngine),
        store.clone(),
        Arc::new(
            parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
                &tmp.path().join("test.db"),
            )
            .unwrap(),
        ),
    ));
    let service = DictationService::new(capture, processing, settings, spy.clone(), store.clone());
    let control = DictationControl::default();
    if cancel {
        control.cancel();
    }
    let receipt = service
        .dictate(
            DictateSpeechInput {
                microphone_uid: "fixture-device".into(),
                target: parakeet_domain::gestures::InsertionTarget {
                    bundle_id: "com.example.fixture".into(),
                    process_id: 1,
                    element_token: "fixture".into(),
                },
            },
            control,
        )
        .await
        .unwrap();
    let calls = spy.0.load(std::sync::atomic::Ordering::SeqCst);
    (tmp, (*store).clone(), receipt, calls)
}
#[tokio::test]
async fn cancelled_dictation_never_runs_models_ai_or_insertion() {
    use parakeet_application::dictation::DictationQueries;
    let (_tmp, store, receipt, calls) = dictate_fixture(true).await;
    assert_eq!(calls, 0);
    assert_eq!(receipt.status, "cancelled");
    assert!(store.detail("r").await.unwrap().unwrap().runs.is_empty());
    assert_eq!(
        store.dictation(&receipt.id).await.unwrap().unwrap().status,
        "cancelled"
    );
}
#[tokio::test]
async fn uncertain_dictation_retains_text_without_ai_or_automatic_insertion() {
    use parakeet_application::dictation::DictationQueries;
    let (_tmp, store, receipt, calls) = dictate_fixture(false).await;
    assert_eq!(calls, 0);
    assert_eq!(receipt.status, "retained_after_failure");
    assert_eq!(receipt.raw, "Potentially fabricated speech");
    assert!(receipt.failure.unwrap().contains("uncertain"));
    assert_eq!(
        store.dictations().await.unwrap()[0].raw,
        "Potentially fabricated speech"
    );
}

#[tokio::test]
async fn job_recovery_preserves_active_legacy_and_prior_evidence() {
    use parakeet_application::jobs::*;
    use std::sync::Arc;
    let (tmp, store) = database().await;
    store.import(&bundle()).await.unwrap();
    let store = Arc::new(store);
    let ownership = Arc::new(
        parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
            &tmp.path().join("test.db"),
        )
        .unwrap(),
    );
    let run = ProcessingRun {
        id: uuid::Uuid::new_v4().to_string(),
        recording_id: "r".into(),
        kind: "transcription".into(),
        model: "fixture".into(),
        model_revision: "test".into(),
        status: "running".into(),
        started_at: "2026-01-01".into(),
        completed_at: None,
        configuration: json!({"ownership_protocol":OWNERSHIP_PROTOCOL,"request":{"recording_id":"r","track_id":"track","model":"fixture","start_ms":0,"duration_ms":5000}}),
        result: json!({"partial_evidence":"retain"}),
        error: None,
    };
    let lease = ownership.try_acquire(&run.id).await.unwrap().unwrap();
    store.start(&run).await.unwrap();
    let mut legacy = run.clone();
    legacy.id = uuid::Uuid::new_v4().to_string();
    legacy.configuration = json!({});
    store.start(&legacy).await.unwrap();
    let service = ProcessingRecoveryService::new(store.clone(), store.clone(), ownership);
    let report = service.recover().await.unwrap();
    assert_eq!(report.active, vec![run.id.clone()]);
    assert_eq!(report.unmanaged, vec![legacy.id.clone()]);
    assert!(report.interrupted.is_empty());
    drop(lease);
    let report = service.recover().await.unwrap();
    assert_eq!(report.interrupted, vec![run.id.clone()]);
    let recovered = ProcessingQueries::run(&*store, &run.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.status, "interrupted");
    assert_eq!(recovered.result, run.result);
    assert_eq!(
        recovered.configuration["request"],
        run.configuration["request"]
    );
    assert_eq!(
        store.detail("r").await.unwrap().unwrap().segments[0].text,
        "Original evidence"
    );
    assert_eq!(
        ProcessingQueries::run(&*store, &legacy.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "running"
    );
    assert!(service.recover().await.unwrap().interrupted.is_empty());
    let mut late = run.clone();
    late.status = "completed".into();
    assert!(store.finish(&late, &[]).await.is_err());
}

#[tokio::test]
async fn retry_links_new_run_and_refuses_live_completed_or_changed_model() {
    use parakeet_application::{jobs::*, processing::*};
    use std::sync::Arc;
    let (tmp, store) = database().await;
    let mut b = bundle();
    b.tracks[0].available = true;
    store.import(&b).await.unwrap();
    let store = Arc::new(store);
    let ownership = Arc::new(
        parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
            &tmp.path().join("test.db"),
        )
        .unwrap(),
    );
    let service = ProcessingService::new(
        store.clone(),
        store.clone(),
        Arc::new(FixtureModels),
        Arc::new(UncertainEngine),
        store.clone(),
        ownership,
    );
    let mut prior = ProcessingRun {
        id: uuid::Uuid::new_v4().to_string(),
        recording_id: "r".into(),
        kind: "transcription".into(),
        model: "fixture".into(),
        model_revision: "test".into(),
        status: "running".into(),
        started_at: "now".into(),
        completed_at: None,
        configuration: json!({"request":{"recording_id":"r","track_id":"track","model":"selected-asr","start_ms":0,"duration_ms":5000}}),
        result: json!({}),
        error: None,
    };
    store.start(&prior).await.unwrap();
    assert!(matches!(
        service.retry(&prior.id).await,
        Err(RetryProcessingError::Conflict(_))
    ));
    prior.status = "failed".into();
    prior.completed_at = Some("then".into());
    prior.error = Some("fixture failure".into());
    store.finish(&prior, &[]).await.unwrap();
    let retried = service.retry(&prior.id).await.unwrap();
    assert_ne!(retried.id, prior.id);
    assert_eq!(retried.status, "completed");
    assert_eq!(retried.configuration["retry_of_run_id"], prior.id);
    assert_eq!(retried.model, "fixture");
    assert_eq!(
        ProcessingQueries::run(&*store, &prior.id)
            .await
            .unwrap()
            .unwrap()
            .error,
        prior.error
    );
    assert!(service.retry(&retried.id).await.is_err());
    let mut changed = prior.clone();
    changed.id = uuid::Uuid::new_v4().to_string();
    changed.status = "running".into();
    changed.model_revision = "old-weights".into();
    store.start(&changed).await.unwrap();
    changed.status = "failed".into();
    store.finish(&changed, &[]).await.unwrap();
    assert!(
        service
            .retry(&changed.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("model revision changed")
    );
    assert_eq!(store.detail("r").await.unwrap().unwrap().runs.len(), 3);
}

#[tokio::test]
async fn audio_relink_verifies_content_is_dry_run_safe_and_preserves_evidence() {
    use parakeet_application::audio_locations::*;
    use parakeet_infrastructure::audio::LocalAudioIdentity;
    use std::sync::Arc;
    let (tmp, store) = database().await;
    let original = tmp.path().join("original.wav");
    std::fs::write(&original, b"fixture bytes retained exactly").unwrap();
    let reader = Arc::new(LocalAudioIdentity);
    let identity = reader.identify(&original).await.unwrap();
    let mut b = bundle();
    b.tracks[0].path = identity.path;
    b.tracks[0].size_bytes = Some(identity.size_bytes);
    b.tracks[0].modified_unix_ms = Some(identity.modified_unix_ms);
    b.tracks[0].available = true;
    store.import(&b).await.unwrap();
    store.rename("r", "User title").await.unwrap();
    let store = Arc::new(store);
    let service = AudioLocationService::new(store.clone(), store.clone(), reader);
    let anchored = service
        .fingerprint(FingerprintAudioInput {
            recording_id: "r".into(),
            track_id: "track".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        anchored.provenance["content_sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    let backup = tmp.path().join("moved.wav");
    std::fs::rename(&original, &backup).unwrap();
    let input = RelinkAudioInput {
        recording_id: "r".into(),
        track_id: "track".into(),
        path: backup.clone(),
        dry_run: true,
    };
    let preview = service.relink(input.clone()).await.unwrap();
    assert!(preview.changed);
    assert!(preview.dry_run);
    assert_eq!(
        store.detail("r").await.unwrap().unwrap().tracks[0].path,
        anchored.path
    );
    let mut input = input;
    input.dry_run = false;
    let saved = service.relink(input.clone()).await.unwrap();
    let detail = store.detail("r").await.unwrap().unwrap();
    assert_eq!(detail.recording.title(), "User title");
    assert_eq!(detail.segments[0], b.segments[0]);
    assert_eq!(
        detail.tracks[0].start_offset_ms,
        b.tracks[0].start_offset_ms
    );
    assert_eq!(detail.tracks[0].id, "track");
    assert_eq!(
        saved.track.path,
        backup.canonicalize().unwrap().to_string_lossy()
    );
    assert!(!service.relink(input).await.unwrap().changed);
    assert_eq!(
        saved.track.provenance["locator_history"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!store.replace_locator(&anchored, &anchored).await.unwrap());
    let wrong = tmp.path().join("wrong.wav");
    std::fs::write(&wrong, b"different file content entirely").unwrap();
    assert!(matches!(
        service
            .relink(RelinkAudioInput {
                recording_id: "r".into(),
                track_id: "track".into(),
                path: wrong,
                dry_run: false
            })
            .await,
        Err(RelinkAudioError::Conflict(_))
    ));
    assert_eq!(
        std::fs::read(backup).unwrap(),
        b"fixture bytes retained exactly"
    );
}

#[tokio::test]
async fn missing_unfingerprinted_audio_cannot_be_replaced_by_a_guess() {
    use parakeet_application::audio_locations::*;
    use std::sync::Arc;
    let (tmp, store) = database().await;
    store.import(&bundle()).await.unwrap();
    let store = Arc::new(store);
    let service = AudioLocationService::new(
        store.clone(),
        store.clone(),
        Arc::new(parakeet_infrastructure::audio::LocalAudioIdentity),
    );
    let replacement = tmp.path().join("unrelated.wav");
    std::fs::write(&replacement, b"unrelated").unwrap();
    assert!(matches!(
        service
            .relink(RelinkAudioInput {
                recording_id: "r".into(),
                track_id: "track".into(),
                path: replacement,
                dry_run: false
            })
            .await,
        Err(RelinkAudioError::Unverifiable(_))
    ));
    assert_eq!(
        store.detail("r").await.unwrap().unwrap().tracks[0],
        bundle().tracks[0]
    );
}

#[derive(Default)]
struct MeetingEngine {
    calls: std::sync::Mutex<Vec<String>>,
    fail_once: std::sync::Mutex<Option<String>>,
}
#[async_trait::async_trait]
impl parakeet_application::processing::SpeechProcessing for MeetingEngine {
    async fn process(
        &self,
        _: &parakeet_application::processing::ProcessRequest,
        track: &AudioTrack,
        _: &ProcessingRun,
    ) -> Result<parakeet_application::processing::EngineOutput, parakeet_application::CapabilityError>
    {
        self.calls.lock().unwrap().push(track.id.clone());
        let mut failure = self.fail_once.lock().unwrap();
        if failure.as_deref() == Some(track.id.as_str()) {
            *failure = None;
            return Err(parakeet_application::CapabilityError::message(
                "injected source failure",
            ));
        }
        Ok(parakeet_application::processing::EngineOutput {
            segments: vec![],
            intervals: vec![],
            raw: json!({"simulated":true}),
            metrics: json!({}),
        })
    }
}
fn meeting_service(
    tmp: &tempfile::TempDir,
    store: std::sync::Arc<SqliteStore>,
    engine: std::sync::Arc<MeetingEngine>,
    writes: std::sync::Arc<dyn parakeet_application::meeting::MeetingFinalizationWrites>,
) -> parakeet_application::meeting::MeetingService {
    use std::sync::Arc;
    let ownership = Arc::new(
        parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
            &tmp.path().join("test.db"),
        )
        .unwrap(),
    );
    let models = Arc::new(FixtureModels);
    let processing = Arc::new(parakeet_application::processing::ProcessingService::new(
        store.clone(),
        store.clone(),
        models.clone(),
        engine,
        store.clone(),
        ownership.clone(),
    ));
    parakeet_application::meeting::MeetingService::new(
        Arc::new(parakeet_application::capture::CaptureService::new(
            Arc::new(FixtureCapture),
            store.clone(),
        )),
        store,
        writes,
        models,
        processing,
        ownership,
    )
}
#[tokio::test]
async fn meeting_capture_saves_a_durable_plan_and_finalization_is_idempotent() {
    use std::sync::{Arc, atomic::AtomicBool};
    let (tmp, store) = database().await;
    let store = Arc::new(store);
    let engine = Arc::new(MeetingEngine::default());
    let service = meeting_service(&tmp, store.clone(), engine.clone(), store.clone());
    let report = service
        .record(
            parakeet_application::capture::CaptureRequest {
                microphone_uid: None,
                system_audio: true,
                title: "Fixture capture".into(),
                maximum_seconds: 1,
                final_transcription: None,
            },
            true,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    let first = report.finalization.unwrap();
    assert_eq!(first.status, "completed");
    assert_eq!(first.track_ids, vec!["track"]);
    assert!(service.pending().await.unwrap().is_empty());
    assert_eq!(service.finalize("r", None).await.unwrap(), first);
    assert_eq!(engine.calls.lock().unwrap().len(), 1);
    let d = store.detail("r").await.unwrap().unwrap();
    assert_eq!(d.recording.metadata["read_only"], true);
    assert_eq!(d.tracks[0].start_offset_ms, Some(2800));
    assert_eq!(d.runs.len(), 1);
    assert_eq!(d.runs[0].configuration["meeting_finalization_id"], first.id);
    assert!(
        service
            .finalize("r", Some("different-model"))
            .await
            .is_err()
    );
    // Existing settings JSON decodes safely; explicit UI preference persists separately.
    use parakeet_application::settings::{AudioPreferencesQueries, AudioPreferencesWrites};
    assert!(
        !store
            .audio_preferences()
            .await
            .unwrap()
            .transcribe_meetings_after_stop
    );
    let p = AudioPreferences {
        transcribe_meetings_after_stop: true,
        ..Default::default()
    };
    store.save_audio_preferences(&p).await.unwrap();
    assert!(
        store
            .audio_preferences()
            .await
            .unwrap()
            .transcribe_meetings_after_stop
    );
}
#[tokio::test]
async fn meeting_partial_failure_resumes_only_failed_sources_and_keeps_title() {
    use std::sync::Arc;
    let (tmp, store) = database().await;
    let mut b = bundle();
    b.tracks[0].available = true;
    let mut second = b.tracks[0].clone();
    second.id = "second".into();
    second.kind = TrackKind::Microphone;
    b.tracks.push(second);
    let mut playback = b.tracks[0].clone();
    playback.id = "playback".into();
    playback.kind = TrackKind::Playback;
    b.tracks.push(playback);
    store.import(&b).await.unwrap();
    let store = Arc::new(store);
    let engine = Arc::new(MeetingEngine::default());
    *engine.fail_once.lock().unwrap() = Some("second".into());
    let service = meeting_service(&tmp, store.clone(), engine.clone(), store.clone());
    let error = service.finalize("r", None).await.unwrap_err();
    let retained = error.retained().unwrap();
    assert_eq!(retained["recording_id"], "r");
    let pending = service.pending().await.unwrap().remove(0);
    assert_eq!(pending.status, "failed");
    assert_eq!(pending.completed.len(), 1);
    assert_eq!(pending.failures.len(), 1);
    assert_eq!(pending.track_ids.len(), 2);
    store.rename("r", "User title survives").await.unwrap();
    let done = service.finalize("r", None).await.unwrap();
    assert_eq!(done.status, "completed");
    assert_eq!(done.attempts, 2);
    let calls = engine.calls.lock().unwrap().clone();
    assert_eq!(calls.iter().filter(|id| id.as_str() == "track").count(), 1);
    assert_eq!(calls.iter().filter(|id| id.as_str() == "second").count(), 2);
    assert!(!calls.contains(&"playback".into()));
    let d = store.detail("r").await.unwrap().unwrap();
    assert_eq!(d.recording.title(), "User title survives");
    assert_eq!(d.runs.len(), 3);
    assert_eq!(d.runs.iter().filter(|r| r.status == "failed").count(), 1);
    assert_eq!(d.recording.metadata["read_only"], true);
}
struct LostMeetingCheckpoint {
    store: std::sync::Arc<SqliteStore>,
    writes: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl parakeet_application::meeting::MeetingFinalizationWrites for LostMeetingCheckpoint {
    async fn replace_finalization(
        &self,
        id: &str,
        expected: Option<&parakeet_application::meeting::MeetingFinalization>,
        next: &parakeet_application::meeting::MeetingFinalization,
    ) -> Result<bool, parakeet_application::PersistenceError> {
        if self
            .writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 2
        {
            return Err(parakeet_application::PersistenceError::new(
                std::io::Error::other("injected checkpoint interruption"),
            ));
        }
        self.store.replace_finalization(id, expected, next).await
    }
}
#[tokio::test]
async fn meeting_checkpoint_loss_reuses_durable_run_and_refuses_an_active_owner() {
    use parakeet_application::{jobs::ProcessingOwnership, meeting::MeetingFinalizationWrites};
    use std::sync::Arc;
    let (tmp, store) = database().await;
    let mut b = bundle();
    b.tracks[0].available = true;
    store.import(&b).await.unwrap();
    let store = Arc::new(store);
    let engine = Arc::new(MeetingEngine::default());
    let writes = Arc::new(LostMeetingCheckpoint {
        store: store.clone(),
        writes: std::sync::atomic::AtomicUsize::new(0),
    });
    let service = meeting_service(&tmp, store.clone(), engine.clone(), writes);
    assert!(service.finalize("r", None).await.is_err());
    let plan = service.pending().await.unwrap().remove(0);
    assert_eq!(plan.status, "running");
    assert!(plan.completed.is_empty());
    let ownership = parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
        &tmp.path().join("test.db"),
    )
    .unwrap();
    let lease = ownership.try_acquire(&plan.id).await.unwrap().unwrap();
    assert!(matches!(
        service.finalize("r", None).await,
        Err(parakeet_application::meeting::FinalizeMeetingError::Conflict(_))
    ));
    drop(lease);
    let done = service.finalize("r", None).await.unwrap();
    assert_eq!(done.completed.len(), 1);
    assert_eq!(engine.calls.lock().unwrap().len(), 1);
    let mut forged = done.clone();
    forged.completed[0].run_id = "not-a-real-run".into();
    assert!(
        store
            .replace_finalization("r", Some(&done), &forged)
            .await
            .unwrap()
    );
    assert!(matches!(
        service.finalize("r", None).await,
        Err(parakeet_application::meeting::FinalizeMeetingError::Conflict(_))
    ));
    assert!(
        !store
            .replace_finalization("r", Some(&done), &done)
            .await
            .unwrap()
    );
}
#[tokio::test]
async fn meeting_missing_source_is_failure_even_when_playback_exists() {
    use std::sync::Arc;
    let (tmp, store) = database().await;
    let mut b = bundle();
    let mut playback = b.tracks[0].clone();
    playback.id = "playback".into();
    playback.kind = TrackKind::Playback;
    playback.available = true;
    b.tracks.push(playback);
    store.import(&b).await.unwrap();
    let store = Arc::new(store);
    let engine = Arc::new(MeetingEngine::default());
    let service = meeting_service(&tmp, store.clone(), engine.clone(), store.clone());
    let error = service.finalize("r", None).await.unwrap_err();
    let retained = error.retained().unwrap();
    assert_eq!(retained["finalization"]["status"], "failed");
    assert_eq!(retained["finalization"]["failures"][0]["track_id"], "track");
    assert!(engine.calls.lock().unwrap().is_empty());
}

struct MeetingRevisionRace(std::sync::atomic::AtomicUsize);
#[async_trait::async_trait]
impl parakeet_application::LocalModels for MeetingRevisionRace {
    async fn selection(
        &self,
    ) -> Result<parakeet_application::ModelSelection, parakeet_application::CapabilityError> {
        FixtureModels.selection().await
    }
    async fn list(
        &self,
    ) -> Result<Vec<parakeet_application::ModelInfo>, parakeet_application::CapabilityError> {
        FixtureModels.list().await
    }
    async fn install(
        &self,
        id: &str,
    ) -> Result<parakeet_application::ModelInfo, parakeet_application::CapabilityError> {
        FixtureModels.install(id).await
    }
    async fn verify(
        &self,
        id: &str,
    ) -> Result<parakeet_application::ModelInfo, parakeet_application::CapabilityError> {
        let mut m = FixtureModels.verify(id).await?;
        if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 2 {
            m.revision = "changed-between-preflight-and-start".into();
        }
        Ok(m)
    }
    async fn select(&self, id: &str) -> Result<(), parakeet_application::CapabilityError> {
        FixtureModels.select(id).await
    }
    async fn select_embedding(
        &self,
        id: &str,
    ) -> Result<(), parakeet_application::CapabilityError> {
        FixtureModels.select_embedding(id).await
    }
    async fn remove(&self, id: &str) -> Result<(), parakeet_application::CapabilityError> {
        FixtureModels.remove(id).await
    }
}
#[tokio::test]
async fn meeting_revision_change_between_preflight_and_run_is_refused() {
    use std::sync::Arc;
    let (tmp, store) = database().await;
    let mut b = bundle();
    b.tracks[0].available = true;
    store.import(&b).await.unwrap();
    let store = Arc::new(store);
    let engine = Arc::new(MeetingEngine::default());
    let ownership = Arc::new(
        parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
            &tmp.path().join("test.db"),
        )
        .unwrap(),
    );
    let models = Arc::new(MeetingRevisionRace(std::sync::atomic::AtomicUsize::new(0)));
    let processing = Arc::new(parakeet_application::processing::ProcessingService::new(
        store.clone(),
        store.clone(),
        models.clone(),
        engine.clone(),
        store.clone(),
        ownership.clone(),
    ));
    let service = parakeet_application::meeting::MeetingService::new(
        Arc::new(parakeet_application::capture::CaptureService::new(
            Arc::new(FixtureCapture),
            store.clone(),
        )),
        store.clone(),
        store.clone(),
        models,
        processing,
        ownership,
    );
    let error = service.finalize("r", None).await.unwrap_err();
    assert_eq!(
        error.retained().unwrap()["finalization"]["model_revision"],
        "test"
    );
    assert!(store.detail("r").await.unwrap().unwrap().runs.is_empty());
    assert!(engine.calls.lock().unwrap().is_empty());
}
