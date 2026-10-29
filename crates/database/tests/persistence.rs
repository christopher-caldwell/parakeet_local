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
        &[],
        &[],
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
    let (a, b) = tokio::join!(
        store.correct(&c, 0, &[], &[]),
        store.correct(&other, 0, &[], &[])
    );
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
        _: &parakeet_application::processing::RunProcessingInput,
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
        store.clone(),
    );
    let unsupported = service
        .run(RunProcessingInput {
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
            .run(RunProcessingInput {
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
                origin: None,
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
            more_ranges: vec![],
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
        admitted_example_id: None,
    };
    assert!(
        !store
            .resolve_suggestion(&stale, 0, None, 0, &[], &[])
            .await
            .unwrap()
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
        _: &parakeet_application::capture::CaptureRecordingInput,
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
        _: &parakeet_application::processing::RunProcessingInput,
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
        store.clone(),
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
        store.clone(),
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
        _: &parakeet_application::processing::RunProcessingInput,
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
        store.clone(),
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
            parakeet_application::meeting::RecordMeetingInput {
                capture: parakeet_application::capture::CaptureRecordingInput {
                    microphone_uid: None,
                    system_audio: true,
                    title: "Fixture capture".into(),
                    maximum_seconds: 1,
                    final_transcription: None,
                },
                transcribe_after_stop: true,
                skip_unavailable_model: false,
                analyze_speakers: false,
                matching_policy: None,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    let first = report.finalization.unwrap();
    assert_eq!(first.status, "completed");
    assert_eq!(first.track_ids, vec!["track"]);
    assert!(service.pending().await.unwrap().is_empty());
    assert_eq!(
        service
            .finalize(parakeet_application::meeting::FinalizeMeetingInput {
                id: ("r").to_string(),
                model: None
            })
            .await
            .unwrap(),
        first
    );
    assert_eq!(engine.calls.lock().unwrap().len(), 1);
    let d = store.detail("r").await.unwrap().unwrap();
    assert_eq!(d.recording.metadata["read_only"], true);
    assert_eq!(d.tracks[0].start_offset_ms, Some(2800));
    assert_eq!(d.runs.len(), 1);
    assert_eq!(d.runs[0].configuration["meeting_finalization_id"], first.id);
    assert!(
        service
            .finalize(parakeet_application::meeting::FinalizeMeetingInput {
                id: ("r").to_string(),
                model: Some(str::to_owned("different-model"))
            })
            .await
            .is_err()
    );
    // Existing settings JSON decodes safely; explicit UI preference persists separately.
    use parakeet_application::settings::{AudioPreferencesQueries, AudioPreferencesWrites};
    assert!(
        store
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
    let error = service
        .finalize(parakeet_application::meeting::FinalizeMeetingInput {
            id: ("r").to_string(),
            model: None,
        })
        .await
        .unwrap_err();
    let retained = error.retained().unwrap();
    assert_eq!(retained["recording_id"], "r");
    let pending = service.pending().await.unwrap().remove(0);
    assert_eq!(pending.status, "failed");
    assert_eq!(pending.completed.len(), 1);
    assert_eq!(pending.failures.len(), 1);
    assert_eq!(pending.track_ids.len(), 2);
    store.rename("r", "User title survives").await.unwrap();
    let done = service
        .finalize(parakeet_application::meeting::FinalizeMeetingInput {
            id: ("r").to_string(),
            model: None,
        })
        .await
        .unwrap();
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
    let failed = d.runs.iter().find(|r| r.status == "failed").unwrap();
    let retry = d
        .runs
        .iter()
        .find(|r| r.configuration["retry_of_run_id"] == failed.id)
        .unwrap();
    assert_eq!(retry.status, "completed");
    assert_eq!(
        retry.configuration["meeting_finalization_id"],
        failed.configuration["meeting_finalization_id"]
    );
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
    assert!(
        service
            .finalize(parakeet_application::meeting::FinalizeMeetingInput {
                id: ("r").to_string(),
                model: None
            })
            .await
            .is_err()
    );
    let plan = service.pending().await.unwrap().remove(0);
    assert_eq!(plan.status, "running");
    assert!(plan.completed.is_empty());
    let ownership = parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
        &tmp.path().join("test.db"),
    )
    .unwrap();
    let lease = ownership.try_acquire(&plan.id).await.unwrap().unwrap();
    assert!(matches!(
        service
            .finalize(parakeet_application::meeting::FinalizeMeetingInput {
                id: ("r").to_string(),
                model: None
            })
            .await,
        Err(parakeet_application::meeting::FinalizeMeetingError::Conflict(_))
    ));
    drop(lease);
    let done = service
        .finalize(parakeet_application::meeting::FinalizeMeetingInput {
            id: ("r").to_string(),
            model: None,
        })
        .await
        .unwrap();
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
        service
            .finalize(parakeet_application::meeting::FinalizeMeetingInput {
                id: ("r").to_string(),
                model: None
            })
            .await,
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
    let error = service
        .finalize(parakeet_application::meeting::FinalizeMeetingInput {
            id: ("r").to_string(),
            model: None,
        })
        .await
        .unwrap_err();
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
        store.clone(),
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
    let error = service
        .finalize(parakeet_application::meeting::FinalizeMeetingInput {
            id: ("r").to_string(),
            model: None,
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.retained().unwrap()["finalization"]["model_revision"],
        "test"
    );
    assert!(store.detail("r").await.unwrap().unwrap().runs.is_empty());
    assert!(engine.calls.lock().unwrap().is_empty());
}

#[derive(Default)]
struct GatedControlEngine {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
    active: std::sync::atomic::AtomicBool,
}
#[async_trait::async_trait]
impl parakeet_application::processing::SpeechProcessing for GatedControlEngine {
    async fn process(
        &self,
        _: &parakeet_application::processing::RunProcessingInput,
        _: &AudioTrack,
        _: &ProcessingRun,
    ) -> Result<parakeet_application::processing::EngineOutput, parakeet_application::CapabilityError>
    {
        unreachable!("controlled test path")
    }
    async fn process_controlled(
        &self,
        _: &parakeet_application::processing::RunProcessingInput,
        track: &AudioTrack,
        run: &ProcessingRun,
        control: parakeet_application::processing_control::ProcessingControl,
    ) -> Result<parakeet_application::processing::EngineOutput, parakeet_application::CapabilityError>
    {
        self.active.store(true, std::sync::atomic::Ordering::SeqCst);
        control.publish(
            parakeet_application::processing_control::ProcessingStage::Transcribing,
            5000,
            Some(10000),
            1,
            Some(50),
        );
        self.started.notify_one();
        self.release.notified().await;
        self.active
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(parakeet_application::processing::EngineOutput {
            segments: vec![TranscriptSegment {
                id: format!("{}:new", run.id),
                recording_id: run.recording_id.clone(),
                run_id: run.id.clone(),
                track_id: Some(track.id.clone()),
                ordinal: 0,
                start_ms: Some(2800),
                end_ms: Some(3800),
                text: "Unpublished cancelled output".into(),
                speaker_claim: None,
                evidence: json!({"simulated":true}),
            }],
            intervals: vec![],
            raw: json!({"simulated":true}),
            metrics: json!({}),
        })
    }
}
fn controlled_service(
    tmp: &tempfile::TempDir,
    store: std::sync::Arc<SqliteStore>,
    engine: std::sync::Arc<dyn parakeet_application::processing::SpeechProcessing>,
    writes: std::sync::Arc<dyn parakeet_application::processing::RunWrites>,
    control: std::sync::Arc<dyn parakeet_application::processing_control::RunControlWrites>,
) -> std::sync::Arc<parakeet_application::processing::ProcessingService> {
    std::sync::Arc::new(parakeet_application::processing::ProcessingService::new(
        store.clone(),
        writes,
        std::sync::Arc::new(FixtureModels),
        engine,
        store,
        std::sync::Arc::new(
            parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
                &tmp.path().join("test.db"),
            )
            .unwrap(),
        ),
        control,
    ))
}
fn controlled_request() -> parakeet_application::processing::RunProcessingInput {
    parakeet_application::processing::RunProcessingInput {
        recording_id: "r".into(),
        track_id: "track".into(),
        model: "fixture".into(),
        start_ms: 0,
        duration_ms: None,
        word_timestamps: false,
    }
}
async fn wait_settled(store: &SqliteStore, id: &str) -> ProcessingRun {
    use parakeet_application::jobs::ProcessingQueries;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let r = store.run(id).await.unwrap().unwrap();
            if r.status != "running" {
                break r;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn cancelling_and_dropping_waiter_keeps_worker_owned_until_exit() {
    use parakeet_application::{jobs::*, processing_control::*};
    use std::sync::{Arc, atomic::Ordering};
    let (tmp, store) = database().await;
    let mut b = bundle();
    b.tracks[0].available = true;
    store.import(&b).await.unwrap();
    let store = Arc::new(store);
    let engine = Arc::new(GatedControlEngine::default());
    let service = controlled_service(
        &tmp,
        store.clone(),
        engine.clone(),
        store.clone(),
        store.clone(),
    );
    let work = service.clone();
    let waiter = tokio::spawn(async move { work.run(controlled_request()).await });
    engine.started.notified().await;
    let id = store.pending_runs().await.unwrap()[0].id.clone();
    let control = ProcessingControlService::new(store.clone(), store.clone());
    assert!(control.cancel(&id).await.unwrap().requested);
    assert!(service.retry(&id).await.is_err());
    waiter.abort();
    let _ = waiter.await;
    let ownership = Arc::new(
        parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
            &tmp.path().join("test.db"),
        )
        .unwrap(),
    );
    assert!(ownership.try_acquire(&id).await.unwrap().is_none());
    let recovery = ProcessingRecoveryService::new(store.clone(), store.clone(), ownership.clone());
    assert_eq!(recovery.recover().await.unwrap().active, vec![id.clone()]);
    assert!(engine.active.load(Ordering::SeqCst));
    engine.release.notify_one();
    let r = wait_settled(&store, &id).await;
    assert_eq!(r.status, "interrupted");
    assert!(!engine.active.load(Ordering::SeqCst));
    assert!(r.configuration["cancellation_requested_at"].is_string());
    assert_eq!(
        r.result["unpublished_segments"].as_array().unwrap().len(),
        1
    );
    let d = store.detail("r").await.unwrap().unwrap();
    assert_eq!(d.segments.len(), 1);
    assert_eq!(d.segments[0].text, "Original evidence");
    assert!(!control.cancel(&id).await.unwrap().requested);
}
struct TerminalCancellationRace(std::sync::Arc<SqliteStore>);
#[async_trait::async_trait]
impl parakeet_application::processing::RunWrites for TerminalCancellationRace {
    async fn start(&self, r: &ProcessingRun) -> Result<(), parakeet_application::PersistenceError> {
        self.0.start(r).await
    }
    async fn finish(
        &self,
        r: &ProcessingRun,
        s: &[TranscriptSegment],
    ) -> Result<(), parakeet_application::PersistenceError> {
        use parakeet_application::processing_control::RunControlWrites;
        if r.status == "completed" {
            assert!(self.0.request_cancellation(&r.id, "terminal-race").await?);
        }
        self.0.finish(r, s).await
    }
}
#[tokio::test]
async fn cancellation_wins_atomic_completion_race_without_publishing_output() {
    use parakeet_application::jobs::ProcessingQueries;
    use std::sync::Arc;
    let (tmp, store) = database().await;
    let mut b = bundle();
    b.tracks[0].available = true;
    store.import(&b).await.unwrap();
    let store = Arc::new(store);
    let service = controlled_service(
        &tmp,
        store.clone(),
        Arc::new(UncertainEngine),
        Arc::new(TerminalCancellationRace(store.clone())),
        store.clone(),
    );
    assert!(matches!(
        service.run(controlled_request()).await,
        Err(parakeet_application::processing::ProcessingError::Cancelled(_))
    ));
    let d = store.detail("r").await.unwrap().unwrap();
    assert_eq!(d.runs[0].status, "interrupted");
    assert_eq!(
        d.runs[0].configuration["cancellation_requested_at"],
        "terminal-race"
    );
    assert!(store.pending_runs().await.unwrap().is_empty());
    assert_eq!(d.segments[0].text, "Original evidence");
}
#[tokio::test]
async fn progress_is_monotonic_and_old_runs_refuse_cancellation() {
    use parakeet_application::jobs::ProcessingQueries;
    use parakeet_application::processing_control::*;
    use std::sync::Arc;
    let (_tmp, store) = database().await;
    store.import(&bundle()).await.unwrap();
    let store = Arc::new(store);
    let mut r = ProcessingRun {
        id: uuid::Uuid::new_v4().to_string(),
        recording_id: "r".into(),
        kind: "transcription".into(),
        model: "fixture".into(),
        model_revision: "test".into(),
        status: "running".into(),
        started_at: "fixture".into(),
        completed_at: None,
        configuration: json!({"ownership_protocol":"file-lock-v1"}),
        result: serde_json::Value::Null,
        error: None,
    };
    store.start(&r).await.unwrap();
    let service = ProcessingControlService::new(store.clone(), store.clone());
    assert!(matches!(
        service.cancel(&r.id).await,
        Err(CancelProcessingRunError::Conflict(_))
    ));
    r.id = uuid::Uuid::new_v4().to_string();
    r.configuration["control_protocol"] = json!(CONTROL_PROTOCOL);
    store.start(&r).await.unwrap();
    let c = ProcessingControl::default();
    c.publish(
        ProcessingStage::Transcribing,
        1000,
        Some(10000),
        0,
        Some(10),
    );
    let old = c.snapshot();
    c.publish(ProcessingStage::Transcribing, 5000, Some(10000), 1, None);
    let next = c.snapshot();
    assert!(store.save_progress(&r.id, &next).await.unwrap());
    assert!(!store.save_progress(&r.id, &old).await.unwrap());
    assert_eq!(
        store.run(&r.id).await.unwrap().unwrap().result["progress"]["audio_completed_ms"],
        5000
    );
    assert!(service.cancel(&r.id).await.unwrap().requested);
    assert!(service.cancel(&r.id).await.unwrap().requested);
    assert_eq!(
        store.control_state(&r.id).await.unwrap().unwrap().status,
        "running"
    );
}

struct BrokenProgress {
    store: std::sync::Arc<SqliteStore>,
    failed: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl parakeet_application::processing_control::RunControlWrites for BrokenProgress {
    async fn save_progress(
        &self,
        _: &str,
        _: &parakeet_application::processing_control::RunProgress,
    ) -> Result<bool, parakeet_application::PersistenceError> {
        self.failed.notify_one();
        Err(parakeet_application::PersistenceError::new(
            std::io::Error::other("injected progress storage failure"),
        ))
    }
    async fn request_cancellation(
        &self,
        id: &str,
        at: &str,
    ) -> Result<bool, parakeet_application::PersistenceError> {
        self.store.request_cancellation(id, at).await
    }
}
#[tokio::test]
async fn progress_write_failure_drains_worker_before_releasing_ownership() {
    use parakeet_application::jobs::*;
    use std::sync::{Arc, atomic::Ordering};
    let (tmp, store) = database().await;
    let mut b = bundle();
    b.tracks[0].available = true;
    store.import(&b).await.unwrap();
    let store = Arc::new(store);
    let engine = Arc::new(GatedControlEngine::default());
    let progress = Arc::new(BrokenProgress {
        store: store.clone(),
        failed: tokio::sync::Notify::new(),
    });
    let service = controlled_service(
        &tmp,
        store.clone(),
        engine.clone(),
        store.clone(),
        progress.clone(),
    );
    let work = service.clone();
    let task = tokio::spawn(async move { work.run(controlled_request()).await });
    engine.started.notified().await;
    progress.failed.notified().await;
    let id = store.pending_runs().await.unwrap()[0].id.clone();
    assert!(!task.is_finished());
    assert!(engine.active.load(Ordering::SeqCst));
    let ownership = parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
        &tmp.path().join("test.db"),
    )
    .unwrap();
    assert!(ownership.try_acquire(&id).await.unwrap().is_none());
    engine.release.notify_one();
    assert!(matches!(
        task.await.unwrap(),
        Err(parakeet_application::processing::ProcessingError::Persistence(_))
    ));
    let run = store.run(&id).await.unwrap().unwrap();
    assert_eq!(run.status, "interrupted");
    assert!(run.error.unwrap().contains("persistence failed"));
    assert!(!engine.active.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancellation_stops_remaining_sources_in_both_recording_workflows() {
    use parakeet_application::{
        capture::CaptureService, jobs::ProcessingQueries, meeting::MeetingService,
        processing::TranscribeRecordingInput, processing_control::ProcessingControlService,
    };
    use std::sync::Arc;
    for meeting in [false, true] {
        let (tmp, store) = database().await;
        let mut b = bundle();
        b.tracks[0].available = true;
        let mut second = b.tracks[0].clone();
        second.id = "second-track".into();
        second.kind = TrackKind::Microphone;
        b.tracks.push(second);
        store.import(&b).await.unwrap();
        let store = Arc::new(store);
        let engine = Arc::new(GatedControlEngine::default());
        let processing = controlled_service(
            &tmp,
            store.clone(),
            engine.clone(),
            store.clone(),
            store.clone(),
        );
        let service = MeetingService::new(
            Arc::new(CaptureService::new(Arc::new(FixtureCapture), store.clone())),
            store.clone(),
            store.clone(),
            Arc::new(FixtureModels),
            processing.clone(),
            Arc::new(
                parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
                    &tmp.path().join("test.db"),
                )
                .unwrap(),
            ),
        );
        let waiter = tokio::spawn(async move {
            if meeting {
                assert!(
                    service
                        .finalize(parakeet_application::meeting::FinalizeMeetingInput {
                            id: ("r").to_string(),
                            model: None
                        })
                        .await
                        .is_err()
                );
            } else {
                let error = processing
                    .recording(TranscribeRecordingInput {
                        id: "r".into(),
                        model: "fixture".into(),
                    })
                    .await
                    .unwrap_err();
                assert_eq!(error.kind(), "cancelled");
            }
        });
        engine.started.notified().await;
        let id = store.pending_runs().await.unwrap()[0].id.clone();
        ProcessingControlService::new(store.clone(), store.clone())
            .cancel(&id)
            .await
            .unwrap();
        engine.release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .unwrap()
            .unwrap();
        let detail = store.detail("r").await.unwrap().unwrap();
        assert_eq!(
            detail.runs.len(),
            1,
            "cancellation must not start the other source"
        );
        assert_eq!(detail.runs[0].status, "interrupted");
        assert_eq!(detail.segments.len(), 1, "original evidence survives");
        if meeting {
            assert_eq!(
                detail.recording.metadata["meeting_finalization"]["status"],
                "failed"
            );
            assert_eq!(
                detail.recording.metadata["meeting_finalization"]["completed"],
                json!([])
            );
        }
    }
}

#[tokio::test]
async fn storage_policy_defaults_is_versioned_and_read_only_preview_does_not_initialize_models() {
    use parakeet_application::storage::*;
    use parakeet_domain::storage::RetentionPolicy;
    use std::sync::Arc;
    let (tmp, store) = database().await;
    let store = Arc::new(store);
    let inspection = Arc::new(parakeet_infrastructure::storage::LocalStorageInspection {
        root: tmp.path().into(),
        database: tmp.path().join("test.db"),
    });
    let service = Arc::new(StorageService::new(
        store.clone(),
        store.clone(),
        inspection,
    ));
    let first = service.policy().await.unwrap();
    assert_eq!(first.revision, 0);
    assert_eq!(first.policy, RetentionPolicy::default());
    let preview = service.preview(None).await.unwrap();
    assert!(preview.candidates.is_empty());
    assert!(!preview.automatic_cleanup);
    assert!(!preview.execution_available);
    assert!(!tmp.path().join("models").exists());
    let p = RetentionPolicy {
        completed_scratch_after_days: Some(30),
    };
    let saved = service
        .save_policy(parakeet_application::storage::SaveRetentionPolicyInput {
            policy: p,
            expected_revision: 0,
        })
        .await
        .unwrap();
    assert_eq!(saved.revision, 1);
    let (a, b) = tokio::join!(
        service.save_policy(parakeet_application::storage::SaveRetentionPolicyInput {
            policy: RetentionPolicy {
                completed_scratch_after_days: Some(7)
            },
            expected_revision: 1
        }),
        service.save_policy(parakeet_application::storage::SaveRetentionPolicyInput {
            policy: RetentionPolicy {
                completed_scratch_after_days: Some(90)
            },
            expected_revision: 1
        })
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(service.policy().await.unwrap().revision, 2);
    assert!(matches!(
        service
            .save_policy(parakeet_application::storage::SaveRetentionPolicyInput {
                policy: RetentionPolicy::default(),
                expected_revision: 1
            })
            .await,
        Err(SaveRetentionPolicyError::Conflict(_))
    ));
    assert!(
        service
            .save_policy(parakeet_application::storage::SaveRetentionPolicyInput {
                policy: RetentionPolicy {
                    completed_scratch_after_days: Some(0)
                },
                expected_revision: 2
            })
            .await
            .is_err()
    );
    assert_eq!(service.policy().await.unwrap().revision, 2);
    let readonly = SqliteStore::new(open_read_only(&tmp.path().join("test.db")).await.unwrap());
    assert_eq!(readonly.retention_policy().await.unwrap().revision, 2);
    assert!(
        readonly
            .save_retention_policy(&RetentionPolicy::default(), 2, "invalid read-only mutation")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn storage_preview_only_admits_old_owned_completed_scratch_without_losing_evidence() {
    use parakeet_application::storage::*;
    use parakeet_domain::storage::{RetentionPolicy, StorageProtection};
    use std::sync::Arc;
    let (tmp, store) = database().await;
    let audio = tmp.path().join("original.wav");
    std::fs::write(&audio, b"read-only source fixture").unwrap();
    let original = std::fs::read(&audio).unwrap();
    let mut eligible = None;
    for name in [
        "eligible",
        "active",
        "failed",
        "interrupted",
        "recent",
        "unresolved",
        "pending",
        "speaker",
        "correction",
        "unmanaged",
    ] {
        let mut b = bundle();
        b.recording.id = name.into();
        b.recording.source_key = format!("fixture:{name}");
        b.recording.title_override = Some(format!("User title {name}"));
        if name == "pending" {
            b.recording.metadata["meeting_finalization"] = json!({"status":"failed"});
        }
        b.tracks[0].id = format!("track-{name}");
        b.tracks[0].recording_id = name.into();
        b.tracks[0].path = audio.to_str().unwrap().into();
        b.tracks[0].available = true;
        b.segments[0].id = format!("segment-{name}");
        b.segments[0].recording_id = name.into();
        b.segments[0].run_id = format!("import-{name}");
        b.segments[0].track_id = Some(b.tracks[0].id.clone());
        store.import(&b).await.unwrap();
        let mut run = ProcessingRun {
            id: uuid::Uuid::new_v4().to_string(),
            recording_id: name.into(),
            kind: "transcription".into(),
            model: "fixture".into(),
            model_revision: "fixture".into(),
            status: "running".into(),
            started_at: "2020-01-01T00:00:00Z".into(),
            completed_at: None,
            configuration: if name == "unmanaged" {
                json!({})
            } else {
                json!({"ownership_protocol":"file-lock-v1"})
            },
            result: json!({"raw":"original result remains in the database"}),
            error: None,
        };
        store.start(&run).await.unwrap();
        if name != "active" {
            run.status = if ["failed", "interrupted"].contains(&name) {
                name
            } else {
                "completed"
            }
            .into();
            run.completed_at = Some(match name {
                "recent" => parakeet_application::now(),
                "unresolved" => "not a timestamp".into(),
                _ => "2020-01-02T00:00:00Z".into(),
            });
            store.finish(&run, &[]).await.unwrap();
        }
        let work = tmp
            .path()
            .join("models/work")
            .join(&run.id)
            .join("whisper-chunks");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("chunk-0000.json"), b"checkpoint fixture").unwrap();
        if name == "eligible" {
            eligible = Some(run.id.clone());
            std::fs::write(work.join("chunk-0001.json.partial"), b"uncertain").unwrap();
            std::fs::hard_link(&audio, work.parent().unwrap().join("audio.f32")).unwrap();
            let request = work.parent().unwrap().join("request.json");
            std::fs::write(&request, b"linked request fixture").unwrap();
            std::fs::hard_link(&request, tmp.path().join("other-link")).unwrap();
        }
        if name == "speaker" {
            store
                .create_profile(&SpeakerProfile {
                    id: "profile".into(),
                    name: "Fixture person".into(),
                    created_at: "fixture".into(),
                })
                .await
                .unwrap();
            store
                .enroll(&VoiceExample {
                    id: "example".into(),
                    profile_id: "profile".into(),
                    recording_id: name.into(),
                    track_id: format!("track-{name}"),
                    start_ms: 0,
                    end_ms: 1000,
                    model: "fixture".into(),
                    revision: "1".into(),
                    capture_domain: "fixture".into(),
                    vector: vec![1., 0.],
                    verified_at: "human fixture".into(),
                    origin: None,
                })
                .await
                .unwrap();
        }
        if name == "correction" {
            store
                .correct(
                    &Correction {
                        id: "correction".into(),
                        segment_id: format!("segment-{name}"),
                        profile_id: None,
                        replacement_text: Some("Human correction".into()),
                        action: "text".into(),
                        created_at: "fixture".into(),
                    },
                    0,
                    &[],
                    &[],
                )
                .await
                .unwrap();
        }
    }
    let unknown = tmp
        .path()
        .join("models/work")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&unknown).unwrap();
    std::fs::write(unknown.join("audio.f32"), b"unknown owner").unwrap();
    for path in [
        "models/weights/model.bin",
        "captures/recoverable/source.caf",
        "playback/current.wav",
        "clipboard-recovery/journal.json",
    ] {
        let p = tmp.path().join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"protected fixture").unwrap();
    }
    let store = Arc::new(store);
    let service = StorageService::new(
        store.clone(),
        store.clone(),
        Arc::new(parakeet_infrastructure::storage::LocalStorageInspection {
            root: tmp.path().into(),
            database: tmp.path().join("test.db"),
        }),
    );
    let readonly_default = service.preview(None).await.unwrap();
    assert!(readonly_default.candidates.is_empty());
    let preview = service
        .preview(Some(RetentionPolicy {
            completed_scratch_after_days: Some(1),
        }))
        .await
        .unwrap();
    assert!(preview.hypothetical);
    assert_eq!(preview.saved_policy.revision, 0);
    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(preview.candidates[0].run_id, eligible.unwrap());
    assert_eq!(
        preview.candidate_totals.logical_bytes,
        b"checkpoint fixture".len() as u64
    );
    for reason in [
        StorageProtection::ActiveRun,
        StorageProtection::RecoverableRun,
        StorageProtection::TooRecentOrFuture,
        StorageProtection::CompletionTimeUnknown,
        StorageProtection::PendingMeeting,
        StorageProtection::SpeakerReference,
        StorageProtection::ManualCorrection,
        StorageProtection::UnknownRun,
        StorageProtection::LinkedAudio,
        StorageProtection::MultipleHardLinks,
        StorageProtection::UncertainArtifact,
        StorageProtection::Models,
        StorageProtection::RecordedAudio,
        StorageProtection::Playback,
        StorageProtection::Recovery,
    ] {
        assert!(
            preview.protected.contains_key(&reason),
            "missing {reason:?}"
        );
    }
    let protected = preview
        .protected
        .values()
        .map(|s| s.logical_bytes)
        .sum::<u64>();
    assert_eq!(
        protected + preview.candidate_totals.logical_bytes,
        preview.usage.owned_totals.logical_bytes
    );
    assert_eq!(std::fs::read(&audio).unwrap(), original);
    assert_eq!(store.examples(None).await.unwrap().len(), 1);
    let corrected = store.detail("correction").await.unwrap().unwrap();
    assert_eq!(
        corrected.recording.title_override.as_deref(),
        Some("User title correction")
    );
    assert_eq!(corrected.corrections.len(), 1);
    assert!(!preview.execution_available);
    assert!(!preview.automatic_cleanup);
}

#[tokio::test]
async fn storage_accounting_separates_database_override_and_referenced_external_audio() {
    use parakeet_application::storage::StorageService;
    use std::sync::Arc;
    let (tmp, store) = database().await;
    let root = tmp.path().join("owned");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("owned.bin"), b"owned").unwrap();
    let outside = tmp.path().join("outside.wav");
    std::fs::write(&outside, b"external source bytes").unwrap();
    let mut b = bundle();
    b.tracks[0].path = outside.to_str().unwrap().into();
    b.tracks[0].available = true;
    store.import(&b).await.unwrap();
    let store = Arc::new(store);
    let service = StorageService::new(
        store.clone(),
        store,
        Arc::new(parakeet_infrastructure::storage::LocalStorageInspection {
            root: root.clone(),
            database: tmp.path().join("test.db"),
        }),
    );
    let report = service.usage().await.unwrap();
    assert_eq!(report.owned_totals.logical_bytes, 5);
    assert!(report.database_outside_root_bytes > 0);
    assert_eq!(
        report.referenced_external_file_bytes,
        b"external source bytes".len() as u64
    );
    assert_eq!(report.external_audio.len(), 1);
    assert_eq!(report.unresolved_audio_references, 0);
    assert!(report.complete);
    assert_eq!(std::fs::read(outside).unwrap(), b"external source bytes");
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 1);
}

#[tokio::test]
async fn regression_excerpt_must_not_hide_existing_complete_transcript() {
    let (_tmp, s) = database().await;
    s.import(&bundle()).await.unwrap();
    let mut run = ProcessingRun {
        id: "full".into(),
        recording_id: "r".into(),
        kind: "transcription".into(),
        model: "fixture".into(),
        model_revision: "v1".into(),
        status: "running".into(),
        started_at: "2020-01-01T00:00:00Z".into(),
        completed_at: None,
        configuration: json!({"track":{"id":"track"},"request":{"start_ms":0,"duration_ms":null}}),
        result: json!({}),
        error: None,
    };
    s.start(&run).await.unwrap();
    run.status = "completed".into();
    run.completed_at = Some("2020-01-01T00:00:01Z".into());
    let mut first = bundle().segments.remove(0);
    first.id = "full-first".into();
    first.run_id = "full".into();
    first.track_id = Some("track".into());
    first.start_ms = Some(0);
    first.end_ms = Some(1000);
    let mut last = first.clone();
    last.id = "full-last".into();
    last.start_ms = Some(9000);
    last.end_ms = Some(10000);
    last.ordinal = 1;
    s.finish(&run, &[first.clone(), last.clone()])
        .await
        .unwrap();
    assert_eq!(s.detail("r").await.unwrap().unwrap().segments.len(), 2);
    run.id = "excerpt".into();
    run.status = "running".into();
    run.completed_at = None;
    run.configuration["request"] = json!({"start_ms":5000,"duration_ms":1000});
    s.start(&run).await.unwrap();
    run.status = "completed".into();
    run.completed_at = Some("2020-01-01T00:00:02Z".into());
    let mut excerpt = first;
    excerpt.id = "excerpt-only".into();
    excerpt.run_id = "excerpt".into();
    excerpt.start_ms = Some(5000);
    excerpt.end_ms = Some(6000);
    s.finish(&run, &[excerpt]).await.unwrap();
    let visible = s.detail("r").await.unwrap().unwrap().segments;
    assert!(
        s.segment("full-last").await.unwrap().is_some(),
        "original evidence still exists"
    );
    assert!(
        visible.iter().any(|s| s.id == "full-last"),
        "a bounded excerpt replaced the entire current transcript: {:?}",
        visible.iter().map(|s| &s.id).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn regression_library_search_must_find_corrected_current_text() {
    let (_tmp, s) = database().await;
    s.import(&bundle()).await.unwrap();
    s.correct(
        &Correction {
            id: "corrected".into(),
            segment_id: "segment".into(),
            profile_id: None,
            replacement_text: Some("zebra-corrected-token".into()),
            action: "text".into(),
            created_at: "now".into(),
        },
        0,
        &[],
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        s.list(Some("zebra-corrected-token")).await.unwrap().len(),
        1,
        "current corrected transcript is invisible to library search"
    );
}

struct ReviewGoodEngine;
#[async_trait::async_trait]
impl parakeet_application::processing::SpeechProcessing for ReviewGoodEngine {
    async fn process(
        &self,
        _: &parakeet_application::processing::RunProcessingInput,
        track: &AudioTrack,
        run: &ProcessingRun,
    ) -> Result<parakeet_application::processing::EngineOutput, parakeet_application::CapabilityError>
    {
        let mut segment = bundle().segments.remove(0);
        segment.id = format!("{}-segment", run.id);
        segment.recording_id = run.recording_id.clone();
        segment.run_id = run.id.clone();
        segment.track_id = Some(track.id.clone());
        Ok(parakeet_application::processing::EngineOutput {
            segments: vec![segment],
            intervals: vec![],
            raw: json!([{"text":"synthetic fixture"}]),
            metrics: json!({"review_adapter":true}),
        })
    }
}
#[tokio::test]
async fn regression_full_transcription_must_report_missing_authoritative_source() {
    use std::sync::Arc;
    let (tmp, s) = database().await;
    let mut b = bundle();
    b.tracks[0].available = true;
    let mut missing = b.tracks[0].clone();
    missing.id = "missing-mic".into();
    missing.kind = TrackKind::Microphone;
    missing.available = false;
    b.tracks.push(missing);
    s.import(&b).await.unwrap();
    let s = Arc::new(s);
    let service = controlled_service(
        &tmp,
        s.clone(),
        Arc::new(ReviewGoodEngine),
        s.clone(),
        s.clone(),
    );
    let result = service
        .recording(parakeet_application::processing::TranscribeRecordingInput {
            id: "r".into(),
            model: "fixture".into(),
        })
        .await;
    assert!(
        result.is_err(),
        "full-transcription returned success with one authoritative source silently excluded"
    );
}

#[tokio::test]
async fn regression_shortcut_stream_failure_must_cancel_active_capture() {
    use parakeet_application::{capture::*, dictation::*, settings::*, *};
    use parakeet_domain::gestures::{InsertionTarget, Shortcut};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct Capture {
        started: Arc<tokio::sync::Notify>,
        stop: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    }
    #[async_trait::async_trait]
    impl AudioCapture for Capture {
        fn current_status(&self) -> serde_json::Value {
            json!({})
        }
        async fn devices(&self) -> Result<serde_json::Value, CapabilityError> {
            Ok(json!([]))
        }
        async fn permissions(&self) -> Result<serde_json::Value, CapabilityError> {
            Ok(json!({}))
        }
        async fn recoverable(&self) -> Result<serde_json::Value, CapabilityError> {
            Ok(json!([]))
        }
        async fn recover(&self, _: &std::path::Path) -> Result<ImportBundle, CapabilityError> {
            Err(CapabilityError::message("unused review capability"))
        }
        async fn record(
            &self,
            _: &CaptureRecordingInput,
            stop: Arc<AtomicBool>,
        ) -> Result<ImportBundle, CapabilityError> {
            *self.stop.lock().unwrap() = Some(stop.clone());
            self.started.notify_one();
            while !stop.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            Err(CapabilityError::message(
                "review capture stopped without audio or insertion",
            ))
        }
    }
    struct Stream {
        phase: u8,
        mode: &'static str,
        started: Arc<tokio::sync::Notify>,
    }
    #[async_trait::async_trait]
    impl ShortcutStream for Stream {
        async fn next_event(&mut self) -> Result<Option<ShortcutEvent>, CapabilityError> {
            if self.phase == 0 {
                self.phase = 1;
                return Ok(Some(ShortcutEvent::Down {
                    binding: "f6".into(),
                    target: InsertionTarget {
                        bundle_id: "review.synthetic".into(),
                        process_id: 1,
                        element_token: "synthetic".into(),
                    },
                }));
            }
            self.started.notified().await;
            match self.mode {
                "error" => Err(CapabilityError::message(
                    "synthetic native shortcut pipe failure",
                )),
                "eof" => Ok(None),
                _ => std::future::pending().await,
            }
        }
    }
    struct Keys(Arc<tokio::sync::Notify>, &'static str);
    #[async_trait::async_trait]
    impl GlobalShortcuts for Keys {
        async fn listen(&self, _: &[Shortcut]) -> Result<Box<dyn ShortcutStream>, CapabilityError> {
            Ok(Box::new(Stream {
                phase: 0,
                mode: self.1,
                started: self.0.clone(),
            }))
        }
    }
    struct NoExternalActions;
    #[async_trait::async_trait]
    impl TextInsertion for NoExternalActions {
        async fn insert(
            &self,
            _: &InsertionTarget,
            _: &str,
        ) -> Result<InsertionReport, CapabilityError> {
            panic!("review never inserts into a real application")
        }
    }
    #[async_trait::async_trait]
    impl TextTransformation for NoExternalActions {
        async fn transform(&self, _: &PromptProfile, _: &str) -> Result<String, CapabilityError> {
            panic!("review never invokes AI")
        }
    }
    impl DictationObserver for NoExternalActions {
        fn event(&self, _: serde_json::Value) {}
    }
    for mode in ["error", "eof", "stop", "caller_abort"] {
        let (tmp, s) = database().await;
        let s = Arc::new(s);
        let settings = Arc::new(SettingsService::new(
            s.clone(),
            s.clone(),
            Arc::new(NoExternalActions),
        ));
        settings
            .save_shortcuts(&[Shortcut {
                id: "f6".into(),
                key: "f6".into(),
                modifiers: vec![],
                enabled: true,
            }])
            .await
            .unwrap();
        let started = Arc::new(tokio::sync::Notify::new());
        let stop = Arc::new(Mutex::new(None));
        let dictation = Arc::new(DictationService::new(
            Arc::new(CaptureService::new(
                Arc::new(Capture {
                    started: started.clone(),
                    stop: stop.clone(),
                }),
                s.clone(),
            )),
            controlled_service(
                &tmp,
                s.clone(),
                Arc::new(ReviewGoodEngine),
                s.clone(),
                s.clone(),
            ),
            settings.clone(),
            Arc::new(NoExternalActions),
            s,
        ));
        let listener = DictationListener::new(
            Arc::new(Keys(started, mode)),
            settings,
            dictation,
            Arc::new(NoExternalActions),
        );
        let shutdown = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { listener.run("synthetic-mic".into(), shutdown).await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while stop.lock().unwrap().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if mode == "stop" {
            shutdown.store(true, Ordering::SeqCst);
        }
        if mode == "caller_abort" {
            task.abort();
        }
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), task)
            .await
            .unwrap();
        match mode {
            "caller_abort" => assert!(result.unwrap_err().is_cancelled()),
            "stop" => assert!(result.unwrap().is_ok()),
            _ => assert!(result.unwrap().is_err()),
        }
        let control = stop.lock().unwrap().clone().unwrap();
        assert!(
            control.load(Ordering::SeqCst),
            "{mode}: active dictation must be cancelled before listener exit"
        );
    }
}

#[tokio::test]
async fn coordinated_transcription_publishes_all_sources_together_and_search_ignores_history() {
    let (_tmp, s) = database().await;
    let mut b = bundle();
    b.tracks[0].available = true;
    s.import(&b).await.unwrap();
    let mut run = ProcessingRun {
        id: "batch-a".into(),
        recording_id: "r".into(),
        kind: "transcription".into(),
        model: "fixture".into(),
        model_revision: "v1".into(),
        status: "running".into(),
        started_at: "now".into(),
        completed_at: None,
        configuration: json!({"track":{"id":"track"},"request":{"start_ms":0},"publication_group":{"id":"batch","track_ids":["track","second"]}}),
        result: json!({}),
        error: None,
    };
    let mut segment = b.segments[0].clone();
    segment.id = "new-a".into();
    segment.run_id = run.id.clone();
    segment.text = "new batch token".into();
    s.start(&run).await.unwrap();
    run.status = "completed".into();
    run.completed_at = Some("now".into());
    s.finish(&run, &[segment.clone()]).await.unwrap();
    assert_eq!(
        s.detail("r").await.unwrap().unwrap().segments[0].id,
        "segment",
        "unfinished group must keep baseline"
    );
    assert!(s.list(Some("new batch token")).await.unwrap().is_empty());
    run.id = "batch-b".into();
    run.status = "running".into();
    run.configuration["track"]["id"] = json!("second");
    s.start(&run).await.unwrap();
    segment.id = "new-b".into();
    segment.run_id = run.id.clone();
    run.status = "completed".into();
    s.finish(&run, &[segment]).await.unwrap();
    assert_eq!(s.detail("r").await.unwrap().unwrap().segments.len(), 2);
    assert!(s.list(Some("Original evidence")).await.unwrap().is_empty());
    // Correction snapshots and Undo agree with the Domain resolver's current text.
    s.correct(
        &Correction {
            id: "edit-1".into(),
            segment_id: "new-a".into(),
            profile_id: None,
            replacement_text: Some("changed unique text".into()),
            action: "text".into(),
            created_at: "now".into(),
        },
        0,
        &[],
        &[],
    )
    .await
    .unwrap();
    assert_eq!(s.list(Some("changed unique text")).await.unwrap().len(), 1);
    s.correct(
        &Correction {
            id: "undo-1".into(),
            segment_id: "new-a".into(),
            profile_id: None,
            replacement_text: None,
            action: "undo".into(),
            created_at: "now".into(),
        },
        1,
        &[],
        &[],
    )
    .await
    .unwrap();
    assert!(
        s.list(Some("changed unique text"))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(s.list(Some("new batch token")).await.unwrap().len(), 1);
}
#[tokio::test]
async fn stale_schema_is_refused_without_migrating() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("test.db");
    create_empty(&p).await.unwrap();
    assert!(open_existing(&p).await.is_err());
    migrate(&p).await.unwrap();
    let pool = open_existing(&p).await.unwrap();
    sqlx::query(
        "DELETE FROM _sqlx_migrations WHERE version=(SELECT MAX(version) FROM _sqlx_migrations)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let error = open_existing(&p).await.unwrap_err().to_string();
    assert!(error.contains("db migrate"), "{error}");
    assert!(open_read_only(&p).await.is_err());
    assert!(verify(&p).await.is_ok());
}
struct UnverifiedModels;
#[async_trait::async_trait]
impl parakeet_application::LocalModels for UnverifiedModels {
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
        m.verified = false;
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
async fn meeting_without_a_verified_asr_model_records_audio_and_reports_the_skip() {
    use std::sync::{Arc, atomic::AtomicBool};
    let (tmp, store) = database().await;
    let store = Arc::new(store);
    let engine = Arc::new(MeetingEngine::default());
    let ownership = Arc::new(
        parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
            &tmp.path().join("test.db"),
        )
        .unwrap(),
    );
    let models = Arc::new(UnverifiedModels);
    let processing = Arc::new(parakeet_application::processing::ProcessingService::new(
        store.clone(),
        store.clone(),
        models.clone(),
        engine.clone(),
        store.clone(),
        ownership.clone(),
        store.clone(),
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
    let report = service
        .record(
            parakeet_application::meeting::RecordMeetingInput {
                capture: parakeet_application::capture::CaptureRecordingInput {
                    microphone_uid: None,
                    system_audio: true,
                    title: "Fixture capture".into(),
                    maximum_seconds: 1,
                    final_transcription: None,
                },
                transcribe_after_stop: true,
                skip_unavailable_model: true,
                analyze_speakers: false,
                matching_policy: None,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert!(report.finalization.is_none());
    assert!(report.transcription_skipped.is_some());
    assert!(store.detail(&report.recording_id).await.unwrap().is_some());
    assert!(engine.calls.lock().unwrap().is_empty());
}
