//! Raw capture audio is kept untouched for a week so a wrong transcript or speaker label can be
//! redone from the original. After that it is replaced by a verified lossless copy.
use super::audio_locations::{AudioFileIdentityReader, AudioLocationWrites};
use super::jobs::ProcessingQueries;
use super::storage::StorageQueries;
use super::*;
use std::collections::BTreeSet;
use std::path::PathBuf;

pub use parakeet_domain::storage::RAW_AUDIO_RETENTION_DAYS;

#[async_trait]
pub trait AudioArchiver: Send + Sync {
    /// Writes a lossless copy next to `source` and returns its path. The copy must already be
    /// decoded and compared against the source; an unverified copy is an error and is removed.
    async fn encode_verified(&self, source: &Path) -> Result<PathBuf, CapabilityError>;
    /// Removes a file this service made or replaced.
    async fn remove(&self, path: &Path) -> Result<(), CapabilityError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedTrack {
    pub recording_id: String,
    pub track_id: String,
    pub bytes_before: u64,
    pub bytes_after: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchiveAudioReport {
    pub retention_days: u64,
    pub archived: Vec<ArchivedTrack>,
    pub not_yet_due: usize,
    pub skipped_busy: usize,
    /// Tracks left exactly as they were, with the reason. Nothing is deleted on failure.
    pub failed: Vec<(String, String)>,
    pub dry_run: bool,
}
operation_error!(ArchiveAudioError,sources{Persistence:PersistenceError=>"persistence",Source:CapabilityError=>"source"},values{});

pub struct AudioArchiveService {
    storage: Arc<dyn StorageQueries>,
    library: Arc<dyn LibraryQueries>,
    processing: Arc<dyn ProcessingQueries>,
    writes: Arc<dyn AudioLocationWrites>,
    reader: Arc<dyn AudioFileIdentityReader>,
    archiver: Arc<dyn AudioArchiver>,
}
impl AudioArchiveService {
    pub fn new(
        storage: Arc<dyn StorageQueries>,
        library: Arc<dyn LibraryQueries>,
        processing: Arc<dyn ProcessingQueries>,
        writes: Arc<dyn AudioLocationWrites>,
        reader: Arc<dyn AudioFileIdentityReader>,
        archiver: Arc<dyn AudioArchiver>,
    ) -> Self {
        Self {
            storage,
            library,
            processing,
            writes,
            reader,
            archiver,
        }
    }
    /// Converts every application-owned raw capture older than the retention period.
    pub async fn archive_due(
        &self,
        now_unix_ms: u64,
        dry_run: bool,
    ) -> Result<ArchiveAudioReport, ArchiveAudioError> {
        let mut report = ArchiveAudioReport {
            retention_days: RAW_AUDIO_RETENTION_DAYS,
            dry_run,
            ..Default::default()
        };
        let busy: BTreeSet<String> = self
            .processing
            .pending_runs()
            .await?
            .into_iter()
            .map(|r| r.recording_id)
            .collect();
        let raw: Vec<_> = self
            .storage
            .storage_audio()
            .await?
            .into_iter()
            .filter(|a| a.path.ends_with(".caf"))
            .collect();
        let recordings: BTreeSet<_> = raw.iter().map(|a| a.recording_id.clone()).collect();
        let limit = RAW_AUDIO_RETENTION_DAYS * 86_400_000;
        for recording in recordings {
            let Some(detail) = self.library.detail(&recording).await? else {
                continue;
            };
            for track in detail
                .tracks
                .into_iter()
                .filter(|t| raw.iter().any(|a| a.track_id == t.id))
            {
                if !track.available
                    || track.provenance["ownership"] != "application_owned"
                    || track.provenance["archived"].is_object()
                {
                    continue;
                }
                let due = track
                    .modified_unix_ms
                    .is_some_and(|m| now_unix_ms.saturating_sub(m) >= limit);
                if !due {
                    report.not_yet_due += 1;
                } else if busy.contains(&track.recording_id) {
                    report.skipped_busy += 1;
                } else if dry_run {
                    report.archived.push(ArchivedTrack {
                        recording_id: track.recording_id.clone(),
                        track_id: track.id.clone(),
                        bytes_before: track.size_bytes.unwrap_or(0),
                        bytes_after: 0,
                    });
                } else {
                    match self.archive_track(track.clone()).await {
                        Ok(done) => report.archived.push(done),
                        Err(reason) => report.failed.push((track.id, reason)),
                    }
                }
            }
        }
        Ok(report)
    }
    async fn archive_track(&self, old: AudioTrack) -> Result<ArchivedTrack, String> {
        let source = PathBuf::from(&old.path);
        let copy = self
            .archiver
            .encode_verified(&source)
            .await
            .map_err(|e| e.to_string())?;
        let identity = match self.reader.identify(&copy).await {
            Ok(identity) => identity,
            Err(e) => {
                let _ = self.archiver.remove(&copy).await;
                return Err(e.to_string());
            }
        };
        let mut next = old.clone();
        let mut history = next.provenance["locator_history"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        history.push(json!({"path":old.path,"size_bytes":old.size_bytes,"modified_unix_ms":old.modified_unix_ms,"relinked_at":now(),"verification":"lossless_archive"}));
        next.provenance["locator_history"] = json!(history);
        next.provenance["archived"] = json!({"codec":"wavpack","archived_at":now(),"original_content_sha256":old.provenance["content_sha256"],"original_path":old.path,"verification":"decoded_float_samples_exact"});
        next.provenance["content_sha256"] = json!(identity.sha256);
        next.path = identity.path;
        next.size_bytes = Some(identity.size_bytes);
        next.modified_unix_ms = Some(identity.modified_unix_ms);
        match self.writes.replace_locator(&old, &next).await {
            Ok(true) => {}
            Ok(false) => {
                let _ = self.archiver.remove(&copy).await;
                return Err("track changed while archiving; left as it was".into());
            }
            Err(e) => {
                let _ = self.archiver.remove(&copy).await;
                return Err(e.to_string());
            }
        }
        // The library now points at the verified copy, so the raw original is no longer needed.
        self.archiver
            .remove(&source)
            .await
            .map_err(|e| format!("archived, but the raw file could not be removed: {e}"))?;
        Ok(ArchivedTrack {
            recording_id: old.recording_id,
            track_id: old.id,
            bytes_before: old.size_bytes.unwrap_or(0),
            bytes_after: identity.size_bytes,
        })
    }
}
