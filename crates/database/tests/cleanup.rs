use parakeet_application::speakers::SpeakerWrites;
use parakeet_application::{
    audio_locations::AudioLocationWrites, cleanup::*, processing::RunWrites, storage::*, *,
};
use parakeet_database::*;
use parakeet_domain::*;
use parakeet_infrastructure::{cleanup::*, models::ModelFiles};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
type Service = CleanupService<SqliteStore, SqliteStore>;
struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    store: Arc<SqliteStore>,
    service: Service,
    run: String,
    relative: String,
    models: Arc<ModelFiles>,
}
async fn fixture() -> Fixture {
    fixture_named("test.db").await
}
async fn fixture_named(database_name: &str) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let db = root.join(database_name);
    create_empty(&db).await.unwrap();
    migrate(&db).await.unwrap();
    let store = Arc::new(SqliteStore::new(open_existing(&db).await.unwrap()));
    let audio = root.join("original.wav");
    std::fs::write(&audio, b"original source remains").unwrap();
    store
        .import(&ImportBundle {
            recording: Recording {
                id: "r".into(),
                source_key: "fixture:r".into(),
                source_kind: "meeting".into(),
                source_title: "Original title".into(),
                title_override: Some("User title".into()),
                created_at: "2020-01-01".into(),
                duration_ms: None,
                metadata: json!({}),
            },
            tracks: vec![AudioTrack {
                id: "t".into(),
                recording_id: "r".into(),
                kind: TrackKind::System,
                path: audio.to_str().unwrap().into(),
                start_offset_ms: Some(1250),
                sample_rate: None,
                duration_ms: None,
                available: true,
                size_bytes: None,
                modified_unix_ms: None,
                provenance: json!({}),
            }],
            segments: vec![],
            original: json!({}),
            warnings: vec![],
        })
        .await
        .unwrap();
    let run = uuid::Uuid::new_v4().to_string();
    let mut processing = ProcessingRun {
        id: run.clone(),
        recording_id: "r".into(),
        kind: "transcription".into(),
        model: "fixture".into(),
        model_revision: "fixture-v1".into(),
        status: "running".into(),
        started_at: "2020-01-01T00:00:00Z".into(),
        completed_at: None,
        configuration: json!({"ownership_protocol":"file-lock-v1"}),
        result: json!({"original":"retained"}),
        error: None,
    };
    store.start(&processing).await.unwrap();
    processing.status = "completed".into();
    processing.completed_at = Some("2020-01-02T00:00:00Z".into());
    store
        .finish(
            &processing,
            &[TranscriptSegment {
                id: "segment".into(),
                recording_id: "r".into(),
                run_id: run.clone(),
                track_id: Some("t".into()),
                ordinal: 0,
                start_ms: Some(0),
                end_ms: Some(1000),
                text: "Raw model evidence".into(),
                speaker_claim: None,
                evidence: json!({}),
            }],
        )
        .await
        .unwrap();
    let relative = format!("models/work/{run}/request.json");
    std::fs::create_dir_all(root.join(&relative).parent().unwrap()).unwrap();
    std::fs::write(root.join(&relative), b"completed private scratch").unwrap();
    let models = Arc::new(ModelFiles::new(root.join("models")));
    let service = CleanupService::new(
        store.clone(),
        store.clone(),
        Arc::new(LocalCleanupFiles::new(root.clone(), db, models.clone()).unwrap()),
    );
    Fixture {
        _tmp: tmp,
        root,
        store,
        service,
        run,
        relative,
        models,
    }
}
fn aged() -> Option<parakeet_domain::storage::RetentionPolicy> {
    Some(parakeet_domain::storage::RetentionPolicy {
        completed_scratch_after_days: Some(30),
    })
}
fn recovery(f: &Fixture, r: &CleanupReview) -> PathBuf {
    f.root.join(".cleanup").join(&r.id).join("files/0000.bin")
}
#[tokio::test]
async fn reviewed_staging_restores_exact_bytes_without_touching_source_titles_or_results() {
    let f = fixture().await;
    let before = f.store.detail("r").await.unwrap().unwrap();
    assert!(f.service.review(None, 128).await.unwrap().is_none());
    let review = f.service.review(aged(), 128).await.unwrap().unwrap();
    assert_eq!(review.files.len(), 1);
    assert_eq!(review.state, CleanupState::Reviewed);
    assert!(matches!(
        f.service.apply(&review.id, &review.digest, false).await,
        Err(CleanupError::Confirmation(_))
    ));
    assert!(matches!(
        f.service.apply(&review.id, "wrong", true).await,
        Err(CleanupError::Conflict(_))
    ));
    assert!(f.root.join(&f.relative).exists());
    let staged = f
        .service
        .apply(&review.id, &review.digest, true)
        .await
        .unwrap();
    assert_eq!(staged.state, CleanupState::Staged);
    assert_eq!(staged.reclaimed_bytes, 0);
    assert!(!f.root.join(&f.relative).exists());
    assert_eq!(
        std::fs::read(recovery(&f, &review)).unwrap(),
        b"completed private scratch"
    );
    assert_eq!(
        f.service
            .apply(&review.id, &review.digest, true)
            .await
            .unwrap()
            .state,
        CleanupState::Staged
    );
    let inspection = parakeet_infrastructure::storage::LocalStorageInspection {
        root: f.root.clone(),
        database: f.root.join("test.db"),
    };
    let observation = inspection.observe(&[]).await.unwrap();
    assert!(
        observation
            .files
            .iter()
            .any(|o| o.relative_path.starts_with(".cleanup/") && o.area == StorageArea::Recovery)
    );
    assert_eq!(
        f.service.restore(&review.id).await.unwrap().state,
        CleanupState::Restored
    );
    assert_eq!(
        f.service.restore(&review.id).await.unwrap().state,
        CleanupState::Restored
    );
    assert_eq!(
        std::fs::read(f.root.join(&f.relative)).unwrap(),
        b"completed private scratch"
    );
    assert!(!recovery(&f, &review).exists());
    assert_eq!(
        std::fs::read(f.root.join("original.wav")).unwrap(),
        b"original source remains"
    );
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(f.store.detail("r").await.unwrap().unwrap()).unwrap()
    );
    assert_eq!(f.store.retention_policy().await.unwrap().revision, 0);
    let ro = SqliteStore::new(open_read_only(&f.root.join("test.db")).await.unwrap());
    assert_eq!(ro.cleanup_reviews().await.unwrap().len(), 1);
    assert_eq!(
        ro.cleanup_review(&review.id).await.unwrap().unwrap().state,
        CleanupState::Restored
    );
}
#[tokio::test]
async fn fresh_policy_references_and_missing_sources_block_stale_reviews() {
    for reason in [
        "policy",
        "linked",
        "missing",
        "pending",
        "correction",
        "speaker",
    ] {
        let f = fixture().await;
        let review = f.service.review(aged(), 128).await.unwrap().unwrap();
        match reason {
            "policy" => {
                f.store
                    .save_retention_policy(
                        &parakeet_domain::storage::RetentionPolicy::default(),
                        0,
                        "now",
                    )
                    .await
                    .unwrap();
            }
            "linked" | "missing" => {
                let t = f.store.detail("r").await.unwrap().unwrap().tracks.remove(0);
                let mut replacement = t.clone();
                replacement.path = if reason == "linked" {
                    f.root.join(&f.relative)
                } else {
                    f.root.join("absent-original")
                }
                .to_str()
                .unwrap()
                .into();
                f.store.replace_locator(&t, &replacement).await.unwrap();
            }
            "correction" => {
                f.store
                    .correct(
                        &Correction {
                            id: "correction".into(),
                            segment_id: "segment".into(),
                            profile_id: None,
                            replacement_text: Some("Explicit user correction".into()),
                            action: "edit".into(),
                            created_at: "now".into(),
                        },
                        0,
                    )
                    .await
                    .unwrap();
            }
            "speaker" => {
                f.store
                    .create_profile(&SpeakerProfile {
                        id: "mock-person".into(),
                        name: "Synthetic fixture".into(),
                        created_at: "now".into(),
                    })
                    .await
                    .unwrap();
                f.store
                    .enroll(&VoiceExample {
                        id: "example".into(),
                        profile_id: "mock-person".into(),
                        recording_id: "r".into(),
                        track_id: "t".into(),
                        start_ms: 0,
                        end_ms: 4000,
                        model: "fixture".into(),
                        revision: "v1".into(),
                        capture_domain: "system".into(),
                        vector: vec![1., 0.],
                        verified_at: "explicit fixture confirmation".into(),
                    })
                    .await
                    .unwrap();
            }
            _ => {
                use parakeet_application::meeting::MeetingFinalizationWrites;
                let plan = parakeet_application::meeting::MeetingFinalization {
                    version: 1,
                    id: uuid::Uuid::new_v4().to_string(),
                    recording_id: "r".into(),
                    model: "fixture".into(),
                    model_revision: "v1".into(),
                    status: "failed".into(),
                    track_ids: vec!["t".into()],
                    completed: vec![],
                    failures: vec![],
                    attempts: 1,
                    updated_at: "now".into(),
                };
                f.store
                    .replace_finalization("r", None, &plan)
                    .await
                    .unwrap();
            }
        }
        assert!(
            matches!(
                f.service.apply(&review.id, &review.digest, true).await,
                Err(CleanupError::Conflict(_))
            ),
            "{reason}"
        );
        assert!(f.root.join(&f.relative).exists());
        assert!(!recovery(&f, &review).exists());
    }
}
#[tokio::test]
async fn changed_hash_symlink_parent_hardlink_and_live_owners_never_move_reviewed_files() {
    use parakeet_application::jobs::ProcessingOwnership;
    for reason in [
        "same-size-write",
        "symlink-parent",
        "hardlink",
        "model-owner",
        "run-owner",
    ] {
        let f = fixture().await;
        let review = f.service.review(aged(), 128).await.unwrap().unwrap();
        let mut model_lease = None;
        let mut run_lease = None;
        match reason {
            "same-size-write" => {
                std::fs::write(f.root.join(&f.relative), b"changed!! private scratch").unwrap()
            }
            "hardlink" => {
                std::fs::hard_link(f.root.join(&f.relative), f.root.join("alias")).unwrap()
            }
            "symlink-parent" => {
                let work = f.root.join("models/work");
                std::fs::rename(&work, f.root.join("saved-work")).unwrap();
                std::os::unix::fs::symlink(f.root.join("saved-work"), work).unwrap();
            }
            "model-owner" => model_lease = Some(f.models.acquire().unwrap()),
            _ => {
                run_lease = parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
                    &f.root.join("test.db"),
                )
                .unwrap()
                .try_acquire(&f.run)
                .await
                .unwrap()
            }
        }
        match f.service.apply(&review.id, &review.digest, true).await {
            Err(_) => {}
            Ok(r) => {
                assert_eq!(r.state, CleanupState::NeedsAttention);
                assert!(!r.errors.is_empty());
            }
        }
        assert!(!recovery(&f, &review).exists());
        drop((model_lease, run_lease));
    }
}
#[tokio::test]
async fn restore_refuses_overwrite_and_recovers_durable_partial_intent() {
    let f = fixture().await;
    let mut review = f.service.review(aged(), 128).await.unwrap().unwrap();
    let second = f.root.join(format!("models/work/{}/audio.f32", f.run));
    std::fs::write(&second, b"second file").unwrap();
    // A later file cannot sneak into an existing review.
    f.service
        .apply(&review.id, &review.digest, true)
        .await
        .unwrap();
    assert!(second.exists());
    std::fs::write(f.root.join(&f.relative), b"occupied destination").unwrap();
    assert_eq!(
        f.service.restore(&review.id).await.unwrap().state,
        CleanupState::NeedsAttention
    );
    assert_eq!(
        std::fs::read(f.root.join(&f.relative)).unwrap(),
        b"occupied destination"
    );
    assert!(recovery(&f, &review).exists());
    std::fs::rename(f.root.join(&f.relative), f.root.join("unrelated-retained")).unwrap();
    // Simulate process death after moving payload but before terminal settlement.
    review.state = CleanupState::Applying;
    let mut unit = f.store.begin().await.unwrap();
    f.store.save(&mut unit, &review).await.unwrap();
    unit.commit().await.unwrap();
    assert_eq!(
        f.service.restore(&review.id).await.unwrap().state,
        CleanupState::Restored
    );
    assert_eq!(
        std::fs::read(f.root.join(&f.relative)).unwrap(),
        b"completed private scratch"
    );
}
#[tokio::test]
async fn gate_serializes_reference_publication_and_stale_prepared_import_fails() {
    let f = fixture().await;
    let gate = StorageGate::for_database(&f.root.join("test.db")).unwrap();
    let alias = f.root.join("alias.db");
    std::os::unix::fs::symlink(f.root.join("test.db"), &alias).unwrap();
    let held = gate.acquire(true).await.unwrap();
    assert!(
        StorageGate::for_database(&alias)
            .unwrap()
            .acquire(false)
            .await
            .is_err()
    );
    let writes = GuardedAudioWrites {
        gate,
        library: f.store.clone(),
        locations: f.store.clone(),
    };
    let t = f.store.detail("r").await.unwrap().unwrap().tracks.remove(0);
    assert!(writes.replace_locator(&t, &t).await.is_err());
    drop(held);
    let review = f.service.review(aged(), 128).await.unwrap().unwrap();
    let mut b = ImportBundle {
        recording: f.store.detail("r").await.unwrap().unwrap().recording,
        tracks: vec![t],
        segments: vec![],
        original: json!({}),
        warnings: vec![],
    };
    b.recording.id = "prepared".into();
    b.recording.source_key = "prepared".into();
    b.tracks[0].id = "prepared-track".into();
    b.tracks[0].recording_id = "prepared".into();
    b.tracks[0].path = f.root.join(&f.relative).to_str().unwrap().into();
    f.service
        .apply(&review.id, &review.digest, true)
        .await
        .unwrap();
    assert!(writes.import(&b).await.is_err());
    assert!(f.store.detail("prepared").await.unwrap().is_none());
    assert!(
        writes
            .replace_locator(
                &f.store.detail("r").await.unwrap().unwrap().tracks[0],
                &b.tracks[0]
            )
            .await
            .is_err()
    );
}
#[tokio::test]
async fn cleanup_uses_one_connection_per_unit_and_drops_uncommitted_work() {
    let f = fixture().await;
    // A one-connection pool would deadlock if a Store operation independently acquired.
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(f.root.join("test.db")))
        .await
        .unwrap();
    let store = Arc::new(SqliteStore::new(pool));
    let service = CleanupService::new(
        store.clone(),
        store.clone(),
        Arc::new(
            LocalCleanupFiles::new(f.root.clone(), f.root.join("test.db"), f.models.clone())
                .unwrap(),
        ),
    );
    let review = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        service.review(aged(), 128),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    let mut unit = store.begin().await.unwrap();
    let mut changed = review.clone();
    changed.state = CleanupState::Applying;
    store.save(&mut unit, &changed).await.unwrap();
    drop(unit);
    assert_eq!(
        store
            .cleanup_review(&review.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        CleanupState::Reviewed
    );
    let staged = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        service.apply(&review.id, &review.digest, true),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(staged.state, CleanupState::Staged);
    assert_eq!(
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            service.restore(&review.id)
        )
        .await
        .unwrap()
        .unwrap()
        .state,
        CleanupState::Restored
    );
}

