use parakeet_application::{
    LibraryQueries, LibraryWrites, audio_archive::AudioArchiveService, processing::RunWrites,
};
use parakeet_database::*;
use parakeet_domain::*;
use parakeet_infrastructure::{archive::FfmpegArchiver, audio::LocalAudioIdentity};
use serde_json::json;
use std::{
    path::Path,
    process::Command,
    sync::Arc,
    time::{Duration, SystemTime},
};

fn write_capture(dir: &Path, name: &str, age_days: u64) -> std::path::PathBuf {
    let path = dir.join(name);
    assert!(
        Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=300:duration=3:sample_rate=48000",
                "-c:a",
                "pcm_f32le"
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let when = SystemTime::now() - Duration::from_secs(age_days * 86_400);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(when)
        .unwrap();
    path
}
async fn add(store: &SqliteStore, id: &str, path: &Path) {
    let meta = std::fs::metadata(path).unwrap();
    let bundle = ImportBundle {
        recording: Recording {
            id: id.into(),
            source_key: format!("original:{id}"),
            source_kind: "meeting".into(),
            source_title: id.into(),
            title_override: None,
            created_at: "2026-01-01".into(),
            duration_ms: Some(3_000),
            metadata: json!({}),
        },
        tracks: vec![AudioTrack {
            id: format!("{id}-track"),
            recording_id: id.into(),
            kind: TrackKind::Microphone,
            path: path.to_str().unwrap().into(),
            start_offset_ms: Some(0),
            sample_rate: Some(48000),
            duration_ms: Some(3_000),
            available: true,
            size_bytes: Some(meta.len()),
            modified_unix_ms: Some(
                meta.modified()
                    .unwrap()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            ),
            provenance: json!({"ownership": "application_owned", "content_sha256": "original-hash"}),
        }],
        segments: vec![],
        original: json!({}),
        warnings: vec![],
    };
    store.import(&bundle).await.unwrap();
}
#[tokio::test]
async fn only_old_idle_application_owned_captures_become_verified_lossless_copies() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("test.db");
    create_empty(&db).await.unwrap();
    migrate(&db).await.unwrap();
    let store = Arc::new(SqliteStore::new(open_existing(&db).await.unwrap()));
    let old = write_capture(tmp.path(), "old-raw.caf", 9);
    let recent = write_capture(tmp.path(), "recent-raw.caf", 2);
    let busy = write_capture(tmp.path(), "busy-raw.caf", 9);
    add(&store, "old", &old).await;
    add(&store, "recent", &recent).await;
    add(&store, "busy", &busy).await;
    store
        .start(&ProcessingRun {
            id: "run".into(),
            recording_id: "busy".into(),
            kind: "transcription".into(),
            model: "fixture".into(),
            model_revision: "1".into(),
            status: "running".into(),
            started_at: "now".into(),
            completed_at: None,
            configuration: json!({}),
            result: json!({}),
            error: None,
        })
        .await
        .unwrap();
    let ownership = Arc::new(
        parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(&db).unwrap(),
    );
    let service = AudioArchiveService::new(
        store.clone(),
        store.clone(),
        store.clone(),
        store.clone(),
        Arc::new(LocalAudioIdentity),
        Arc::new(FfmpegArchiver),
        ownership.clone(),
    );
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let preview = service.archive_due(now, true).await.unwrap();
    assert_eq!(preview.archived.len(), 1);
    assert!(old.exists(), "a dry run changes nothing");

    // A recording being analyzed holds its lease, and the archive leaves it alone.
    {
        use parakeet_application::jobs::ProcessingOwnership;
        let key = parakeet_domain::stable_id("meeting-analysis-controller", "old");
        let _analyzing = ownership.try_acquire(&key).await.unwrap().unwrap();
        let held = service.archive_due(now, false).await.unwrap();
        assert!(held.archived.is_empty());
        assert_eq!(held.skipped_busy, 2);
        assert!(old.exists());
    }

    let report = service.archive_due(now, false).await.unwrap();
    assert_eq!(
        (
            report.archived.len(),
            report.not_yet_due,
            report.skipped_busy
        ),
        (1, 1, 1)
    );
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(!old.exists() && recent.exists() && busy.exists());
    let track = store.detail("old").await.unwrap().unwrap().tracks.remove(0);
    assert!(track.path.ends_with(".wv"));
    assert_eq!(
        track.provenance["archived"]["original_content_sha256"],
        "original-hash"
    );
    assert_ne!(track.provenance["content_sha256"], "original-hash");
    assert_eq!(
        track.provenance["locator_history"][0]["verification"],
        "lossless_archive"
    );
    assert!(track.available);
    parakeet_infrastructure::audio::decode(&track, 0, 1_000)
        .await
        .unwrap();

    let again = service.archive_due(now, false).await.unwrap();
    assert!(
        again.archived.is_empty(),
        "already archived tracks are not touched twice"
    );
}
