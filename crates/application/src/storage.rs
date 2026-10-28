use super::*;
use parakeet_domain::storage::*;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetentionPolicySnapshot {
    pub policy: RetentionPolicy,
    pub revision: u64,
    pub updated_at: Option<String>,
}
#[derive(Debug, Clone)]
pub struct StorageRunFact {
    pub id: String,
    pub recording_id: String,
    pub status: String,
    pub ownership_verified: bool,
    pub completed_at: Option<String>,
    pub pending_meeting: bool,
    pub speaker_reference: bool,
    pub manual_correction: bool,
}
#[derive(Debug, Clone)]
pub struct AudioStorageReference {
    pub track_id: String,
    pub recording_id: String,
    pub path: String,
}
#[async_trait]
pub trait StorageQueries: Send + Sync {
    async fn storage_audio(&self) -> Result<Vec<AudioStorageReference>, PersistenceError>;
    async fn storage_runs(&self) -> Result<Vec<StorageRunFact>, PersistenceError>;
    async fn retention_policy(&self) -> Result<RetentionPolicySnapshot, PersistenceError>;
}
#[async_trait]
pub trait StoragePolicyWrites: Send + Sync {
    async fn save_retention_policy(
        &self,
        policy: &RetentionPolicy,
        expected_revision: u64,
        at: &str,
    ) -> Result<Option<RetentionPolicySnapshot>, PersistenceError>;
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
    pub run_id: Option<String>,
    pub recognized_completed_scratch: bool,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionCandidate {
    pub relative_path: String,
    pub run_id: String,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPreview {
    pub preview_id: String,
    pub usage: StorageUsage,
    pub saved_policy: RetentionPolicySnapshot,
    pub effective_policy: RetentionPolicy,
    pub hypothetical: bool,
    pub candidates: Vec<RetentionCandidate>,
    pub candidate_totals: StorageSize,
    pub protected: BTreeMap<StorageProtection, StorageSize>,
    pub automatic_cleanup: bool,
    pub execution_available: bool,
}
operation_error!(InspectStorageError,sources{Persistence:PersistenceError=>"persistence",Inspection:CapabilityError=>"storage"},values{});
operation_error!(ReadRetentionPolicyError,sources{Persistence:PersistenceError=>"persistence"},values{});
operation_error!(SaveRetentionPolicyError,sources{Persistence:PersistenceError=>"persistence",Invalid:ValidationError=>"invalid_input"},values{Conflict=>"conflict"});
operation_error!(PreviewRetentionError,sources{Persistence:PersistenceError=>"persistence",Inspection:CapabilityError=>"storage",Invalid:ValidationError=>"invalid_input"},values{});
pub struct StorageService {
    queries: Arc<dyn StorageQueries>,
    writes: Arc<dyn StoragePolicyWrites>,
    inspection: Arc<dyn StorageInspection>,
}
impl StorageService {
    pub fn new(
        queries: Arc<dyn StorageQueries>,
        writes: Arc<dyn StoragePolicyWrites>,
        inspection: Arc<dyn StorageInspection>,
    ) -> Self {
        Self {
            queries,
            writes,
            inspection,
        }
    }
    pub async fn policy(&self) -> Result<RetentionPolicySnapshot, ReadRetentionPolicyError> {
        Ok(self.queries.retention_policy().await?)
    }
    pub async fn save_policy(
        &self,
        policy: RetentionPolicy,
        expected_revision: u64,
    ) -> Result<RetentionPolicySnapshot, SaveRetentionPolicyError> {
        policy.validate()?;
        if expected_revision >= i64::MAX as u64 {
            return Err(ValidationError("policy revision is out of range".into()).into());
        }
        self.writes
            .save_retention_policy(&policy, expected_revision, &now())
            .await?
            .ok_or_else(|| {
                SaveRetentionPolicyError::Conflict(
                    "retention policy changed; refresh before saving".into(),
                )
            })
    }
    pub async fn usage(&self) -> Result<StorageUsage, InspectStorageError> {
        let audio = self.queries.storage_audio().await?;
        Ok(usage(self.inspection.observe(&audio).await?))
    }
    pub async fn preview(
        &self,
        override_policy: Option<RetentionPolicy>,
    ) -> Result<RetentionPreview, PreviewRetentionError> {
        if let Some(p) = &override_policy {
            p.validate()?;
        }
        let saved = self.queries.retention_policy().await?;
        saved.policy.validate()?;
        let audio = self.queries.storage_audio().await?;
        let run_facts = self.queries.storage_runs().await?;
        let observation = self.inspection.observe(&audio).await?;
        let policy = override_policy
            .clone()
            .unwrap_or_else(|| saved.policy.clone());
        let now = Utc::now().timestamp_millis();
        let runs: BTreeMap<_, _> = run_facts.iter().map(|r| (r.id.as_str(), r)).collect();
        let mut protected = BTreeMap::<StorageProtection, StorageSize>::new();
        let mut candidates = vec![];
        let mut candidate_totals = StorageSize::default();
        let mut seen = BTreeSet::new();
        for file in &observation.files {
            // Hard-link aliases are one accounting object. Multiple links are
            // always protected, including aliases to audio outside the root.
            if !seen.insert((file.device, file.inode)) {
                continue;
            }
            let reason = protection(file, &runs, &policy, now);
            if reason == StorageProtection::EligibleCompletedScratch {
                candidate_totals.add(file);
                candidates.push(RetentionCandidate {
                    relative_path: file.relative_path.clone(),
                    run_id: file
                        .run_id
                        .clone()
                        .expect("eligible scratch has an identified run"),
                    logical_bytes: file.logical_bytes,
                    allocated_bytes: file.allocated_bytes,
                });
            } else {
                protected.entry(reason).or_default().add(file);
            }
        }
        Ok(RetentionPreview {
            preview_id: new_id(),
            usage: usage(observation),
            saved_policy: saved,
            effective_policy: policy,
            hypothetical: override_policy.is_some(),
            candidates,
            candidate_totals,
            protected,
            automatic_cleanup: false,
            execution_available: false,
        })
    }
}
fn protection(
    file: &StorageFileObservation,
    runs: &BTreeMap<&str, &StorageRunFact>,
    policy: &RetentionPolicy,
    now: i64,
) -> StorageProtection {
    use StorageProtection::*;
    if !file.linked_audio_tracks.is_empty() {
        return LinkedAudio;
    }
    if file.hard_links > 1 {
        return MultipleHardLinks;
    }
    if !file.ordinary_file {
        return UncertainArtifact;
    }
    if file.area != StorageArea::Processing {
        return match file.area {
            StorageArea::Database => Database,
            StorageArea::RecordedAudio => RecordedAudio,
            StorageArea::Models => Models,
            StorageArea::Recovery => Recovery,
            StorageArea::Playback => Playback,
            _ => OtherFiles,
        };
    }
    if !file.recognized_completed_scratch {
        return UncertainArtifact;
    }
    let Some(run) = file.run_id.as_deref().and_then(|id| runs.get(id)) else {
        return UnknownRun;
    };
    RunRetentionEvidence {
        status: run.status.clone(),
        ownership_verified: run.ownership_verified,
        completed_unix_ms: run
            .completed_at
            .as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp_millis()),
        pending_meeting: run.pending_meeting,
        speaker_reference: run.speaker_reference,
        manual_correction: run.manual_correction,
    }
    .protection(policy, now)
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
        measurement: "File sizes and allocated blocks; hard links counted once within the root. Shared APFS extents, compression and actual freed space are not measured. Preview is observational and does not authorize deletion.".into(),
    }
}