#[tokio::test]
async fn caller_cancellation_retains_owned_driver_unit_and_leases_until_settlement() {
    use async_trait::async_trait;
    struct PausedFiles {
        inner: Arc<dyn CleanupFiles>,
        moved: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }
    struct PausedAccess {
        inner: Box<dyn CleanupAccess>,
        moved: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }
    #[async_trait]
    impl CleanupFiles for PausedFiles {
        async fn acquire(
            &self,
            runs: &[String],
        ) -> Result<Box<dyn CleanupAccess>, CapabilityError> {
            Ok(Box::new(PausedAccess {
                inner: self.inner.acquire(runs).await?,
                moved: self.moved.clone(),
                release: self.release.clone(),
            }))
        }
    }
    #[async_trait]
    impl CleanupAccess for PausedAccess {
        fn scope(&self) -> CleanupScope {
            self.inner.scope()
        }
        async fn observe(
            &self,
            audio: &[AudioStorageReference],
        ) -> Result<StorageObservation, CapabilityError> {
            self.inner.observe(audio).await
        }
        async fn identify(
            &self,
            candidate: &RetentionCandidate,
        ) -> Result<CleanupFile, CapabilityError> {
            self.inner.identify(candidate).await
        }
        async fn transfer(
            &self,
            review: &CleanupReview,
            restore: bool,
        ) -> Result<Vec<String>, CapabilityError> {
            let result = self.inner.transfer(review, restore).await?;
            self.moved.notify_one();
            self.release.notified().await;
            Ok(result)
        }
    }
    let f = fixture().await;
    let review = f.service.review(aged(), 128).await.unwrap().unwrap();
    let moved = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let service = CleanupService::new(
        f.store.clone(),
        f.store.clone(),
        Arc::new(PausedFiles {
            inner: Arc::new(
                LocalCleanupFiles::new(f.root.clone(), f.root.join("test.db"), f.models.clone())
                    .unwrap(),
            ),
            moved: moved.clone(),
            release: release.clone(),
        }),
    );
    let (id, digest) = (review.id.clone(), review.digest.clone());
    let task = tokio::spawn(async move { service.apply(&id, &digest, true).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), moved.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(!f.root.join(&f.relative).exists());
    assert!(recovery(&f, &review).exists());
    assert_eq!(
        f.service.show(&review.id).await.unwrap().state,
        CleanupState::Applying
    );
    assert!(
        StorageGate::for_database(&f.root.join("test.db"))
            .unwrap()
            .acquire(false)
            .await
            .is_err(),
        "caller cancellation must not release filesystem ownership"
    );
    assert!(
        f.models.acquire().is_err(),
        "model ownership outlives the caller"
    );
    let mut contested = tokio::spawn({
        let store = f.store.clone();
        async move {
            store
                .save_retention_policy(
                    &parakeet_domain::storage::RetentionPolicy::default(),
                    0,
                    "contended update",
                )
                .await
        }
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut contested)
            .await
            .is_err(),
        "SQLite unit must still exclude concurrent publication"
    );
    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if f.service.show(&review.id).await.unwrap().state == CleanupState::Staged {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    contested.await.unwrap().unwrap();
    assert_eq!(
        f.service.restore(&review.id).await.unwrap().state,
        CleanupState::Restored
    );
}

#[tokio::test]
async fn recovery_links_and_wrong_roots_are_refused_without_following_them() {
    let f = fixture().await;
    let review = f.service.review(aged(), 128).await.unwrap().unwrap();
    let other = f.root.join("other-root");
    std::fs::create_dir(&other).unwrap();
    let service = CleanupService::new(
        f.store.clone(),
        f.store.clone(),
        Arc::new(
            LocalCleanupFiles::new(
                other.clone(),
                f.root.join("test.db"),
                Arc::new(ModelFiles::new(other.join("models"))),
            )
            .unwrap(),
        ),
    );
    assert!(matches!(
        service.apply(&review.id, &review.digest, true).await,
        Err(CleanupError::Conflict(_))
    ));
    f.service
        .apply(&review.id, &review.digest, true)
        .await
        .unwrap();
    let directory = recovery(&f, &review).parent().unwrap().to_path_buf();
    std::fs::rename(&directory, f.root.join("preserved-recovery")).unwrap();
    std::os::unix::fs::symlink(f.root.join("preserved-recovery"), &directory).unwrap();
    assert_eq!(
        f.service.restore(&review.id).await.unwrap().state,
        CleanupState::NeedsAttention
    );
    assert!(!f.root.join(&f.relative).exists());
    assert_eq!(
        std::fs::read(f.root.join("preserved-recovery/0000.bin")).unwrap(),
        b"completed private scratch"
    );
}

#[tokio::test]
#[ignore = "explicit isolated native QA fixture; requires PARAKEET_CLEANUP_FIXTURE_RECEIPT"]
async fn prepare_native_cleanup_fixture() {
    let receipt = std::env::var_os("PARAKEET_CLEANUP_FIXTURE_RECEIPT")
        .expect("explicit receipt path required");
    let f = fixture_named("parakeet.db").await;
    drop(f.service);
    drop(f.store);
    let root = f._tmp.keep();
    std::fs::write(
        receipt,
        serde_json::to_vec_pretty(
            &json!({"root":root,"relative_path":f.relative,"run":f.run,"synthetic":true}),
        )
        .unwrap(),
    )
    .unwrap();
}
