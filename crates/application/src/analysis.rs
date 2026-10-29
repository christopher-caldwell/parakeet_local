//! Durable, source-aware meeting analysis. Analysis only predicts; it never names a person or
//! enrolls a voice. Examples come from the user's explicit identifications (`speakers`).
use super::*;
use crate::{
    audio_locations::AudioFileIdentityReader,
    jobs::ProcessingOwnership,
    processing::{ProcessingError, ProcessingService, PublicationGroup, RunProcessingInput},
    speakers::{EnrollmentCandidate, SpeakerService, SuggestSpeakerInput},
};
use parakeet_domain::recognition::MatchPolicy;
use std::collections::BTreeMap;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisModel {
    pub id: String,
    pub revision: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisRecipe {
    pub asr: AnalysisModel,
    pub diarization: AnalysisModel,
    pub embedding: Option<AnalysisModel>,
    /// Calibrated criteria for after-Stop name suggestions; suggestions are never confirmations.
    #[serde(default)]
    pub matching_policy: Option<MatchPolicy>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisSource {
    pub track: AudioTrack,
    pub sha256: String,
    pub transcription: Option<String>,
    pub diarization: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingAnalysis {
    pub version: u32,
    pub id: String,
    pub recording_id: String,
    pub status: String,
    pub stage: String,
    pub asr: AnalysisModel,
    pub diarization: AnalysisModel,
    pub embedding: Option<AnalysisModel>,
    pub matching_policy: Option<MatchPolicy>,
    pub sources: Vec<AnalysisSource>,
    pub publication_group: PublicationGroup,
    pub candidates: Vec<EnrollmentCandidate>,
    pub suggestions: BTreeMap<String, String>,
    pub failures: Vec<String>,
    /// Where the microphone only carried far-end speech; used to withhold "Me".
    #[serde(default)]
    pub bleed: Option<BleedSummary>,
    pub updated_at: String,
}
impl MeetingAnalysis {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.version != 1
            || Uuid::parse_str(&self.id).is_err()
            || self.recording_id.is_empty()
            || self.sources.is_empty()
            || self
                .sources
                .iter()
                .any(|s| s.track.recording_id != self.recording_id || s.sha256.len() != 64)
            || !["running", "completed", "failed", "interrupted"].contains(&self.status.as_str())
        {
            return Err(ValidationError("invalid meeting analysis plan".into()));
        }
        if let Some(p) = &self.matching_policy {
            p.validate()?;
            if self.embedding.is_none() {
                return Err(ValidationError(
                    "matching requires a pinned embedding model".into(),
                ));
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct AnalyzeMeetingInput {
    pub recording_id: String,
    pub matching_policy: Option<MatchPolicy>,
    pub new_run: bool,
}
#[async_trait]
pub trait MeetingAnalysisWrites: Send + Sync {
    async fn replace_analysis(
        &self,
        expected: Option<&MeetingAnalysis>,
        next: &MeetingAnalysis,
    ) -> Result<bool, PersistenceError>;
}
operation_error!(ShowMeetingAnalysisError,sources{Persistence:PersistenceError=>"persistence",Invalid:ValidationError=>"invalid_input"},values{NotFound=>"not_found"});
#[derive(Debug, thiserror::Error)]
pub enum AnalyzeMeetingError {
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error(transparent)]
    Processing(#[from] ProcessingError),
    #[error("recording not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("recording {} analysis needs attention",.0.recording_id)]
    Incomplete(Box<MeetingAnalysis>),
}
impl AnalyzeMeetingError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Persistence(_) => "persistence",
            Self::Capability(_) => "model",
            Self::Processing(_) => "processing",
            Self::Invalid(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Incomplete(_) => "incomplete",
        }
    }
    pub fn retained(&self) -> Option<Value> {
        if let Self::Incomplete(plan) = self {
            Some(json!({"analysis":plan}))
        } else {
            None
        }
    }
}
/// Stretches of the microphone mostly explained by the system source (far-end speech heard
/// through speakers). Recorded as evidence; the audio and transcripts are never altered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BleedSummary {
    pub method: String,
    pub delay_ms: i64,
    pub analyzed_seconds: f64,
    pub flagged_seconds: f64,
    pub spans: Vec<parakeet_domain::reconciliation::BleedSpan>,
}
#[async_trait]
pub trait BleedDetection: Send + Sync {
    async fn detect(
        &self,
        microphone: &AudioTrack,
        system: &AudioTrack,
    ) -> Result<BleedSummary, CapabilityError>;
}
pub struct MeetingAnalysisService {
    bleed: Option<Arc<dyn BleedDetection>>,
    queries: Arc<dyn LibraryQueries>,
    writes: Arc<dyn MeetingAnalysisWrites>,
    processing: Arc<ProcessingService>,
    speakers: Arc<SpeakerService>,
    models: Arc<dyn LocalModels>,
    identity: Arc<dyn AudioFileIdentityReader>,
    ownership: Arc<dyn ProcessingOwnership>,
}
impl MeetingAnalysisService {
    pub fn new(
        queries: Arc<dyn LibraryQueries>,
        writes: Arc<dyn MeetingAnalysisWrites>,
        processing: Arc<ProcessingService>,
        speakers: Arc<SpeakerService>,
        models: Arc<dyn LocalModels>,
        identity: Arc<dyn AudioFileIdentityReader>,
        ownership: Arc<dyn ProcessingOwnership>,
    ) -> Self {
        Self {
            bleed: None,
            queries,
            writes,
            processing,
            speakers,
            models,
            identity,
            ownership,
        }
    }
    /// A saved running plan can outlive its process. Probe the controller lease,
    /// including between source jobs, without changing the plan or its results.
    pub async fn is_active(&self, recording_id: &str) -> Result<bool, CapabilityError> {
        let key = stable_id("meeting-analysis-controller", recording_id);
        Ok(self.ownership.try_acquire(&key).await?.is_none())
    }
    /// Optional: lets analysis record where the microphone only carries the system source.
    pub fn with_bleed_detection(mut self, detector: Arc<dyn BleedDetection>) -> Self {
        self.bleed = Some(detector);
        self
    }
    pub async fn show(
        &self,
        id: &str,
    ) -> Result<Option<MeetingAnalysis>, ShowMeetingAnalysisError> {
        let d = self
            .queries
            .detail(id)
            .await?
            .ok_or_else(|| ShowMeetingAnalysisError::NotFound(id.into()))?;
        Ok(read_plan(&d.recording)?)
    }
    pub async fn analyze(
        &self,
        input: AnalyzeMeetingInput,
    ) -> Result<MeetingAnalysis, AnalyzeMeetingError> {
        self.analyze_pinned(input, None).await
    }
    pub(crate) async fn prepare_recipe(
        &self,
        matching_policy: Option<MatchPolicy>,
    ) -> Result<AnalysisRecipe, AnalyzeMeetingError> {
        if let Some(policy) = &matching_policy {
            policy.validate()?;
        }
        let selected = self.models.selection().await?;
        Ok(AnalysisRecipe {
            asr: self.pin(&selected.asr, "asr").await?,
            diarization: self.pin(&selected.diarization, "diarization").await?,
            embedding: if matching_policy.is_some() {
                Some(self.pin(&selected.embedding, "embedding").await?)
            } else {
                None
            },
            matching_policy,
        })
    }
    pub(crate) async fn analyze_pinned(
        &self,
        input: AnalyzeMeetingInput,
        recipe: Option<AnalysisRecipe>,
    ) -> Result<MeetingAnalysis, AnalyzeMeetingError> {
        let mut driver = Self::new(
            self.queries.clone(),
            self.writes.clone(),
            self.processing.clone(),
            self.speakers.clone(),
            self.models.clone(),
            self.identity.clone(),
            self.ownership.clone(),
        );
        driver.bleed = self.bleed.clone();
        tokio::spawn(async move { driver.drive(input, recipe).await })
            .await
            .map_err(CapabilityError::new)?
    }
    async fn save(
        &self,
        prior: Option<&MeetingAnalysis>,
        next: &mut MeetingAnalysis,
    ) -> Result<(), AnalyzeMeetingError> {
        next.updated_at = now();
        next.validate()?;
        if !self.writes.replace_analysis(prior, next).await? {
            return Err(AnalyzeMeetingError::Conflict(
                "analysis changed; reload before continuing".into(),
            ));
        }
        Ok(())
    }
    async fn pin(&self, id: &str, category: &str) -> Result<AnalysisModel, AnalyzeMeetingError> {
        let info = self.models.readiness(id).await?;
        if (category == "embedding" && info.detail["provides_voice_embeddings"] != true)
            || (category != "embedding" && info.category != category)
        {
            return Err(ValidationError(format!("model {id} is not a {category} model")).into());
        }
        if !info.installed || !info.verified {
            return Err(CapabilityError::message(format!(
                "model {id} must be installed and verified explicitly"
            ))
            .into());
        }
        Ok(AnalysisModel {
            id: info.id,
            revision: info.revision,
        })
    }
    async fn drive(
        &self,
        input: AnalyzeMeetingInput,
        recipe: Option<AnalysisRecipe>,
    ) -> Result<MeetingAnalysis, AnalyzeMeetingError> {
        if let Some(p) = &input.matching_policy {
            p.validate()?;
        }
        let key = stable_id("meeting-analysis-controller", &input.recording_id);
        // `is_active` probes by briefly taking this lease, so retry a few times before
        // concluding that another controller owns the recording.
        let mut owner = None;
        for attempt in 0..5 {
            owner = self.ownership.try_acquire(&key).await?;
            if owner.is_some() {
                break;
            }
            if attempt < 4 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
        let _owner = owner.ok_or_else(|| {
            AnalyzeMeetingError::Conflict("another process is analyzing this recording".into())
        })?;
        let d = self
            .queries
            .detail(&input.recording_id)
            .await?
            .ok_or_else(|| AnalyzeMeetingError::NotFound(input.recording_id.clone()))?;
        let prior = read_plan(&d.recording)?;
        let mut plan = if let Some(plan) = prior.as_ref().filter(|_| !input.new_run) {
            if input
                .matching_policy
                .as_ref()
                .is_some_and(|p| json!(p) != json!(plan.matching_policy))
            {
                return Err(AnalyzeMeetingError::Conflict(
                    "the saved analysis has a pinned policy; use --new-run for a different policy"
                        .into(),
                ));
            }
            if recipe.as_ref().is_some_and(|r| {
                r.asr != plan.asr
                    || r.diarization != plan.diarization
                    || r.embedding != plan.embedding
            }) {
                return Err(AnalyzeMeetingError::Conflict("saved analysis differs from the after-stop recipe; inspect it before continuing".into()));
            }
            plan.clone()
        } else {
            let recipe = if let Some(recipe) = recipe {
                recipe
            } else {
                let selected = self.models.selection().await?;
                AnalysisRecipe {
                    asr: self.pin(&selected.asr, "asr").await?,
                    diarization: self.pin(&selected.diarization, "diarization").await?,
                    embedding: if input.matching_policy.is_some() {
                        Some(self.pin(&selected.embedding, "embedding").await?)
                    } else {
                        None
                    },
                    matching_policy: input.matching_policy.clone(),
                }
            };
            let AnalysisRecipe {
                asr,
                diarization,
                embedding,
                matching_policy,
            } = recipe;
            let mut sources = vec![];
            for track in d.tracks.iter().filter(|t| t.kind != TrackKind::Playback) {
                if !track.available {
                    return Err(AnalyzeMeetingError::Conflict(format!(
                        "authoritative source {} is unavailable; retained results remain unchanged",
                        track.id
                    )));
                }
                let hash = self.check_source(track).await?;
                // Only reuse a currently published full ASR result with verified source content.
                let transcription = d
                    .runs
                    .iter()
                    .rev()
                    .find(|r| {
                        r.status == "completed"
                            && r.kind == "transcription"
                            && r.model == asr.id
                            && r.model_revision == asr.revision
                            && r.configuration["track"] == json!(track)
                            && r.result["metrics"]["source_sha256"] == hash
                            && d.segments.iter().any(|s| s.run_id == r.id)
                    })
                    .map(|r| r.id.clone());
                sources.push(AnalysisSource {
                    track: track.clone(),
                    sha256: hash,
                    transcription,
                    diarization: None,
                });
            }
            if sources.is_empty() {
                return Err(
                    ValidationError("no authoritative audio sources to analyze".into()).into(),
                );
            }
            let id = new_id();
            let publication_group = PublicationGroup {
                id: id.clone(),
                track_ids: sources
                    .iter()
                    .filter(|s| s.transcription.is_none())
                    .map(|s| s.track.id.clone())
                    .collect(),
            };
            MeetingAnalysis {
                version: 1,
                id,
                recording_id: input.recording_id,
                status: "running".into(),
                stage: "transcribing".into(),
                asr,
                diarization,
                embedding,
                matching_policy,
                sources,
                publication_group,
                candidates: vec![],
                suggestions: BTreeMap::new(),
                failures: vec![],
                bleed: None,
                updated_at: now(),
            }
        };
        // Recheck the exact sources even for a completed/idempotent request.
        for source in &plan.sources {
            let current = d
                .tracks
                .iter()
                .find(|t| t.id == source.track.id)
                .ok_or_else(|| AnalyzeMeetingError::Conflict("saved source was removed".into()))?;
            if json!(current) != json!(source.track)
                || self.check_source(current).await? != source.sha256
            {
                return Err(AnalyzeMeetingError::Conflict(
                    "source identity changed; start a new analysis after inspection".into(),
                ));
            }
        }
        // Validate historical receipts even when no new inference is required.
        for source in &plan.sources {
            for (id, model, kind) in [
                (&source.transcription, &plan.asr, "transcription"),
                (&source.diarization, &plan.diarization, "diarization"),
            ] {
                if let Some(id) = id {
                    if !d.runs.iter().any(|r| {
                        r.id == *id
                            && r.status == "completed"
                            && r.kind == kind
                            && r.model == model.id
                            && r.model_revision == model.revision
                            && r.configuration["track"] == json!(source.track)
                    }) {
                        return Err(AnalyzeMeetingError::Conflict(
                            "saved analysis result no longer matches its evidence".into(),
                        ));
                    }
                } else if plan.status == "completed" {
                    return Err(AnalyzeMeetingError::Conflict(
                        "completed analysis lacks a source receipt".into(),
                    ));
                }
            }
        }
        if plan.status == "completed" && !input.new_run {
            return Ok(plan);
        }
        for (model, category) in [(&plan.asr, "asr"), (&plan.diarization, "diarization")]
            .into_iter()
            .chain(plan.embedding.iter().map(|m| (m, "embedding")))
        {
            if self.pin(&model.id, category).await?.revision != model.revision {
                return Err(AnalyzeMeetingError::Conflict(
                    "pinned model revision changed; start a new analysis".into(),
                ));
            }
        }
        let before = prior.clone();
        plan.status = "running".into();
        plan.failures.clear();
        self.save(before.as_ref(), &mut plan).await?;
        let work = self.steps(&mut plan).await;
        if let Err(error) = work {
            let prior = plan.clone();
            plan.status = if matches!(
                error,
                AnalyzeMeetingError::Processing(ProcessingError::Cancelled(_))
            ) {
                "interrupted"
            } else {
                "failed"
            }
            .into();
            plan.failures.push(error.to_string());
            self.save(Some(&prior), &mut plan).await?;
            return Err(AnalyzeMeetingError::Incomplete(Box::new(plan)));
        }
        let prior = plan.clone();
        plan.status = "completed".into();
        plan.stage = "completed".into();
        self.save(Some(&prior), &mut plan).await?;
        Ok(plan)
    }
    async fn check_source(&self, track: &AudioTrack) -> Result<String, AnalyzeMeetingError> {
        let identity = self.identity.identify(Path::new(&track.path)).await?;
        if track.size_bytes.is_some_and(|s| s != identity.size_bytes)
            || track
                .modified_unix_ms
                .is_some_and(|m| m != identity.modified_unix_ms)
            || track.provenance["content_sha256"]
                .as_str()
                .is_some_and(|h| h != identity.sha256)
        {
            return Err(AnalyzeMeetingError::Conflict(format!(
                "source {} changed; inspect/relink before processing",
                track.id
            )));
        }
        Ok(identity.sha256)
    }
    async fn steps(&self, plan: &mut MeetingAnalysis) -> Result<(), AnalyzeMeetingError> {
        for diarize in [false, true] {
            let prior = plan.clone();
            plan.stage = if diarize { "diarizing" } else { "transcribing" }.into();
            self.save(Some(&prior), plan).await?;
            for index in 0..plan.sources.len() {
                let source = &plan.sources[index];
                let model = if diarize {
                    &plan.diarization
                } else {
                    &plan.asr
                };
                let saved = if diarize {
                    &source.diarization
                } else {
                    &source.transcription
                };
                let detail = self
                    .queries
                    .detail(&plan.recording_id)
                    .await?
                    .ok_or_else(|| AnalyzeMeetingError::NotFound(plan.recording_id.clone()))?;
                if detail.runs.iter().any(|r| {
                    r.status == "running"
                        && r.configuration["meeting_analysis_id"] == plan.id
                        && r.configuration["track"]["id"] == source.track.id
                }) {
                    return Err(AnalyzeMeetingError::Conflict("a source job still reports running; inspect Background jobs and recover abandoned owners before resuming analysis".into()));
                }
                let correlated = detail.runs.iter().rev().find(|r| {
                    r.status == "completed"
                        && r.kind
                            == if diarize {
                                "diarization"
                            } else {
                                "transcription"
                            }
                        && r.model == model.id
                        && r.model_revision == model.revision
                        && r.configuration["meeting_analysis_id"] == plan.id
                        && r.configuration["track"] == json!(source.track)
                });
                if let Some(id) = saved {
                    if !detail.runs.iter().any(|r| {
                        r.id == *id
                            && r.status == "completed"
                            && r.kind
                                == if diarize {
                                    "diarization"
                                } else {
                                    "transcription"
                                }
                            && r.model == model.id
                            && r.model_revision == model.revision
                            && r.configuration["track"] == json!(source.track)
                    }) {
                        return Err(AnalyzeMeetingError::Conflict(
                            "saved source result no longer matches its evidence".into(),
                        ));
                    }
                    continue;
                }
                if self.check_source(&source.track).await? != source.sha256 {
                    return Err(AnalyzeMeetingError::Conflict(
                        "source content changed during analysis".into(),
                    ));
                }
                let run = if let Some(run) = correlated {
                    run.clone()
                } else {
                    self.processing
                        .run_analysis(
                            RunProcessingInput {
                                recording_id: plan.recording_id.clone(),
                                track_id: source.track.id.clone(),
                                model: model.id.clone(),
                                start_ms: 0,
                                duration_ms: None,
                                word_timestamps: false,
                            },
                            &plan.id,
                            &model.revision,
                            if diarize {
                                None
                            } else {
                                Some(&plan.publication_group)
                            },
                        )
                        .await?
                };
                let prior = plan.clone();
                if diarize {
                    plan.sources[index].diarization = Some(run.id);
                } else {
                    plan.sources[index].transcription = Some(run.id);
                }
                self.save(Some(&prior), plan).await?;
            }
        }
        self.check_bleed(plan).await?;
        self.check_speaker_saturation(plan).await?;
        let prior = plan.clone();
        plan.stage = "preparing_speakers".into();
        self.save(Some(&prior), plan).await?;
        let runs: Vec<_> = plan
            .sources
            .iter()
            .filter_map(|s| s.diarization.clone())
            .collect();
        let candidates = self
            .speakers
            .candidates_for_analysis(&plan.recording_id, &runs)
            .await
            .map_err(CapabilityError::new)?;
        let prior = plan.clone();
        plan.candidates = candidates;
        plan.stage = "comparing_speakers".into();
        self.save(Some(&prior), plan).await?;
        if let (Some(policy), Some(model)) = (&plan.matching_policy, &plan.embedding) {
            let policy = policy.clone();
            let model = model.clone();
            for candidate in plan.candidates.clone() {
                let id = stable_id(
                    "analysis-suggestion",
                    &format!("{}:{}", plan.id, candidate.id),
                );
                let receipt = self
                    .speakers
                    .suggest_for_analysis(
                        SuggestSpeakerInput {
                            recording_id: plan.recording_id.clone(),
                            track_id: candidate.track_id.clone(),
                            start_ms: candidate.local_start_ms,
                            end_ms: candidate.local_end_ms,
                            more_ranges: candidate.more_ranges.clone(),
                            policy: policy.clone(),
                        },
                        &model,
                        &id,
                    )
                    .await
                    .map_err(CapabilityError::new)?;
                let prior = plan.clone();
                plan.suggestions.insert(candidate.id, receipt.id);
                self.save(Some(&prior), plan).await?;
            }
        }
        Ok(())
    }
}
impl MeetingAnalysisService {
    /// A diarizer that used every speaker slot may have merged different people on that source.
    async fn check_speaker_saturation(
        &self,
        plan: &mut MeetingAnalysis,
    ) -> Result<(), AnalyzeMeetingError> {
        let Some(detail) = self.queries.detail(&plan.recording_id).await? else {
            return Ok(());
        };
        let mut warnings = vec![];
        for source in &plan.sources {
            let run = source
                .diarization
                .as_ref()
                .and_then(|id| detail.runs.iter().find(|r| &r.id == id));
            if run.is_some_and(|r| r.result["metrics"]["speaker_slots_saturated"] == true) {
                warnings.push(format!(
                    "all speaker slots were used on source {}: different people may have been merged; verify its voices or diarize it with speakrs Community-1",
                    source.track.id
                ));
            }
        }
        if warnings.is_empty() {
            return Ok(());
        }
        let prior = plan.clone();
        for warning in warnings {
            if !plan.failures.contains(&warning) {
                plan.failures.push(warning);
            }
        }
        self.save(Some(&prior), plan).await
    }
    /// Best effort: a missing detector, a single source or a failing check never fails analysis;
    /// without the evidence, "Me" simply relies on the other safeguards.
    async fn check_bleed(&self, plan: &mut MeetingAnalysis) -> Result<(), AnalyzeMeetingError> {
        let Some(detector) = &self.bleed else {
            return Ok(());
        };
        if plan.bleed.is_some() {
            return Ok(());
        }
        let mic = plan
            .sources
            .iter()
            .map(|s| &s.track)
            .find(|t| t.kind == TrackKind::Microphone);
        let system = plan
            .sources
            .iter()
            .map(|s| &s.track)
            .find(|t| t.kind == TrackKind::System);
        let (Some(mic), Some(system)) = (mic.cloned(), system.cloned()) else {
            return Ok(());
        };
        let prior = plan.clone();
        plan.stage = "checking_bleed".into();
        self.save(Some(&prior), plan).await?;
        let prior = plan.clone();
        match detector.detect(&mic, &system).await {
            Ok(summary) => plan.bleed = Some(summary),
            Err(error) => plan
                .failures
                .push(format!("bleed check unavailable: {error}")),
        }
        self.save(Some(&prior), plan).await
    }
}
fn read_plan(recording: &Recording) -> Result<Option<MeetingAnalysis>, ValidationError> {
    let Some(value) = recording
        .metadata
        .get("meeting_analysis")
        .filter(|v| !v.is_null())
    else {
        return Ok(None);
    };
    let plan: MeetingAnalysis = serde_json::from_value(value.clone())
        .map_err(|e| ValidationError(format!("invalid saved analysis: {e}")))?;
    plan.validate()?;
    if plan.recording_id != recording.id {
        return Err(ValidationError(
            "analysis belongs to another recording".into(),
        ));
    }
    Ok(Some(plan))
}
