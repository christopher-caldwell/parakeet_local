//! Real SQLite/filesystem/ownership adapters with deterministic inference, not model accuracy tests.
use async_trait::async_trait;
use parakeet_application::{
    analysis::*, audio_locations::*, capture::*, meeting::*, processing::*, speakers::*, *,
};
use parakeet_database::*;
use parakeet_domain::{recognition::MatchPolicy, *};
use parakeet_infrastructure::{audio::LocalAudioIdentity, jobs::FileProcessingOwnership};
use serde_json::json;
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
#[derive(Default)]
struct Models {
    revision: Mutex<String>,
    wrong: AtomicBool,
}
#[async_trait]
impl LocalModels for Models {
    async fn selection(&self) -> Result<ModelSelection, CapabilityError> {
        Ok(ModelSelection {
            asr: "asr".into(),
            diarization: "diari".into(),
            embedding: "embed".into(),
        })
    }
    async fn list(&self) -> Result<Vec<ModelInfo>, CapabilityError> {
        Ok(vec![])
    }
    async fn select(&self, _: &str) -> Result<(), CapabilityError> {
        unreachable!()
    }
    async fn select_embedding(&self, _: &str) -> Result<(), CapabilityError> {
        unreachable!()
    }
    async fn install(&self, _: &str) -> Result<ModelInfo, CapabilityError> {
        panic!("no implicit download")
    }
    async fn remove(&self, _: &str) -> Result<(), CapabilityError> {
        unreachable!()
    }
    async fn verify(&self, id: &str) -> Result<ModelInfo, CapabilityError> {
        Ok(ModelInfo {
            id: id.into(),
            category: if id == "asr" || self.wrong.load(Ordering::SeqCst) {
                "asr"
            } else {
                "diarization"
            }
            .into(),
            revision: self.revision.lock().unwrap().clone(),
            installed: true,
            verified: true,
            bytes: Some(1),
            license: "fixture".into(),
            path: None,
            experimental_word_timestamps: false,
            detail: json!({"provides_voice_embeddings":id=="embed"}),
        })
    }
}
#[derive(Default)]
struct Engine {
    calls: Mutex<Vec<(String, String)>>,
    fail: Mutex<Option<String>>,
    embeddings: AtomicUsize,
    delay: AtomicBool,
}
#[async_trait]
impl SpeechProcessing for Engine {
    async fn process(
        &self,
        request: &RunProcessingInput,
        track: &AudioTrack,
        run: &ProcessingRun,
    ) -> Result<EngineOutput, CapabilityError> {
        self.calls
            .lock()
            .unwrap()
            .push((track.id.clone(), request.model.clone()));
        if self.delay.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        }
        if self.fail.lock().unwrap().as_ref() == Some(&format!("{}:{}", track.id, request.model)) {
            return Err(CapabilityError::message("fixture failure"));
        }
        let hash = LocalAudioIdentity
            .identify(Path::new(&track.path))
            .await?
            .sha256;
        let offset = track.start_offset_ms.unwrap();
        Ok(EngineOutput {
            segments: if request.model == "asr" {
                vec![TranscriptSegment {
                    id: format!("{}-s", run.id),
                    recording_id: run.recording_id.clone(),
                    run_id: run.id.clone(),
                    track_id: Some(track.id.clone()),
                    ordinal: 0,
                    start_ms: Some(offset + 500),
                    end_ms: Some(offset + 7500),
                    text: "Synthetic words".into(),
                    speaker_claim: None,
                    evidence: json!({"local_start_ms":500,"local_end_ms":7500}),
                }]
            } else {
                vec![]
            },
            intervals: if request.model == "diari" {
                // The microphone voice speaks after the system audio finishes; one that coincided
                // with system speech would be treated as echo and not offered for naming.
                let lead = if track.kind == TrackKind::Microphone {
                    7000
                } else {
                    0
                };
                vec![SpeakerInterval {
                    start_ms: offset + lead,
                    end_ms: offset + lead + 8000,
                    speaker: "anonymous-0".into(),
                    track_id: Some(track.id.clone()),
                }]
            } else {
                vec![]
            },
            raw: json!({"synthetic":true}),
            metrics: json!({"source_sha256":hash}),
        })
    }
}
#[async_trait]
impl VoiceEmbeddings for Engine {
    async fn embed(&self, _: &EmbeddingRequest) -> Result<EmbeddingOutput, CapabilityError> {
        self.embeddings.fetch_add(1, Ordering::SeqCst);
        Ok(EmbeddingOutput {
            model: "fixture-space".into(),
            revision: "1".into(),
            vector: vec![1., 0.],
            metrics: json!({"synthetic":true}),
        })
    }
}
struct Fixture {
    tmp: tempfile::TempDir,
    store: Arc<SqliteStore>,
    models: Arc<Models>,
    engine: Arc<Engine>,
    ownership: Arc<FileProcessingOwnership>,
    processing: Arc<ProcessingService>,
    analysis: Arc<MeetingAnalysisService>,
    bundle: ImportBundle,
}
impl Fixture {
    async fn new(two: bool, import: bool) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("db");
        create_empty(&db).await.unwrap();
        migrate(&db).await.unwrap();
        let store = Arc::new(SqliteStore::new(open_existing(&db).await.unwrap()));
        let mut tracks = vec![];
        for (id, offset, kind) in [
            ("system", 0, TrackKind::System),
            ("mic", 1250, TrackKind::Microphone),
        ]
        .into_iter()
        .take(if two { 2 } else { 1 })
        {
            let path = tmp.path().join(id);
            std::fs::write(&path, b"synthetic audio fixture, not real speech").unwrap();
            let identity = LocalAudioIdentity.identify(&path).await.unwrap();
            tracks.push(AudioTrack {
                id: id.into(),
                recording_id: "r".into(),
                kind,
                path: path.to_string_lossy().into(),
                start_offset_ms: Some(offset),
                sample_rate: Some(16000),
                duration_ms: Some(8000),
                available: true,
                size_bytes: Some(identity.size_bytes),
                modified_unix_ms: Some(identity.modified_unix_ms),
                provenance: json!({"content_sha256":identity.sha256}),
            });
        }
        let bundle = ImportBundle {
            recording: Recording {
                id: "r".into(),
                source_key: "fixture:r".into(),
                source_kind: "meeting".into(),
                source_title: "Synthetic meeting".into(),
                title_override: Some("My title".into()),
                created_at: now(),
                duration_ms: Some(9250),
                metadata: json!({"original":true}),
            },
            tracks,
            segments: vec![TranscriptSegment {
                id: "imported".into(),
                recording_id: "r".into(),
                run_id: "import:r".into(),
                track_id: None,
                ordinal: 0,
                start_ms: None,
                end_ms: None,
                text: "Imported baseline".into(),
                speaker_claim: None,
                evidence: json!({}),
            }],
            original: json!({"preserved":true}),
            warnings: vec![],
        };
        if import {
            store.import(&bundle).await.unwrap();
        }
        let models = Arc::new(Models {
            revision: Mutex::new("1".into()),
            wrong: AtomicBool::new(false),
        });
        let engine = Arc::new(Engine::default());
        let ownership = Arc::new(FileProcessingOwnership::for_database(&db).unwrap());
        let processing = Arc::new(ProcessingService::new(
            store.clone(),
            store.clone(),
            models.clone(),
            engine.clone(),
            store.clone(),
            ownership.clone(),
            store.clone(),
        ));
        let speakers = Arc::new(SpeakerService::new(
            store.clone(),
            store.clone(),
            store.clone(),
            engine.clone(),
            models.clone(),
        ));
        let analysis = Arc::new(MeetingAnalysisService::new(
            store.clone(),
            store.clone(),
            processing.clone(),
            speakers,
            models.clone(),
            Arc::new(LocalAudioIdentity),
            ownership.clone(),
        ));
        Self {
            tmp,
            store,
            models,
            engine,
            ownership,
            processing,
            analysis,
            bundle,
        }
    }
    fn input(&self) -> AnalyzeMeetingInput {
        AnalyzeMeetingInput {
            recording_id: "r".into(),
            matching_policy: None,
            new_run: false,
        }
    }
    fn meeting(&self) -> MeetingService {
        let capture = Arc::new(CaptureService::new(
            Arc::new(Capture {
                bundle: self.bundle.clone(),
            }),
            self.store.clone(),
        ));
        MeetingService::new(
            capture,
            self.store.clone(),
            self.store.clone(),
            self.models.clone(),
            self.processing.clone(),
            self.ownership.clone(),
        )
        .with_analysis(self.analysis.clone())
    }
}
#[tokio::test]
async fn anonymous_analysis_preserves_evidence_and_is_idempotent() {
    let f = Fixture::new(true, true).await;
    let done = f.analysis.analyze(f.input()).await.unwrap();
    assert_eq!(done.status, "completed");
    assert_eq!(done.candidates.len(), 2);
    assert!(done.suggestions.is_empty());
    assert_eq!(f.engine.embeddings.load(Ordering::SeqCst), 0);
    assert_eq!(f.engine.calls.lock().unwrap().len(), 4);
    let d = f.store.detail("r").await.unwrap().unwrap();
    assert_eq!(d.recording.title(), "My title");
    assert_eq!(d.recording.metadata["original"], true);
    assert_eq!(d.segments.len(), 2);
    assert_eq!(d.segments[1].start_ms, Some(1750));
    assert_eq!(f.analysis.analyze(f.input()).await.unwrap().id, done.id);
    assert_eq!(f.engine.calls.lock().unwrap().len(), 4);
    assert!(f.store.examples(None).await.unwrap().is_empty());
    // Completed receipts are verified rather than trusted JSON.
    let mut forged = done.clone();
    forged.sources[0].diarization = Some("absent".into());
    assert!(
        f.store
            .replace_analysis(Some(&done), &forged)
            .await
            .unwrap()
    );
    assert!(matches!(
        f.analysis.analyze(f.input()).await,
        Err(AnalyzeMeetingError::Conflict(_))
    ));
}
#[tokio::test]
async fn partial_source_resume_keeps_coherent_publication_and_no_duplicate_completed_work() {
    let f = Fixture::new(true, true).await;
    *f.engine.fail.lock().unwrap() = Some("system:asr".into());
    let Err(AnalyzeMeetingError::Incomplete(failed)) = f.analysis.analyze(f.input()).await else {
        panic!("expected retained failure")
    };
    assert_eq!(failed.status, "failed");
    assert!(failed.sources[0].transcription.is_some());
    assert_eq!(
        f.store.detail("r").await.unwrap().unwrap().segments[0].id,
        "imported"
    );
    *f.engine.fail.lock().unwrap() = None;
    let detail = f.store.detail("r").await.unwrap().unwrap();
    let failed_run = detail.runs.iter().find(|r| r.status == "failed").unwrap();
    let retried = f.processing.retry(&failed_run.id).await.unwrap();
    assert_eq!(retried.configuration["meeting_analysis_id"], failed.id);
    let done = f.analysis.analyze(f.input()).await.unwrap();
    assert_eq!(done.id, failed.id);
    assert_eq!(
        f.engine
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, m)| t == "mic" && m == "asr")
            .count(),
        1
    );
    assert_eq!(
        f.store.detail("r").await.unwrap().unwrap().segments.len(),
        2
    );
    // A stale writer cannot replace a newer checkpoint.
    assert!(
        !f.store
            .replace_analysis(Some(&failed), &done)
            .await
            .unwrap()
    );
}
#[tokio::test]
async fn explicit_matching_stays_unknown_and_never_enrolls_predictions() {
    let f = Fixture::new(false, true).await;
    let mut input = f.input();
    input.matching_policy = Some(MatchPolicy {
        minimum_score: 0.8,
        minimum_margin: 0.1,
        minimum_examples: 2,
    });
    let done = f.analysis.analyze(input.clone()).await.unwrap();
    assert_eq!(done.suggestions.len(), 1);
    assert_eq!(f.engine.embeddings.load(Ordering::SeqCst), 1);
    let suggestions = f.store.suggestions(Some("r")).await.unwrap();
    assert_eq!(suggestions.len(), 1);
    assert!(suggestions[0].decision.suggested_profile.is_none());
    assert!(f.store.examples(None).await.unwrap().is_empty());
    assert_eq!(f.analysis.analyze(input).await.unwrap().id, done.id);
    assert_eq!(f.engine.embeddings.load(Ordering::SeqCst), 1);
    // Simulate a crash after suggestion persistence but before the final checkpoint.
    let mut interrupted = done.clone();
    interrupted.status = "failed".into();
    interrupted.suggestions.clear();
    assert!(
        f.store
            .replace_analysis(Some(&done), &interrupted)
            .await
            .unwrap()
    );
    assert_eq!(
        f.analysis
            .analyze(f.input())
            .await
            .unwrap()
            .suggestions
            .len(),
        1
    );
    assert_eq!(f.engine.embeddings.load(Ordering::SeqCst), 1);
    let mut input = f.input();
    input.matching_policy = Some(MatchPolicy {
        minimum_score: 0.9,
        minimum_margin: 0.1,
        minimum_examples: 2,
    });
    assert!(matches!(
        f.analysis.analyze(input).await,
        Err(AnalyzeMeetingError::Conflict(_))
    ));
}
#[tokio::test]
async fn new_analysis_archives_receipt_and_keeps_corrections_and_original_runs() {
    let f = Fixture::new(false, true).await;
    let first = f.analysis.analyze(f.input()).await.unwrap();
    let d = f.store.detail("r").await.unwrap().unwrap();
    let segment = &d.segments[0];
    f.store
        .correct(
            &Correction {
                id: "correction".into(),
                segment_id: segment.id.clone(),
                profile_id: None,
                replacement_text: Some("Human words".into()),
                action: "text".into(),
                created_at: now(),
            },
            0,
            &[],
            &[],
        )
        .await
        .unwrap();
    let mut input = f.input();
    input.new_run = true;
    let next = f.analysis.analyze(input).await.unwrap();
    assert_ne!(first.id, next.id);
    assert_eq!(
        next.sources[0].transcription,
        first.sources[0].transcription
    );
    let d = f.store.detail("r").await.unwrap().unwrap();
    assert_eq!(d.runs.len(), 3);
    assert_eq!(d.corrections.len(), 1);
    assert_eq!(d.recording.metadata["analysis_history"][0]["id"], first.id);
    assert_eq!(d.recording.title(), "My title");
}
#[tokio::test]
async fn source_and_model_changes_refuse_resume() {
    let f = Fixture::new(false, true).await;
    *f.engine.fail.lock().unwrap() = Some("system:diari".into());
    assert!(f.analysis.analyze(f.input()).await.is_err());
    *f.models.revision.lock().unwrap() = "2".into();
    assert!(matches!(
        f.analysis.analyze(f.input()).await,
        Err(AnalyzeMeetingError::Conflict(_))
    ));
    *f.models.revision.lock().unwrap() = "1".into();
    std::fs::write(&f.bundle.tracks[0].path, b"replaced").unwrap();
    assert!(matches!(
        f.analysis.analyze(f.input()).await,
        Err(AnalyzeMeetingError::Conflict(_))
    ));
}
#[tokio::test]
async fn workflow_lease_survives_caller_abort_and_excludes_other_controller() {
    let f = Fixture::new(false, true).await;
    f.engine.delay.store(true, Ordering::SeqCst);
    let service = f.analysis.clone();
    let input = f.input();
    let caller = tokio::spawn(async move { service.analyze(input).await });
    while f.engine.calls.lock().unwrap().is_empty() {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(f.analysis.is_active("r").await.unwrap());
    caller.abort();
    assert!(matches!(
        f.analysis.analyze(f.input()).await,
        Err(AnalyzeMeetingError::Conflict(_))
    ));
    for _ in 0..100 {
        if f.analysis
            .show("r")
            .await
            .unwrap()
            .is_some_and(|p| p.status == "completed")
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("owned workflow did not settle");
}
struct Capture {
    bundle: ImportBundle,
}
#[async_trait]
impl AudioCapture for Capture {
    fn current_status(&self) -> serde_json::Value {
        json!({})
    }
    async fn devices(&self) -> Result<serde_json::Value, CapabilityError> {
        Ok(json!({}))
    }
    async fn permissions(&self) -> Result<serde_json::Value, CapabilityError> {
        Ok(json!({}))
    }
    async fn recoverable(&self) -> Result<serde_json::Value, CapabilityError> {
        Ok(json!([]))
    }
    async fn recover(&self, _: &Path) -> Result<ImportBundle, CapabilityError> {
        Ok(self.bundle.clone())
    }
    async fn record(
        &self,
        request: &CaptureRecordingInput,
        _: Arc<AtomicBool>,
    ) -> Result<ImportBundle, CapabilityError> {
        assert!(
            request
                .final_transcription
                .as_ref()
                .unwrap()
                .speaker_analysis
                .is_some()
        );
        Ok(self.bundle.clone())
    }
}
#[tokio::test]
async fn after_stop_intent_preflights_and_completes_shared_workflow() {
    let f = Fixture::new(false, false).await;
    let service = f.meeting();
    let report = service
        .record(
            RecordMeetingInput {
                capture: CaptureRecordingInput {
                    dictation: false,
                    microphone_uid: None,
                    system_audio: true,
                    title: "Synthetic meeting".into(),
                    maximum_seconds: 1,
                    final_transcription: None,
                },
                transcribe_after_stop: true,
                skip_unavailable_model: false,
                analyze_speakers: true,
                matching_policy: None,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.finalization.unwrap().status, "completed");
    assert_eq!(
        f.analysis.show("r").await.unwrap().unwrap().status,
        "completed"
    );
    assert!(service.pending().await.unwrap().is_empty());
    assert_eq!(f.engine.calls.lock().unwrap().len(), 2); // Existing pinned ASR reused.
    assert_eq!(
        service
            .finalize(FinalizeMeetingInput {
                id: "r".into(),
                model: None
            })
            .await
            .unwrap()
            .status,
        "completed"
    );
    assert_eq!(f.engine.calls.lock().unwrap().len(), 2);
    assert!(f.tmp.path().exists());
}
#[tokio::test]
async fn after_stop_analysis_failure_can_resume_through_finalization() {
    let f = Fixture::new(false, false).await;
    *f.engine.fail.lock().unwrap() = Some("system:diari".into());
    let service = f.meeting();
    let error = service
        .record(
            RecordMeetingInput {
                capture: CaptureRecordingInput {
                    dictation: false,
                    microphone_uid: None,
                    system_audio: true,
                    title: "Synthetic meeting".into(),
                    maximum_seconds: 1,
                    final_transcription: None,
                },
                transcribe_after_stop: true,
                skip_unavailable_model: false,
                analyze_speakers: true,
                matching_policy: None,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap_err();
    assert_eq!(error.retained_recording_id(), Some("r"));
    assert_eq!(service.pending().await.unwrap().len(), 1);
    *f.engine.fail.lock().unwrap() = None;
    service
        .finalize(FinalizeMeetingInput {
            id: "r".into(),
            model: None,
        })
        .await
        .unwrap();
    assert!(service.pending().await.unwrap().is_empty());
    assert_eq!(
        f.engine
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, m)| m == "asr")
            .count(),
        1
    );
}
#[tokio::test]
async fn explicit_matching_can_suggest_fixture_identity_without_creating_more_examples() {
    let f = Fixture::new(false, true).await;
    f.store
        .create_profile(&SpeakerProfile {
            id: "fixture-person".into(),
            name: "Synthetic identity".into(),
            created_at: now(),
        })
        .await
        .unwrap();
    for n in 0..2 {
        f.store
            .enroll(&VoiceExample {
                id: format!("reference-{n}"),
                profile_id: "fixture-person".into(),
                recording_id: "r".into(),
                track_id: "system".into(),
                start_ms: n * 4000,
                end_ms: n * 4000 + 4000,
                model: "fixture-space".into(),
                revision: "1".into(),
                capture_domain: "system".into(),
                vector: vec![1., 0.],
                verified_at: now(),
                origin: None,
            })
            .await
            .unwrap();
    }
    let mut input = f.input();
    input.matching_policy = Some(MatchPolicy {
        minimum_score: 0.8,
        minimum_margin: 0.1,
        minimum_examples: 2,
    });
    let done = f.analysis.analyze(input).await.unwrap();
    assert_eq!(done.suggestions.len(), 1);
    assert_eq!(
        f.store.suggestions(Some("r")).await.unwrap()[0]
            .decision
            .suggested_profile
            .as_deref(),
        Some("fixture-person")
    );
    assert_eq!(f.store.examples(None).await.unwrap().len(), 2);
}
#[tokio::test]
async fn wrong_model_role_refuses_capture_and_pending_analysis_protects_processing_files() {
    use parakeet_application::storage::StorageQueries;
    let f = Fixture::new(false, false).await;
    f.models.wrong.store(true, Ordering::SeqCst);
    assert!(
        f.meeting()
            .record(
                RecordMeetingInput {
                    capture: CaptureRecordingInput {
                        dictation: false,
                        microphone_uid: None,
                        system_audio: true,
                        title: "Synthetic meeting".into(),
                        maximum_seconds: 1,
                        final_transcription: None
                    },
                    transcribe_after_stop: true,
                    skip_unavailable_model: false,
                    analyze_speakers: true,
                    matching_policy: None
                },
                Arc::new(AtomicBool::new(false))
            )
            .await
            .is_err()
    );
    assert!(f.store.list(None).await.unwrap().is_empty());
    assert!(f.engine.calls.lock().unwrap().is_empty());
    f.models.wrong.store(false, Ordering::SeqCst);
    *f.engine.fail.lock().unwrap() = Some("system:diari".into());
    assert!(
        f.meeting()
            .record(
                RecordMeetingInput {
                    capture: CaptureRecordingInput {
                        dictation: false,
                        microphone_uid: None,
                        system_audio: true,
                        title: "Synthetic meeting".into(),
                        maximum_seconds: 1,
                        final_transcription: None
                    },
                    transcribe_after_stop: true,
                    skip_unavailable_model: false,
                    analyze_speakers: true,
                    matching_policy: None
                },
                Arc::new(AtomicBool::new(false))
            )
            .await
            .is_err()
    );
    assert!(
        f.store
            .storage_runs()
            .await
            .unwrap()
            .iter()
            .all(|r| r.pending_meeting)
    );
    *f.engine.fail.lock().unwrap() = None;
    f.meeting()
        .finalize(FinalizeMeetingInput {
            id: "r".into(),
            model: None,
        })
        .await
        .unwrap();
    assert!(
        f.store
            .storage_runs()
            .await
            .unwrap()
            .iter()
            .all(|r| !r.pending_meeting)
    );
}
#[tokio::test]
async fn cancellation_settles_analysis_as_interrupted_and_resume_reuses_prior_work() {
    use parakeet_application::processing_control::RunControlWrites;
    let f = Fixture::new(false, true).await;
    f.engine.delay.store(true, Ordering::SeqCst);
    let analysis = f.analysis.clone();
    let input = f.input();
    let worker = tokio::spawn(async move { analysis.analyze(input).await });
    loop {
        let d = f.store.detail("r").await.unwrap().unwrap();
        if let Some(r) = d.runs.iter().find(|r| r.status == "running") {
            f.store.request_cancellation(&r.id, &now()).await.unwrap();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let Err(AnalyzeMeetingError::Incomplete(receipt)) = worker.await.unwrap() else {
        panic!("expected cancellation")
    };
    assert_eq!(receipt.status, "interrupted");
    f.engine.delay.store(false, Ordering::SeqCst);
    assert_eq!(
        f.analysis.analyze(f.input()).await.unwrap().status,
        "completed"
    );
}
#[tokio::test]
async fn abandoned_source_requires_owned_recovery_before_analysis_resume() {
    use parakeet_application::jobs::{OWNERSHIP_PROTOCOL, ProcessingRecoveryService};
    let f = Fixture::new(false, true).await;
    *f.engine.fail.lock().unwrap() = Some("system:asr".into());
    let Err(AnalyzeMeetingError::Incomplete(plan)) = f.analysis.analyze(f.input()).await else {
        panic!("expected failure")
    };
    let detail = f.store.detail("r").await.unwrap().unwrap();
    let mut abandoned = detail.runs[0].clone();
    abandoned.id = new_id();
    abandoned.status = "running".into();
    abandoned.completed_at = None;
    abandoned.error = None;
    abandoned.configuration["ownership_protocol"] = json!(OWNERSHIP_PROTOCOL);
    f.store.start(&abandoned).await.unwrap();
    *f.engine.fail.lock().unwrap() = None;
    let calls = f.engine.calls.lock().unwrap().len();
    assert!(matches!(
        f.analysis.analyze(f.input()).await,
        Err(AnalyzeMeetingError::Incomplete(_))
    ));
    assert_eq!(f.engine.calls.lock().unwrap().len(), calls);
    let recovery =
        ProcessingRecoveryService::new(f.store.clone(), f.store.clone(), f.ownership.clone());
    // Simulate the durable state left by a terminated controller. The saved
    // running label must not imply that a worker still owns the recording.
    let saved = f.analysis.show("r").await.unwrap().unwrap();
    let mut stale = saved.clone();
    stale.status = "running".into();
    assert!(
        f.store
            .replace_analysis(Some(&saved), &stale)
            .await
            .unwrap()
    );
    assert!(!f.analysis.is_active("r").await.unwrap());
    use parakeet_application::jobs::ProcessingOwnership;
    let key = stable_id("meeting-analysis-controller", "r");
    let owner = f.ownership.try_acquire(&key).await.unwrap().unwrap();
    assert!(f.analysis.is_active("r").await.unwrap());
    drop(owner);
    assert!(!f.analysis.is_active("r").await.unwrap());
    assert!(
        recovery
            .recover_recording("other-recording")
            .await
            .unwrap()
            .interrupted
            .is_empty()
    );
    assert_eq!(
        recovery.recover_recording("r").await.unwrap().interrupted,
        vec![abandoned.id]
    );
    // Recovery changes abandoned source jobs, while the old plan remains
    // resumable even though its saved status still says running.
    assert_eq!(
        f.analysis.show("r").await.unwrap().unwrap().status,
        "running"
    );
    assert!(!f.analysis.is_active("r").await.unwrap());
    let resumed = f.analysis.analyze(f.input()).await.unwrap();
    assert_eq!(resumed.id, plan.id);
    assert_eq!(resumed.status, "completed");
    assert!(!resumed.candidates.is_empty());
}
