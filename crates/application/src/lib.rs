use async_trait::async_trait;
use chrono::Utc;
use parakeet_domain::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};
use uuid::Uuid;

macro_rules! operation_error {
    ($name:ident, sources {$($source_variant:ident:$source_type:ty => $source_kind:literal),* $(,)?}, values {$($value_variant:ident => $value_kind:literal),* $(,)?}) => {
        #[derive(Debug,thiserror::Error)]pub enum $name {
            $(#[error(transparent)] $source_variant(#[from] $source_type),)*
            $(#[error("{0}")] $value_variant(String),)*
        }
        impl $name{pub fn kind(&self)->&'static str{match self{$(Self::$source_variant(_)=>$source_kind,)*$(Self::$value_variant(_)=>$value_kind,)*}}}
    };
}
pub mod audio_locations;
pub mod jobs;
pub mod meeting;
pub mod processing;
pub mod processing_control;
pub mod settings;
pub mod speakers;

#[derive(Debug, thiserror::Error)]
#[error("persistence operation failed: {source}")]
pub struct PersistenceError {
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}
impl PersistenceError {
    pub fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(error),
        }
    }
}
#[derive(Debug, thiserror::Error)]
#[error("capability operation failed: {source}")]
pub struct CapabilityError {
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}
impl CapabilityError {
    pub fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(error),
        }
    }
    pub fn message(message: impl Into<String>) -> Self {
        Self::new(std::io::Error::other(message.into()))
    }
}

