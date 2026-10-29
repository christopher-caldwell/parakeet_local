use super::*;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone)]
pub struct AudioStorageReference {
    pub track_id: String,
    pub recording_id: String,
    pub path: String,
}
#[async_trait]
pub trait StorageQueries: Send + Sync {
    async fn storage_audio(&self) -> Result<Vec<AudioStorageReference>, PersistenceError>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageArea {
    Database,
    RecordedAudio,
    Models,
    Processing,
    Playback,
    Recovery,
    Evaluations,
    Other,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageFileObservation {
    pub relative_path: String,
    pub area: StorageArea,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub device: u64,
    pub inode: u64,
    pub hard_links: u64,
    pub ordinary_file: bool,
    pub linked_audio_tracks: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalAudioObservation {
    pub path: String,
    pub track_ids: Vec<String>,
    pub missing_or_unresolved: bool,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageObservation {
    pub root: String,
    pub files: Vec<StorageFileObservation>,
    pub external_audio: Vec<ExternalAudioObservation>,
    pub database_outside_root_bytes: u64,
    pub volume_available_bytes: Option<u64>,
    pub volume_total_bytes: Option<u64>,
    pub complete: bool,
    pub warnings: Vec<String>,
}
#[async_trait]
pub trait StorageInspection: Send + Sync {
    async fn observe(
        &self,
        audio: &[AudioStorageReference],
    ) -> Result<StorageObservation, CapabilityError>;
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageSize {
    pub files: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
}
impl StorageSize {
    fn add(&mut self, file: &StorageFileObservation) {
        self.files += 1;
        self.logical_bytes = self.logical_bytes.saturating_add(file.logical_bytes);
        self.allocated_bytes = self.allocated_bytes.saturating_add(file.allocated_bytes);
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageUsage {
    pub observed_at: String,
    pub root: String,
    pub owned_totals: StorageSize,
    pub areas: BTreeMap<StorageArea, StorageSize>,
    pub external_audio: Vec<ExternalAudioObservation>,
    pub referenced_external_file_bytes: u64,
    pub unresolved_audio_references: usize,
    pub database_outside_root_bytes: u64,
    pub volume_available_bytes: Option<u64>,
    pub volume_total_bytes: Option<u64>,
    pub complete: bool,
    pub warnings: Vec<String>,
    pub measurement: String,
}
operation_error!(InspectStorageError,sources{Persistence:PersistenceError=>"persistence",Inspection:CapabilityError=>"storage"},values{});
/// Reports what the library uses on disk. Nothing here deletes or moves files; raw captures
/// are compacted only by the archive (see `audio_archive`).
pub struct StorageService {
    queries: Arc<dyn StorageQueries>,
    inspection: Arc<dyn StorageInspection>,
}
impl StorageService {
    pub fn new(queries: Arc<dyn StorageQueries>, inspection: Arc<dyn StorageInspection>) -> Self {
        Self {
            queries,
            inspection,
        }
    }
    pub async fn usage(&self) -> Result<StorageUsage, InspectStorageError> {
        let audio = self.queries.storage_audio().await?;
        Ok(usage(self.inspection.observe(&audio).await?))
    }
}
fn usage(observation: StorageObservation) -> StorageUsage {
    let mut total = StorageSize::default();
    let mut areas = BTreeMap::<StorageArea, StorageSize>::new();
    let mut seen = BTreeSet::new();
    for file in &observation.files {
        if seen.insert((file.device, file.inode)) {
            total.add(file);
            areas.entry(file.area).or_default().add(file);
        }
    }
    let referenced_external_file_bytes = observation
        .external_audio
        .iter()
        .filter(|a| !a.missing_or_unresolved)
        .map(|a| a.logical_bytes)
        .fold(0u64, u64::saturating_add);
    let unresolved_audio_references = observation
        .external_audio
        .iter()
        .filter(|a| a.missing_or_unresolved)
        .map(|a| a.track_ids.len())
        .sum();
    StorageUsage {
        referenced_external_file_bytes,
        unresolved_audio_references,
        observed_at: now(),
        root: observation.root,
        owned_totals: total,
        areas,
        external_audio: observation.external_audio,
        database_outside_root_bytes: observation.database_outside_root_bytes,
        volume_available_bytes: observation.volume_available_bytes,
        volume_total_bytes: observation.volume_total_bytes,
        complete: observation.complete,
        warnings: observation.warnings,
        measurement: "File sizes and allocated blocks; hard links counted once within the root. Shared APFS extents, compression and actual freed space are not measured.".into(),
    }
}
