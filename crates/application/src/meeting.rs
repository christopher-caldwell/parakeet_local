//! Durable post-capture work. Raw audio is imported before any inference begins.
use super::*;
use crate::{
    capture::{CaptureRecordingInput, CaptureService},
    jobs::ProcessingOwnership,
    processing::{ProcessingService, RunProcessingInput},
};
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletedMeetingTrack {
    pub track_id: String,
    pub run_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetingTrackFailure {
    pub track_id: Option<String>,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetingFinalization {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_analysis: Option<crate::analysis::AnalysisRecipe>,
    pub id: String,
    pub recording_id: String,
    pub model: String,
    pub model_revision: String,
    pub status: String,
    pub track_ids: Vec<String>,
    pub completed: Vec<CompletedMeetingTrack>,
    pub failures: Vec<MeetingTrackFailure>,
    pub attempts: u32,
    pub updated_at: String,
}
impl MeetingFinalization {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.version != 1
            || Uuid::parse_str(&self.id).is_err()
            || self.model.trim().is_empty()
            || self.model_revision.trim().is_empty()
            || !["pending", "running", "completed", "failed"].contains(&self.status.as_str())
        {
            return Err(ValidationError("invalid meeting finalization plan".into()));
        }
        let unique: std::collections::BTreeSet<_> = self.track_ids.iter().collect();
        if unique.len() != self.track_ids.len()
            || self.track_ids.iter().any(|id| id.trim().is_empty())
        {
            return Err(ValidationError(
                "invalid finalization source identities".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn attach(&self, bundle: &mut ImportBundle) -> Result<(), ValidationError> {
        self.validate()?;
        if !bundle.recording.metadata.is_object() {
            return Err(ValidationError("capture metadata must be an object".into()));
        }
        let mut plan = self.clone();
        plan.recording_id = bundle.recording.id.clone();
        plan.track_ids = bundle
            .tracks
            .iter()
            .filter(|t| t.kind != TrackKind::Playback)
            .map(|t| t.id.clone())
            .collect();
        bundle.recording.metadata["meeting_finalization"] =
            serde_json::to_value(plan).map_err(|e| ValidationError(e.to_string()))?;
        Ok(())
    }
}
#[async_trait]
pub trait MeetingFinalizationWrites: Send + Sync {
    /// Conditional replacement of only the workflow metadata. Titles, source
    /// provenance and other recording fields must not be replaced.
    async fn replace_finalization(
        &self,
        recording_id: &str,
        expected: Option<&MeetingFinalization>,
        next: &MeetingFinalization,
    ) -> Result<bool, PersistenceError>;
}
#[derive(Debug, thiserror::Error)]
pub enum FinalizeMeetingError {
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error("recording not found: {0}")]
    NotFound(String),
    #[error("meeting finalization conflict: {0}")]
    Conflict(String),
    #[error("recording {recording_id} is saved; speaker analysis needs attention")]
    Analysis {
        recording_id: String,
        #[source]
        source: Box<crate::analysis::AnalyzeMeetingError>,
    },
    #[error("recording {recording_id} is saved; final transcription needs attention")]
    Incomplete {
        recording_id: String,
        report: Box<MeetingFinalization>,
    },
}
impl FinalizeMeetingError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Incomplete { .. } | Self::Analysis { .. } => "incomplete",
            Self::Invalid(_) => "invalid_input",
            Self::Persistence(_) => "persistence",
            Self::Capability(_) => "capability",
        }
    }
    pub fn retained(&self) -> Option<Value> {
        match self {
            Self::Analysis {
                recording_id,
                source,
            } => Some(
                json!({"recording_id":recording_id,"speaker_analysis":source.retained(),"failure":source.to_string()}),
            ),
            Self::Incomplete {
                recording_id,
                report,
            } => Some(json!({"recording_id":recording_id,"finalization":report})),
            _ => None,
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum RecordMeetingError {
    #[error(transparent)]
    Capture(#[from] crate::capture::CaptureRecordingError),
    #[error(transparent)]
    Preparation(#[from] FinalizeMeetingError),
    #[error("speaker analysis preflight failed before capture: {0}")]
    AnalysisPreparation(#[source] Box<crate::analysis::AnalyzeMeetingError>),
    #[error("recording {recording_id} is saved, but meeting processing did not complete: {source}")]
    Finalization {
        recording_id: String,
        #[source]
        source: FinalizeMeetingError,
    },
}
impl RecordMeetingError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Capture(e) => e.kind(),
            Self::AnalysisPreparation(e) => e.kind(),
            Self::Preparation(e) | Self::Finalization { source: e, .. } => e.kind(),
        }
    }
    pub fn retained_recording_id(&self) -> Option<&str> {
        match self {
            Self::Finalization { recording_id, .. } => Some(recording_id),
            _ => None,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct RecordedMeeting {
    pub recording_id: String,
    pub finalization: Option<MeetingFinalization>,
    /// Why automatic transcription did not start although it was requested.
    #[serde(default)]
    pub transcription_skipped: Option<String>,
    /// Why speaker detection after Stop did not run although the preference asked for it.
    #[serde(default)]
    pub analysis_skipped: Option<String>,
}
operation_error!(ListPendingMeetingsError,sources{Persistence:PersistenceError=>"persistence",Invalid:ValidationError=>"invalid_input"},values{});
#[derive(Debug, Clone)]
pub struct RecordMeetingInput {
    pub capture: CaptureRecordingInput,
    pub transcribe_after_stop: bool,
    /// Preference-driven transcription records audio only when no verified ASR model exists;
    /// an explicit request keeps failing before capture.
    pub skip_unavailable_model: bool,
    pub analyze_speakers: bool,
    /// Saved calibrated criteria for after-Stop suggestions, when the user enabled them.
    pub matching_policy: Option<parakeet_domain::recognition::MatchPolicy>,
}
#[derive(Debug, Clone)]
pub struct FinalizeMeetingInput {
    pub id: String,
    pub model: Option<String>,
}
pub struct MeetingService {
    capture: Arc<CaptureService>,
    queries: Arc<dyn LibraryQueries>,
    writes: Arc<dyn MeetingFinalizationWrites>,
    models: Arc<dyn LocalModels>,
    processing: Arc<ProcessingService>,
    ownership: Arc<dyn ProcessingOwnership>,
    analysis: Option<Arc<crate::analysis::MeetingAnalysisService>>,
}
impl MeetingService {
    pub fn new(
        capture: Arc<CaptureService>,
        queries: Arc<dyn LibraryQueries>,
        writes: Arc<dyn MeetingFinalizationWrites>,
        models: Arc<dyn LocalModels>,
        processing: Arc<ProcessingService>,
        ownership: Arc<dyn ProcessingOwnership>,
    ) -> Self {
        Self {
            capture,
            queries,
            writes,
            models,
            processing,
            ownership,
            analysis: None,
        }
    }
    pub fn with_analysis(mut self, analysis: Arc<crate::analysis::MeetingAnalysisService>) -> Self {
        self.analysis = Some(analysis);
        self
    }

    async fn plan(&self, model: &str) -> Result<MeetingFinalization, FinalizeMeetingError> {
        let id = if model == "selected-asr" {
            self.models.selection().await?.asr
        } else {
            model.to_owned()
        };
        let model = self.models.readiness(&id).await?;
        if model.category != "asr" || !model.verified {
            return Err(ValidationError("automatic final transcription requires an installed, verified ASR model; record without it or install/verify a model explicitly".into()).into());
        }
        Ok(MeetingFinalization {
            speaker_analysis: None,
            version: 1,
            id: new_id(),
            recording_id: String::new(),
            model: model.id,
            model_revision: model.revision,
            status: "pending".into(),
            track_ids: vec![],
            completed: vec![],
            failures: vec![],
            attempts: 0,
            updated_at: now(),
        })
    }
    pub async fn record(
        &self,
        input: RecordMeetingInput,
        stop: Arc<AtomicBool>,
    ) -> Result<RecordedMeeting, RecordMeetingError> {
        let RecordMeetingInput {
            capture: mut request,
            transcribe_after_stop,
            skip_unavailable_model: input_skip,
            analyze_speakers,
            matching_policy,
        } = input;
        request
            .validate()
            .map_err(crate::capture::CaptureRecordingError::Invalid)?;
        let mut transcription_skipped = None;
        let mut analysis_skipped = None;
        let planned = if transcribe_after_stop || analyze_speakers {
            match self.plan("selected-asr").await {
                Ok(plan) => Some(plan),
                // Record-only fallback: never lose the meeting because no verified model exists.
                // Speaker detection needs the transcript, so it is skipped for the same reason.
                Err(FinalizeMeetingError::Invalid(reason)) if input_skip => {
                    transcription_skipped = Some(reason.to_string());
                    if analyze_speakers {
                        analysis_skipped = Some(reason.to_string());
                    }
                    None
                }
                Err(error) => return Err(error.into()),
            }
        } else {
            None
        };
        request.final_transcription = if let Some(mut plan) = planned {
            if analyze_speakers {
                let service = self.analysis.as_ref().ok_or_else(|| {
                    FinalizeMeetingError::Conflict(
                        "speaker analysis is not configured at this entry point".into(),
                    )
                })?;
                match service.prepare_recipe(matching_policy).await {
                    Ok(recipe) => {
                        if recipe.asr.id != plan.model || recipe.asr.revision != plan.model_revision
                        {
                            return Err(FinalizeMeetingError::Conflict(
                                "ASR selection changed during preflight; retry".into(),
                            )
                            .into());
                        }
                        plan.speaker_analysis = Some(recipe);
                    }
                    // Like transcription: a preference never blocks recording; say why it was skipped.
                    Err(error) if input_skip => {
                        analysis_skipped = Some(error.to_string());
                    }
                    Err(source) => {
                        return Err(RecordMeetingError::AnalysisPreparation(Box::new(source)));
                    }
                }
            }
            Some(plan)
        } else {
            None
        };
        let planned_transcription = request.final_transcription.is_some();
        let bundle = self.capture.record(request, stop).await?;
        let recording_id = bundle.recording.id;
        let finalization = if planned_transcription {
            Some(
                self.finalize(crate::meeting::FinalizeMeetingInput {
                    id: recording_id.to_string(),
                    model: None,
                })
                .await
                .map_err(|source| RecordMeetingError::Finalization {
                    recording_id: recording_id.clone(),
                    source,
                })?,
            )
        } else {
            None
        };
        Ok(RecordedMeeting {
            recording_id,
            finalization,
            transcription_skipped,
            analysis_skipped,
        })
    }
    pub async fn pending(&self) -> Result<Vec<MeetingFinalization>, ListPendingMeetingsError> {
        let mut result = vec![];
        for recording in self.queries.list(None).await? {
            if let Some(plan) = Self::read_plan(&recording)?
                && (plan.status != "completed"
                    || (plan.speaker_analysis.is_some()
                        && recording.metadata["meeting_analysis"]["status"] != "completed"))
            {
                result.push(plan);
            }
        }
        Ok(result)
    }
    fn read_plan(recording: &Recording) -> Result<Option<MeetingFinalization>, ValidationError> {
        let Some(value) = recording
            .metadata
            .get("meeting_finalization")
            .filter(|v| !v.is_null())
        else {
            return Ok(None);
        };
        let plan: MeetingFinalization = serde_json::from_value(value.clone())
            .map_err(|e| ValidationError(format!("invalid saved finalization: {e}")))?;
        plan.validate()?;
        if plan.recording_id != recording.id {
            return Err(ValidationError(
                "saved finalization belongs to another recording".into(),
            ));
        }
        Ok(Some(plan))
    }
    async fn save(
        &self,
        prior: &MeetingFinalization,
        next: &MeetingFinalization,
    ) -> Result<(), FinalizeMeetingError> {
        if !self
            .writes
            .replace_finalization(&next.recording_id, Some(prior), next)
            .await?
        {
            return Err(FinalizeMeetingError::Conflict(
                "saved plan changed; reload before continuing".into(),
            ));
        }
        Ok(())
    }
    /// Idempotent resume: the pinned model never follows a later selection change.
    /// Completed correlated source runs are reused after a checkpoint interruption.
    pub async fn finalize(
        &self,
        input: FinalizeMeetingInput,
    ) -> Result<MeetingFinalization, FinalizeMeetingError> {
        let id = input.id.as_str();
        let model = input.model.as_deref();
        let mut detail = self
            .queries
            .detail(id)
            .await?
            .ok_or_else(|| FinalizeMeetingError::NotFound(id.into()))?;
        if !detail.recording.metadata.is_object() {
            return Err(ValidationError("recording metadata is not an object".into()).into());
        }
        let plan = if let Some(plan) = Self::read_plan(&detail.recording)? {
            plan
        } else {
            let mut plan = self.plan(model.unwrap_or("selected-asr")).await?;
            plan.recording_id = id.into();
            plan.track_ids = detail
                .tracks
                .iter()
                .filter(|t| t.kind != TrackKind::Playback)
                .map(|t| t.id.clone())
                .collect();
            if !self.writes.replace_finalization(id, None, &plan).await? {
                return Err(FinalizeMeetingError::Conflict(
                    "another operation prepared final transcription; resume the saved plan".into(),
                ));
            }
            plan
        };
        if let Some(model) = model
            && model != "selected-asr"
            && model != plan.model
        {
            return Err(FinalizeMeetingError::Conflict("this recording already has a pinned finalization; use explicit transcribe-all for a new model".into()));
        }
        let prior_plan_id = plan.id.clone();
        let _lease = self.ownership.try_acquire(&plan.id).await?.ok_or_else(|| {
            FinalizeMeetingError::Conflict("another process is finalizing this recording".into())
        })?;
        detail = self
            .queries
            .detail(id)
            .await?
            .ok_or_else(|| FinalizeMeetingError::NotFound(id.into()))?;
        let mut plan = Self::read_plan(&detail.recording)?
            .ok_or_else(|| FinalizeMeetingError::Conflict("saved plan disappeared".into()))?;
        if plan.id != prior_plan_id {
            return Err(FinalizeMeetingError::Conflict(
                "saved plan was replaced while acquiring ownership".into(),
            ));
        }
        if plan.status == "completed" {
            if !plan.track_ids.is_empty()
                && plan.track_ids.iter().all(|t| {
                    plan.completed.iter().any(|c| {
                        &c.track_id == t
                            && detail.runs.iter().any(|r| {
                                r.id == c.run_id
                                    && r.status == "completed"
                                    && r.kind == "transcription"
                                    && r.model == plan.model
                                    && r.model_revision == plan.model_revision
                                    && r.configuration["meeting_finalization_id"] == plan.id
                                    && r.configuration["request"]["track_id"] == *t
                            })
                    })
                })
            {
                return self.finish_analysis(plan).await;
            }
            return Err(FinalizeMeetingError::Conflict(
                "completed plan has no matching durable source-run evidence".into(),
            ));
        }
        let prior = plan.clone();
        plan.status = "running".into();
        plan.attempts = plan
            .attempts
            .checked_add(1)
            .ok_or_else(|| ValidationError("finalization attempt counter overflow".into()))?;
        plan.failures.clear();
        plan.updated_at = now();
        self.save(&prior, &plan).await?;
        let ready = self.models.readiness(&plan.model).await;
        match ready {
            Ok(m) if m.verified && m.category == "asr" && m.revision == plan.model_revision => {}
            other => {
                let message = match other { Err(e)=>e.to_string(), _=>"pinned model revision/readiness changed; restore and verify it before resuming".into() };
                plan.failures.push(MeetingTrackFailure {
                    track_id: None,
                    message,
                });
            }
        }
        if plan.track_ids.is_empty() {
            plan.failures.push(MeetingTrackFailure {
                track_id: None,
                message: "no authoritative source tracks; playback is not a substitute".into(),
            });
        }
        if plan.failures.is_empty() {
            for track_id in plan.track_ids.clone() {
                let previous = plan.clone();
                let mut cancelled = false;
                plan.completed.retain(|c| c.track_id != track_id);
                if !detail
                    .tracks
                    .iter()
                    .any(|t| t.id == track_id && t.kind != TrackKind::Playback)
                {
                    plan.failures.push(MeetingTrackFailure {
                        track_id: Some(track_id.clone()),
                        message: "planned source is missing or is derived playback".into(),
                    });
                    plan.updated_at = now();
                    self.save(&previous, &plan).await?;
                    continue;
                }
                // Trust durable run evidence, not the workflow's summary alone.
                let completed = detail.runs.iter().find(|r| {
                    r.kind == "transcription"
                        && r.status == "completed"
                        && r.model == plan.model
                        && r.model_revision == plan.model_revision
                        && r.configuration["meeting_finalization_id"] == plan.id
                        && r.configuration["request"]["track_id"] == track_id
                });
                if let Some(run) = completed {
                    plan.completed.retain(|c| c.track_id != track_id);
                    plan.completed.push(CompletedMeetingTrack {
                        track_id: track_id.clone(),
                        run_id: run.id.clone(),
                    });
                } else if detail.runs.iter().any(|r| {
                    r.status == "running"
                        && r.configuration["meeting_finalization_id"] == plan.id
                        && r.configuration["request"]["track_id"] == track_id
                }) {
                    plan.failures.push(MeetingTrackFailure{track_id:Some(track_id.clone()),message:"an earlier source run is still running; use jobs recover to inspect abandoned ownership before retrying".into()});
                } else {
                    match self
                        .processing
                        .run_meeting(
                            RunProcessingInput {
                                recording_id: id.into(),
                                track_id: track_id.clone(),
                                model: plan.model.clone(),
                                start_ms: 0,
                                duration_ms: None,
                                word_timestamps: false,
                            },
                            &plan.id,
                            &plan.model_revision,
                        )
                        .await
                    {
                        Ok(run) => {
                            plan.completed.retain(|c| c.track_id != track_id);
                            plan.completed.push(CompletedMeetingTrack {
                                track_id: track_id.clone(),
                                run_id: run.id,
                            });
                        }
                        Err(e) => {
                            cancelled =
                                matches!(e, crate::processing::ProcessingError::Cancelled(_));
                            plan.failures.push(MeetingTrackFailure {
                                track_id: Some(track_id.clone()),
                                message: e.to_string(),
                            });
                        }
                    }
                }
                plan.updated_at = now();
                self.save(&previous, &plan).await?;
                if cancelled {
                    break;
                }
            }
        }
        let prior = plan.clone();
        plan.status = if plan.failures.is_empty() {
            "completed"
        } else {
            "failed"
        }
        .into();
        plan.updated_at = now();
        self.save(&prior, &plan).await?;
        if plan.status == "completed" {
            self.finish_analysis(plan).await
        } else {
            Err(FinalizeMeetingError::Incomplete {
                recording_id: id.into(),
                report: Box::new(plan),
            })
        }
    }
    async fn finish_analysis(
        &self,
        plan: MeetingFinalization,
    ) -> Result<MeetingFinalization, FinalizeMeetingError> {
        if let Some(recipe) = &plan.speaker_analysis {
            let service=self.analysis.as_ref().ok_or_else(||FinalizeMeetingError::Conflict("this saved plan requires speaker analysis; configure the analysis service before resuming".into()))?;
            service
                .analyze_pinned(
                    crate::analysis::AnalyzeMeetingInput {
                        recording_id: plan.recording_id.clone(),
                        matching_policy: None,
                        new_run: false,
                    },
                    Some(recipe.clone()),
                )
                .await
                .map_err(|source| FinalizeMeetingError::Analysis {
                    recording_id: plan.recording_id.clone(),
                    source: Box::new(source),
                })?;
        }
        Ok(plan)
    }
}