operation_error!(ImportRecordingsError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence",Source:CapabilityError=>"source"},values{});
operation_error!(ListRecordingsError,sources{Persistence:PersistenceError=>"persistence"},values{});
operation_error!(ShowRecordingError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});
operation_error!(RenameRecordingError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});
operation_error!(InspectTracksError,sources{Read:ShowRecordingError=>"nested",Source:CapabilityError=>"source"},values{});
operation_error!(ExportTextError,sources{Read:ShowRecordingError=>"nested"},values{});
operation_error!(ExportMarkdownError,sources{Read:ShowRecordingError=>"nested"},values{});
operation_error!(PlanImportError,sources{Invalid:ValidationError=>"invalid_input",Source:CapabilityError=>"source"},values{});
operation_error!(DiscoverRecordingsError,sources{Source:CapabilityError=>"source"},values{});
operation_error!(InspectMediaError,sources{Source:CapabilityError=>"source"},values{});
operation_error!(MediaImportError,sources{Source:CapabilityError=>"source",Persistence:PersistenceError=>"persistence"},values{});
operation_error!(ImportLegacyError,sources{Source:CapabilityError=>"source",Persistence:PersistenceError=>"persistence"},values{});
#[derive(Debug, Clone)]
pub struct ImportRecordingsInput {
    pub path: std::path::PathBuf,
    pub dry_run: bool,
}
#[derive(Debug, Clone)]
pub struct RenameRecordingInput {
    pub id: String,
    pub title: String,
}
#[derive(Debug, Clone)]
pub struct ImportLegacyInput {
    pub path: std::path::PathBuf,
    pub dry_run: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingDetail {
    pub recording: Recording,
    pub tracks: Vec<AudioTrack>,
    pub segments: Vec<TranscriptSegment>,
    pub runs: Vec<ProcessingRun>,
    pub corrections: Vec<Correction>,
    pub attributions: Vec<parakeet_domain::reconciliation::SegmentAttribution>,
    #[serde(default)]
    pub word_attributions: Vec<parakeet_domain::word_timing::WordAttribution>,
    /// Me / Them / Unknown per segment: where the audio came from, never a confirmed identity.
    #[serde(default)]
    pub source_attributions: Vec<parakeet_domain::reconciliation::SourceAttribution>,
    pub unmapped_corrected_segments: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovery {
    pub root: String,
    pub recordings: Vec<ImportBundle>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ImportReport {
    pub imported: usize,
    pub existing: usize,
    pub recordings: Vec<String>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct RecordingAudit {
    pub meetings: usize,
    pub microphone: usize,
    pub system: usize,
    pub both: usize,
    pub usable_alignment: usize,
    pub diarized_claims: usize,
    pub missing_source_audio: usize,
    pub duration_ms_sorted: Vec<i64>,
    pub source_formats: Vec<String>,
    pub recovered_source_claims: usize,
    pub timed_segments: usize,
    pub untimed_segments: usize,
    pub warnings: Vec<String>,
}
impl Discovery {
    pub fn audit(&self) -> RecordingAudit {
        let has =
            |r: &ImportBundle, k: TrackKind| r.tracks.iter().any(|t| t.kind == k && t.available);
        let mut durations: Vec<_> = self
            .recordings
            .iter()
            .filter_map(|r| r.recording.duration_ms)
            .collect();
        durations.sort();
        let mut formats = std::collections::BTreeSet::new();
        for r in &self.recordings {
            for t in &r.tracks {
                if let Some(ext) = Path::new(&t.path).extension() {
                    formats.insert(ext.to_string_lossy().to_string());
                }
            }
        }
        RecordingAudit {
            meetings: self.recordings.len(),
            microphone: self
                .recordings
                .iter()
                .filter(|r| has(r, TrackKind::Microphone))
                .count(),
            system: self
                .recordings
                .iter()
                .filter(|r| has(r, TrackKind::System))
                .count(),
            both: self
                .recordings
                .iter()
                .filter(|r| has(r, TrackKind::Microphone) && has(r, TrackKind::System))
                .count(),
            usable_alignment: self
                .recordings
                .iter()
                .filter(|r| {
                    r.tracks
                        .iter()
                        .filter(|t| matches!(t.kind, TrackKind::Microphone | TrackKind::System))
                        .any(|t| t.available && t.meeting_offset_ms().is_some())
                })
                .count(),
            diarized_claims: self
                .recordings
                .iter()
                .filter(|r| {
                    r.original["transcript"]["diarizationSegments"]
                        .as_array()
                        .is_some_and(|s| !s.is_empty())
                })
                .count(),
            missing_source_audio: self
                .recordings
                .iter()
                .filter(|r| {
                    !has(r, TrackKind::Microphone)
                        && !has(r, TrackKind::System)
                        && !has(r, TrackKind::File)
                })
                .count(),
            duration_ms_sorted: durations,
            source_formats: formats.into_iter().collect(),
            recovered_source_claims: self
                .recordings
                .iter()
                .filter(|r| r.original["transcript"]["recoveredFromCrash"] == true)
                .count(),
            timed_segments: self
                .recordings
                .iter()
                .flat_map(|r| &r.segments)
                .filter(|s| s.start_ms.is_some())
                .count(),
            untimed_segments: self
                .recordings
                .iter()
                .flat_map(|r| &r.segments)
                .filter(|s| s.start_ms.is_none())
                .count(),
            warnings: self
                .warnings
                .iter()
                .cloned()
                .chain(self.recordings.iter().flat_map(|r| r.warnings.clone()))
                .collect(),
        }
    }
}
#[async_trait]
pub trait RecordingSource: Send + Sync {
    async fn discover(&self, root: &Path) -> Result<Discovery, CapabilityError>;
    async fn import_file(&self, path: &Path) -> Result<ImportBundle, CapabilityError>;
    async fn inspect_tracks(&self, tracks: &[AudioTrack]) -> Result<Vec<Value>, CapabilityError>;
}
#[async_trait]
pub trait LibraryQueries: Send + Sync {
    async fn list(&self, search: Option<&str>) -> Result<Vec<Recording>, PersistenceError>;
    async fn detail(&self, id: &str) -> Result<Option<RecordingDetail>, PersistenceError>;
}
#[async_trait]
pub trait LibraryWrites: Send + Sync {
    async fn import(&self, bundle: &ImportBundle) -> Result<bool, PersistenceError>;
    async fn rename(&self, id: &str, title: &str) -> Result<bool, PersistenceError>;
}
#[derive(Clone)]
pub struct LibraryService {
    queries: Arc<dyn LibraryQueries>,
    writes: Arc<dyn LibraryWrites>,
    source: Arc<dyn RecordingSource>,
}
impl LibraryService {
    pub fn new(
        queries: Arc<dyn LibraryQueries>,
        writes: Arc<dyn LibraryWrites>,
        source: Arc<dyn RecordingSource>,
    ) -> Self {
        Self {
            queries,
            writes,
            source,
        }
    }
    pub async fn import_path(
        &self,
        input: ImportRecordingsInput,
    ) -> Result<ImportReport, ImportRecordingsError> {
        let path = input.path.as_path();
        let dry_run = input.dry_run;
        let discovery = inspect_import_source(self.source.as_ref(), path).await?;
        let mut report = ImportReport {
            imported: 0,
            existing: 0,
            recordings: vec![],
            warnings: discovery.warnings,
        };
        for bundle in discovery.recordings {
            for s in &bundle.segments {
                s.validate()?;
            }
            report.warnings.extend(bundle.warnings.clone());
            report.recordings.push(bundle.recording.id.clone());
            if dry_run {
                continue;
            }
            if self.writes.import(&bundle).await? {
                report.imported += 1;
            } else {
                report.existing += 1;
            }
        }
        Ok(report)
    }
    pub async fn list(&self, search: Option<&str>) -> Result<Vec<Recording>, ListRecordingsError> {
        Ok(self.queries.list(search).await?)
    }
    pub async fn show(&self, id: &str) -> Result<RecordingDetail, ShowRecordingError> {
        let mut d = self
            .queries
            .detail(id)
            .await?
            .ok_or_else(|| ShowRecordingError::NotFound(id.into()))?;
        order_segments(&mut d.segments);
        let active = d
            .segments
            .iter()
            .map(|s| s.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        d.unmapped_corrected_segments = d
            .corrections
            .iter()
            .filter(|c| !active.contains(c.segment_id.as_str()))
            .map(|c| c.segment_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut latest = std::collections::BTreeMap::<String, Vec<SpeakerInterval>>::new();
        for (track, run) in current_diarization_runs(&d.runs) {
            if let Ok(intervals) = serde_json::from_value(run.result["intervals"].clone()) {
                latest.insert(track, intervals);
            }
        }
        let intervals = latest.into_values().flatten().collect::<Vec<_>>();
        d.attributions = parakeet_domain::reconciliation::reconcile(&d.segments, &intervals);
        let bleed: Vec<parakeet_domain::reconciliation::BleedSpan> = serde_json::from_value(
            d.recording.metadata["meeting_analysis"]["bleed"]["spans"].clone(),
        )
        .unwrap_or_default();
        d.source_attributions = parakeet_domain::reconciliation::attribute_sources(
            &d.segments,
            &d.attributions,
            &intervals,
            &d.tracks,
            &bleed,
        );
        d.word_attributions = parakeet_domain::word_timing::reconcile_words(
            &d.segments,
            &intervals,
            &d.attributions,
            &d.tracks,
        );
        Ok(d)
    }
    pub async fn tracks(&self, id: &str) -> Result<Vec<Value>, InspectTracksError> {
        let detail = self.show(id).await?;
        Ok(self.source.inspect_tracks(&detail.tracks).await?)
    }
    pub async fn rename(&self, input: RenameRecordingInput) -> Result<(), RenameRecordingError> {
        let id = &input.id;
        let title = &input.title;
        Recording::validate_title(title)?;
        if !self.writes.rename(id, title).await? {
            return Err(RenameRecordingError::NotFound(id.into()));
        }
        Ok(())
    }
    pub async fn export_markdown(&self, id: &str) -> Result<String, ExportMarkdownError> {
        let d = self.show(id).await?;
        let mut text = format!(
            "# {}\n\nRecording: {}\n\n",
            d.recording.title(),
            d.recording.id
        );
        for segment in d.segments {
            let correction = parakeet_domain::corrections::resolve(&segment.id, &d.corrections);
            let words = correction.text.as_deref().unwrap_or(&segment.text);
            let source = d
                .source_attributions
                .iter()
                .find(|a| a.segment_id == segment.id)
                .map(|a| match a.category.as_str() {
                    "me" => "Me (microphone)",
                    "them" => "Them (system audio)",
                    _ => "Unknown source",
                })
                .unwrap_or("Unknown source");
            let speaker = match correction.speaker.as_ref() {
                Some(parakeet_domain::corrections::SpeakerOverride::Profile(id)) => id.as_str(),
                _ => source,
            };
            let time = segment
                .start_ms
                .map(|x| format!("{:.3}s", x as f64 / 1000.))
                .unwrap_or_else(|| "untimed".into());
            text.push_str(&format!("- [{time}] {speaker}: {words}\n"));
        }
        Ok(text)
    }
    pub async fn export_text(&self, id: &str) -> Result<String, ExportTextError> {
        let detail = self.show(id).await?;
        Ok(detail
            .segments
            .iter()
            .map(|segment| {
                parakeet_domain::corrections::resolve(&segment.id, &detail.corrections)
                    .text
                    .unwrap_or_else(|| segment.text.clone())
            })
            .collect::<Vec<_>>()
            .join("\n\n"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    #[serde(default)]
    pub experimental_word_timestamps: bool,
    pub id: String,
    pub category: String,
    pub revision: String,
    pub installed: bool,
    pub verified: bool,
    pub bytes: Option<u64>,
    pub license: String,
    pub path: Option<String>,
    pub detail: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDownloadProgress {
    pub model: String,
    pub file: String,
    pub file_index: usize,
    pub total_files: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}
#[async_trait]
pub trait LocalModels: Send + Sync {
    fn download_progress(&self) -> Option<ModelDownloadProgress> {
        None
    }
    async fn selection(&self) -> Result<ModelSelection, CapabilityError>;
    async fn select(&self, id: &str) -> Result<(), CapabilityError>;
    async fn select_embedding(&self, id: &str) -> Result<(), CapabilityError>;
    async fn list(&self) -> Result<Vec<ModelInfo>, CapabilityError>;
    async fn install(&self, id: &str) -> Result<ModelInfo, CapabilityError>;
    async fn verify(&self, id: &str) -> Result<ModelInfo, CapabilityError>;
    async fn readiness(&self, id: &str) -> Result<ModelInfo, CapabilityError> {
        self.verify(id).await
    }
    async fn remove(&self, id: &str) -> Result<(), CapabilityError>;
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSelection {
    pub asr: String,
    pub diarization: String,
    #[serde(default = "default_embedding_choice")]
    pub embedding: String,
}
fn default_embedding_choice() -> String {
    "speakrs-community-1-coreml".into()
}
impl Default for ModelSelection {
    fn default() -> Self {
        Self {
            asr: "parakeet-tdt-0.6b-v2".into(),
            diarization: "nemotron3-fast128".into(),
            embedding: default_embedding_choice(),
        }
    }
}
operation_error!(ListModelsError,sources{Provider:CapabilityError=>"capability"},values{});
operation_error!(InstallModelError,sources{Provider:CapabilityError=>"capability"},values{});
operation_error!(VerifyModelError,sources{Provider:CapabilityError=>"capability"},values{});
operation_error!(RemoveModelError,sources{Provider:CapabilityError=>"capability"},values{});
operation_error!(SelectModelError,sources{Provider:CapabilityError=>"capability"},values{});
operation_error!(SelectedModelsError,sources{Provider:CapabilityError=>"capability"},values{});
operation_error!(SelectEmbeddingModelError,sources{Provider:CapabilityError=>"capability"},values{Unsupported=>"invalid_input"});
pub struct ModelService {
    models: Arc<dyn LocalModels>,
}
impl ModelService {
    pub fn download_progress(&self) -> Option<ModelDownloadProgress> {
        self.models.download_progress()
    }
    pub async fn select_embedding(
        &self,
        id: &str,
    ) -> Result<ModelSelection, SelectEmbeddingModelError> {
        let model = self.models.readiness(id).await?;
        if !model.verified || model.detail["provides_voice_embeddings"] != true {
            return Err(SelectEmbeddingModelError::Unsupported(
                "Choose an installed, verified bundle that supplies voice embeddings".into(),
            ));
        }
        self.models.select_embedding(id).await?;
        Ok(self.models.selection().await?)
    }

    pub async fn selection(&self) -> Result<ModelSelection, SelectedModelsError> {
        Ok(self.models.selection().await?)
    }
    pub async fn select(&self, id: &str) -> Result<ModelSelection, SelectModelError> {
        let model = self.models.readiness(id).await?;
        if !model.verified {
            return Err(CapabilityError::message(
                "install and verify the model before selecting it",
            )
            .into());
        }
        self.models.select(id).await?;
        Ok(self.models.selection().await?)
    }

    pub fn new(models: Arc<dyn LocalModels>) -> Self {
        Self { models }
    }
    pub async fn list(&self) -> Result<Vec<ModelInfo>, ListModelsError> {
        let selected = self.models.selection().await?;
        let mut list = self.models.list().await?;
        for m in &mut list {
            m.detail["selected"] = json!(selected.asr == m.id || selected.diarization == m.id);
            m.detail["selected_for_embeddings"] = json!(selected.embedding == m.id);
        }
        Ok(list)
    }
    pub async fn install(&self, id: &str) -> Result<ModelInfo, InstallModelError> {
        Ok(self.models.install(id).await?)
    }
    pub async fn verify(&self, id: &str) -> Result<ModelInfo, VerifyModelError> {
        Ok(self.models.verify(id).await?)
    }
    pub async fn remove(&self, id: &str) -> Result<(), RemoveModelError> {
        self.models.remove(id).await?;
        Ok(())
    }
}
pub fn now() -> String {
    Utc::now().to_rfc3339()
}
pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}
pub fn receipt() -> Value {
    json!({"application_version":env!("CARGO_PKG_VERSION"),"local_only":true})
}

async fn inspect_import_source(
    source: &dyn RecordingSource,
    path: &Path,
) -> Result<Discovery, CapabilityError> {
    if path.is_file() {
        Ok(Discovery {
            root: path.display().to_string(),
            recordings: vec![source.import_file(path).await?],
            warnings: vec![],
        })
    } else {
        source.discover(path).await
    }
}
pub struct DiscoveryService {
    source: Arc<dyn RecordingSource>,
}
impl DiscoveryService {
    pub fn new(source: Arc<dyn RecordingSource>) -> Self {
        Self { source }
    }
    pub async fn plan_import(&self, path: &Path) -> Result<ImportReport, PlanImportError> {
        let discovery = inspect_import_source(self.source.as_ref(), path).await?;
        let mut report = ImportReport {
            imported: 0,
            existing: 0,
            recordings: vec![],
            warnings: discovery.warnings,
        };
        for bundle in discovery.recordings {
            for segment in &bundle.segments {
                segment.validate()?;
            }
            report.recordings.push(bundle.recording.id);
            report.warnings.extend(bundle.warnings);
        }
        Ok(report)
    }
    pub async fn discover(&self, path: &Path) -> Result<Discovery, DiscoverRecordingsError> {
        Ok(self.source.discover(path).await?)
    }
}
#[async_trait]
pub trait MediaImport: Send + Sync {
    async fn inspect(&self, url: &str) -> Result<Value, CapabilityError>;
    async fn download(&self, url: &str) -> Result<ImportBundle, CapabilityError>;
}
pub struct MediaService {
    media: Arc<dyn MediaImport>,
    writes: Arc<dyn LibraryWrites>,
    queries: Arc<dyn LibraryQueries>,
}
impl MediaService {
    pub fn new(
        media: Arc<dyn MediaImport>,
        writes: Arc<dyn LibraryWrites>,
        queries: Arc<dyn LibraryQueries>,
    ) -> Self {
        Self {
            media,
            writes,
            queries,
        }
    }
    pub async fn inspect(&self, url: &str) -> Result<Value, InspectMediaError> {
        Ok(self.media.inspect(url).await?)
    }
    pub async fn import(&self, url: &str) -> Result<Recording, MediaImportError> {
        let bundle = self.media.download(url).await?;
        self.writes.import(&bundle).await?;
        Ok(self
            .queries
            .detail(&bundle.recording.id)
            .await?
            .ok_or_else(|| {
                PersistenceError::new(std::io::Error::other("imported recording is missing"))
            })?
            .recording)
    }
}

pub mod evaluation;
pub mod scoring;

#[async_trait]
pub trait AudioPlayback: Send + Sync {
    async fn play(
        &self,
        track: &AudioTrack,
        start_ms: i64,
        duration_ms: i64,
    ) -> Result<(), CapabilityError>;
}
operation_error!(PlayAudioError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence",Source:CapabilityError=>"source"},values{NotFound=>"not_found"});
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayAudioInput {
    pub recording_id: String,
    pub track_id: String,
    pub start_ms: i64,
    pub duration_ms: i64,
}
pub struct PlaybackService {
    queries: Arc<dyn LibraryQueries>,
    player: Arc<dyn AudioPlayback>,
}
impl PlaybackService {
    pub fn new(queries: Arc<dyn LibraryQueries>, player: Arc<dyn AudioPlayback>) -> Self {
        Self { queries, player }
    }
    pub async fn play(&self, input: PlayAudioInput) -> Result<(), PlayAudioError> {
        let recording_id = input.recording_id.as_str();
        let track_id = input.track_id.as_str();
        let start_ms = input.start_ms;
        let duration_ms = input.duration_ms;
        if start_ms < 0 || duration_ms <= 0 || duration_ms > 30_000 {
            return Err(
                ValidationError("play a nonnegative interval up to 30 seconds".into()).into(),
            );
        }
        let d = self
            .queries
            .detail(recording_id)
            .await?
            .ok_or_else(|| PlayAudioError::NotFound(recording_id.into()))?;
        let t = d
            .tracks
            .iter()
            .find(|t| t.id == track_id)
            .ok_or_else(|| PlayAudioError::NotFound(track_id.into()))?;
        self.player.play(t, start_ms, duration_ms).await?;
        Ok(())
    }
}

pub mod capture;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacySourceRecord {
    pub source_id: String,
    pub source_type: String,
    pub title: String,
    pub created_at: String,
    pub duration_ms: Option<i64>,
    pub file_path: Option<String>,
    pub original: Value,
}
#[async_trait]
pub trait LegacyCatalog: Send + Sync {
    async fn read(&self, path: &Path) -> Result<Vec<LegacySourceRecord>, CapabilityError>;
}
#[async_trait]
pub trait LegacyNormalization: Send + Sync {
    async fn normalize(&self, source: &LegacySourceRecord)
    -> Result<ImportBundle, CapabilityError>;
}
pub struct LegacyImportService {
    catalog: Arc<dyn LegacyCatalog>,
    normalizer: Arc<dyn LegacyNormalization>,
    writes: Arc<dyn LibraryWrites>,
}
impl LegacyImportService {
    pub fn new(
        catalog: Arc<dyn LegacyCatalog>,
        normalizer: Arc<dyn LegacyNormalization>,
        writes: Arc<dyn LibraryWrites>,
    ) -> Self {
        Self {
            catalog,
            normalizer,
            writes,
        }
    }
    pub async fn import(
        &self,
        input: ImportLegacyInput,
    ) -> Result<ImportReport, ImportLegacyError> {
        let path = input.path.as_path();
        let dry_run = input.dry_run;
        let rows = self.catalog.read(path).await?;
        let mut report = ImportReport {
            imported: 0,
            existing: 0,
            recordings: vec![],
            warnings: vec![],
        };
        for row in rows {
            let b = self.normalizer.normalize(&row).await?;
            report.recordings.push(b.recording.id.clone());
            report.warnings.extend(b.warnings.clone());
            if !dry_run {
                if self.writes.import(&b).await? {
                    report.imported += 1;
                } else {
                    report.existing += 1;
                }
            }
        }
        Ok(report)
    }
}

pub mod dictation;

#[cfg(test)]
mod consent_tests;

operation_error!(TranscribeMediaError,sources{Import:MediaImportError=>"nested"},values{});
#[derive(Debug, Clone)]
pub struct TranscribeMediaInput {
    pub url: String,
    pub model: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct MediaTranscription {
    pub recording: Recording,
    pub runs: Vec<ProcessingRun>,
    pub transcription_failure: Option<String>,
}
pub struct MediaTranscriptionService {
    media: Arc<MediaService>,
    processing: Arc<processing::ProcessingService>,
}
impl MediaTranscriptionService {
    pub fn new(media: Arc<MediaService>, processing: Arc<processing::ProcessingService>) -> Self {
        Self { media, processing }
    }
    pub async fn transcribe(
        &self,
        input: TranscribeMediaInput,
    ) -> Result<MediaTranscription, TranscribeMediaError> {
        let recording = self.media.import(&input.url).await?;
        let result = self
            .processing
            .recording(processing::TranscribeRecordingInput {
                id: recording.id.clone(),
                model: input.model,
            })
            .await;
        let (runs, transcription_failure) = match result {
            Ok(r) => (r, None),
            Err(e) => (vec![], Some(e.to_string())),
        };
        Ok(MediaTranscription {
            recording,
            runs,
            transcription_failure,
        })
    }
}

pub mod cluster_edits;

operation_error!(PlaySegmentError,sources{Persistence:PersistenceError=>"persistence",Playback:PlayAudioError=>"nested",Invalid:ValidationError=>"invalid_input"},values{NotFound=>"not_found",Unlinked=>"source"});
#[derive(Debug, Clone)]
pub struct PlaySegmentInput {
    pub recording_id: String,
    pub segment_id: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct PlayedSegment {
    pub track_id: String,
    pub local_start_ms: i64,
    pub played_duration_ms: i64,
    pub truncated_to_30_seconds: bool,
}
impl PlaybackService {
    pub async fn play_segment(
        &self,
        input: PlaySegmentInput,
    ) -> Result<PlayedSegment, PlaySegmentError> {
        let d = self
            .queries
            .detail(&input.recording_id)
            .await?
            .ok_or_else(|| PlaySegmentError::NotFound(input.recording_id.clone()))?;
        let segment = d
            .segments
            .iter()
            .find(|s| s.id == input.segment_id)
            .ok_or_else(|| PlaySegmentError::NotFound(input.segment_id))?;
        let track = d
            .tracks
            .iter()
            .find(|t| Some(&t.id) == segment.track_id.as_ref())
            .ok_or_else(|| {
                PlaySegmentError::Unlinked("segment has no authoritative audio-source link".into())
            })?;
        let start = segment.evidence["local_start_ms"].as_i64().or_else(|| {
            segment
                .start_ms
                .zip(track.meeting_offset_ms())
                .and_then(|(time, offset)| time.checked_sub(offset))
        });
        let end = segment.evidence["local_end_ms"].as_i64().or_else(|| {
            segment
                .end_ms
                .zip(track.meeting_offset_ms())
                .and_then(|(time, offset)| time.checked_sub(offset))
        });
        let (Some(start), Some(end)) = (start, end) else {
            return Err(PlaySegmentError::Unlinked(
                "segment has no reliable source-local timing".into(),
            ));
        };
        let duration = end
            .checked_sub(start)
            .filter(|duration| *duration > 0)
            .ok_or_else(|| ValidationError("segment interval is invalid".into()))?;
        self.play(PlayAudioInput {
            recording_id: input.recording_id,
            track_id: track.id.clone(),
            start_ms: start,
            duration_ms: duration.min(30000),
        })
        .await?;
        Ok(PlayedSegment {
            track_id: track.id.clone(),
            local_start_ms: start,
            played_duration_ms: duration.min(30000),
            truncated_to_30_seconds: duration > 30000,
        })
    }
}

pub mod storage;

pub mod cleanup;

pub mod analysis;
