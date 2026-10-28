use super::*;
use crate::jobs::{OWNERSHIP_PROTOCOL, ProcessingOwnership, ProcessingQueries};
use crate::processing_control::*;

#[derive(Debug, thiserror::Error)]
pub enum ProcessingError {
    #[error("processing cancelled; retained run {0}")]
    Cancelled(String),
    #[error("recording or track not found: {0}")]
    NotFound(String),
    #[error("source track unavailable or changed: {0}")]
    Unavailable(String),
    #[error("processing conflict: {0}")]
    Conflict(String),
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Engine(#[from] CapabilityError),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunProcessingInput {
    pub recording_id: String,
    pub track_id: String,
    pub model: String,
    pub start_ms: i64,
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub word_timestamps: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineOutput {
    pub segments: Vec<TranscriptSegment>,
    pub intervals: Vec<SpeakerInterval>,
    pub raw: Value,
    pub metrics: Value,
}
#[async_trait]
pub trait SpeechProcessing: Send + Sync {
    fn cancellation_mode(&self, _model: &str) -> &'static str {
        "at_engine_boundary"
    }
    async fn process_controlled(
        &self,
        request: &RunProcessingInput,
        track: &AudioTrack,
        run: &ProcessingRun,
        _control: ProcessingControl,
    ) -> Result<EngineOutput, CapabilityError> {
        self.process(request, track, run).await
    }
    async fn process(
        &self,
        request: &RunProcessingInput,
        track: &AudioTrack,
        run: &ProcessingRun,
    ) -> Result<EngineOutput, CapabilityError>;
}
#[derive(Debug, Clone)]
pub struct EmbeddingRequest {
    pub track: AudioTrack,
    pub start_ms: i64,
    pub end_ms: i64,
    pub model: String,
}
#[async_trait]
pub trait VoiceEmbeddings: Send + Sync {
    async fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingOutput, CapabilityError>;
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingOutput {
    pub model: String,
    pub revision: String,
    pub vector: Vec<f32>,
    pub metrics: Value,
}
#[async_trait]
pub trait RunWrites: Send + Sync {
    async fn start(&self, run: &ProcessingRun) -> Result<(), PersistenceError>;
    async fn finish(
        &self,
        run: &ProcessingRun,
        segments: &[TranscriptSegment],
    ) -> Result<(), PersistenceError>;
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicationGroup {
    pub id: String,
    pub track_ids: Vec<String>,
}
pub struct ProcessingService {
    queries: Arc<dyn LibraryQueries>,
    writes: Arc<dyn RunWrites>,
    models: Arc<dyn LocalModels>,
    engine: Arc<dyn SpeechProcessing>,
    runs: Arc<dyn ProcessingQueries>,
    ownership: Arc<dyn ProcessingOwnership>,
    control_writes: Arc<dyn RunControlWrites>,
}
impl ProcessingService {
    pub fn new(
        queries: Arc<dyn LibraryQueries>,
        writes: Arc<dyn RunWrites>,
        models: Arc<dyn LocalModels>,
        engine: Arc<dyn SpeechProcessing>,
        runs: Arc<dyn ProcessingQueries>,
        ownership: Arc<dyn ProcessingOwnership>,
        control_writes: Arc<dyn RunControlWrites>,
    ) -> Self {
        Self {
            queries,
            writes,
            models,
            engine,
            runs,
            ownership,
            control_writes,
        }
    }
    pub async fn run(&self, request: RunProcessingInput) -> Result<ProcessingRun, ProcessingError> {
        self.run_inner(request, None, None, None, None).await
    }
    pub(crate) async fn run_meeting(
        &self,
        request: RunProcessingInput,
        finalization_id: &str,
        expected_revision: &str,
    ) -> Result<ProcessingRun, ProcessingError> {
        self.run_inner(
            request,
            None,
            Some((finalization_id, expected_revision)),
            None,
            None,
        )
        .await
    }
    pub(crate) async fn run_analysis(
        &self,
        request: RunProcessingInput,
        id: &str,
        revision: &str,
        group: Option<&PublicationGroup>,
    ) -> Result<ProcessingRun, ProcessingError> {
        self.run_inner(request, None, None, group, Some((id, revision)))
            .await
    }
    async fn run_inner(
        &self,
        mut request: RunProcessingInput,
        retry_of: Option<&ProcessingRun>,
        finalization: Option<(&str, &str)>,
        publication_group: Option<&PublicationGroup>,
        analysis: Option<(&str, &str)>,
    ) -> Result<ProcessingRun, ProcessingError> {
        if request.model == "selected-asr" || request.model == "selected-diarization" {
            let selected = self.models.selection().await?;
            request.model = if request.model == "selected-asr" {
                selected.asr
            } else {
                selected.diarization
            };
        }
        if request.start_ms < 0 || request.duration_ms.is_some_and(|d| d <= 0 || d > 7_200_000) {
            return Err(ValidationError("processing interval must be positive, <=2 hours, and start at a nonnegative position".into()).into());
        }
        let d = self
            .queries
            .detail(&request.recording_id)
            .await?
            .ok_or_else(|| ProcessingError::NotFound(request.recording_id.clone()))?;
        let track = d
            .tracks
            .iter()
            .find(|t| t.id == request.track_id)
            .ok_or_else(|| ProcessingError::NotFound(request.track_id.clone()))?;
        if track
            .duration_ms
            .is_some_and(|duration| request.start_ms >= duration)
            || request
                .start_ms
                .checked_add(request.duration_ms.unwrap_or(0))
                .is_none()
        {
            return Err(ValidationError(
                "processing interval starts outside the source or overflows its timing range"
                    .into(),
            )
            .into());
        }
        if !track.available {
            return Err(ProcessingError::Unavailable(track.path.clone()));
        }
        // Finalization retries the latest compatible unfinished source, while
        // completed sources remain handled by the meeting checkpoint.
        let retry_of = retry_of.or_else(|| {
            finalization.and_then(|(id, _)| {
                d.runs
                    .iter()
                    .filter(|run| {
                        ["failed", "interrupted"].contains(&run.status.as_str())
                            && run.model == request.model
                            && run.configuration["meeting_finalization_id"] == id
                            && run.configuration["request"] == json!(request)
                    })
                    .max_by(|a, b| a.started_at.cmp(&b.started_at).then(a.id.cmp(&b.id)))
            })
        });
        let model = self.models.readiness(&request.model).await?;
        if request.word_timestamps && !model.experimental_word_timestamps {
            return Err(ValidationError(
                "selected model does not support experimental word timestamps".into(),
            )
            .into());
        }
        if retry_of.is_some_and(|old| old.model_revision != model.revision)
            || finalization.is_some_and(|(_, revision)| revision != model.revision)
            || analysis.is_some_and(|(_, revision)| revision != model.revision)
        {
            return Err(ProcessingError::Conflict("model revision changed; start an explicit new transcription instead of retrying old settings".into()));
        }
        if !model.verified {
            return Err(CapabilityError::message(
                "model is not verified; explicitly install/verify it first",
            )
            .into());
        }
        let meeting_group = if let Some((id, _)) = finalization {
            Some(PublicationGroup {
                id: id.into(),
                track_ids: serde_json::from_value(
                    d.recording.metadata["meeting_finalization"]["track_ids"].clone(),
                )
                .map_err(|e| ValidationError(format!("invalid expected meeting sources: {e}")))?,
            })
        } else {
            None
        };
        let publication_group = publication_group.or(meeting_group.as_ref());
        let mut run = ProcessingRun {
            id: new_id(),
            recording_id: request.recording_id.clone(),
            kind: if model.category == "asr" {
                "transcription"
            } else {
                "diarization"
            }
            .into(),
            model: model.id,
            model_revision: model.revision,
            status: "running".into(),
            started_at: now(),
            completed_at: None,
            configuration: json!({"request":request,"track":track,"receipt":receipt(),"ownership_protocol":OWNERSHIP_PROTOCOL,"control_protocol":CONTROL_PROTOCOL,"cancellation_mode":self.engine.cancellation_mode(&request.model),"retry_of_run_id":retry_of.map(|r| &r.id),"meeting_finalization_id":finalization.map(|(id,_)|id),"publication_group":publication_group,"meeting_analysis_id":analysis.map(|(id,_)|id)}),
            result: Value::Null,
            error: None,
        };
        let _lease =
            self.ownership.try_acquire(&run.id).await?.ok_or_else(|| {
                ProcessingError::Conflict("new run ownership is already held".into())
            })?;
        self.writes.start(&run).await?;
        let engine = self.engine.clone();
        let writes = self.writes.clone();
        let queries = self.runs.clone();
        let control_writes = self.control_writes.clone();
        let track = track.clone();
        // The driver owns the lease independently of its caller. Dropping a UI
        // waiter must not release ownership while a blocking SDK still runs.
        tokio::spawn(async move {
            let _lease = _lease;
            let control = ProcessingControl::default();
            let mut persistence_failure = None;
            let mut saved_sequence = None;
            let output = {
                let worker = engine.process_controlled(&request, &track, &run, control.clone());
                tokio::pin!(worker);
                loop {
                    tokio::select! {
                        result = &mut worker => break result,
                        _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                            match queries.control_state(&run.id).await {
                                Ok(Some(state)) if state.cancellation_requested_at.is_some() => control.request_cancellation(),
                                Ok(Some(_)) => {},
                                Ok(None) => {
                                    control.request_cancellation();
                                    persistence_failure = Some(PersistenceError::new(std::io::Error::other("active run disappeared")));
                                },
                                Err(e) => {
                                    control.request_cancellation();
                                    persistence_failure = Some(e);
                                },
                            }
                            let progress = control.snapshot();
                            if saved_sequence != Some(progress.sequence) && persistence_failure.is_none() {
                                match control_writes.save_progress(&run.id, &progress).await {
                                    Ok(true) => saved_sequence = Some(progress.sequence),
                                    Ok(false) => {
                                        control.request_cancellation();
                                        persistence_failure = Some(PersistenceError::new(std::io::Error::other("active progress was not saved")));
                                    },
                                    Err(e) => {
                                        control.request_cancellation();
                                        persistence_failure = Some(e);
                                    },
                                }
                            }
                        }
                    }
                }
            };
            // Observe a request that raced with the final worker callback.
            if queries.control_state(&run.id).await?.is_some_and(|s| s.cancellation_requested_at.is_some()) {
                control.request_cancellation();
            }
            let output = output.and_then(|output| {
                let mut ids = std::collections::BTreeSet::new();
                for segment in &output.segments {
                    segment.validate().map_err(CapabilityError::new)?;
                    if segment.recording_id != run.recording_id || segment.run_id != run.id
                        || segment.track_id.as_deref() != Some(track.id.as_str()) || !ids.insert(&segment.id) {
                        return Err(CapabilityError::message("engine returned a segment outside its recording/run/track or a duplicate ID"));
                    }
                }
                for interval in &output.intervals {
                    interval.validate().map_err(CapabilityError::new)?;
                    if interval.track_id.as_deref() != Some(track.id.as_str()) {
                        return Err(CapabilityError::message("engine returned speaker activity for a different track"));
                    }
                }
                Ok(output)
            });
            run.completed_at = Some(now());
            let mut segments = vec![];
            let mut engine_error = None;
            match output {
                Ok(output) => {
                    segments = output.segments;
                    run.status = "completed".into();
                    run.result = json!({"raw":output.raw,"intervals":output.intervals,"metrics":output.metrics});
                },
                Err(error) => {
                    run.status = "failed".into();
                    run.error = Some(error.to_string());
                    run.result = json!({"worker_error":error.to_string()});
                    engine_error = Some(error);
                },
            }
            if control.is_cancelled() {
                run.status = "interrupted".into();
                run.error = Some(if persistence_failure.is_some() {
                    "Stopped after progress/control persistence failed; source audio and completed chunks retained"
                } else {
                    "Cancellation requested; worker stopped, source audio and completed chunk artifacts retained"
                }.into());
                if !segments.is_empty() {
                    run.result["unpublished_segments"] = json!(segments);
                }
                segments.clear();
            }
            let mut progress = control.snapshot();
            progress.sequence += 1;
            progress.stage = match run.status.as_str() {
                "completed" => ProcessingStage::Completed,
                "interrupted" => ProcessingStage::Interrupted,
                _ => ProcessingStage::Failed,
            };
            run.result["progress"] = json!(progress);
            if let Err(error) = writes.finish(&run, &segments).await {
                if run.status != "interrupted" && queries.control_state(&run.id).await?.is_some_and(|s| s.cancellation_requested_at.is_some()) {
                    // The SQL write arbitrates cancellation versus completion.
                    run.status = "interrupted".into();
                    run.error = Some("Cancellation won terminal settlement; worker stopped, output retained without publication".into());
                    run.result["unpublished_segments"] = json!(segments);
                    progress.stage = ProcessingStage::Interrupted;
                    run.result["progress"] = json!(progress);
                    writes.finish(&run, &[]).await?;
                } else {
                    return Err(ProcessingError::Persistence(error));
                }
            }
            if let Some(error) = persistence_failure {
                return Err(ProcessingError::Persistence(error));
            }
            match run.status.as_str() {
                "completed" => Ok(run),
                "interrupted" => Err(ProcessingError::Cancelled(run.id)),
                _ => Err(ProcessingError::Engine(engine_error.unwrap_or_else(|| CapabilityError::message(run.error.unwrap_or_else(|| "processing failed".into()))))),
            }
        }).await.map_err(|e| ProcessingError::Engine(CapabilityError::new(e)))?
    }
}
#[derive(Debug, thiserror::Error)]
pub enum RetryProcessingError {
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error("{0}")]
    Processing(#[from] ProcessingError),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Invalid(String),
}
impl RetryProcessingError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Persistence(_) => "persistence",
            Self::Processing(ProcessingError::Cancelled(_)) => "cancelled",
            Self::Processing(_) => "nested",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Invalid(_) => "invalid_input",
        }
    }
}
impl ProcessingService {
    pub async fn retry(&self, run_id: &str) -> Result<ProcessingRun, RetryProcessingError> {
        let prior = self
            .runs
            .run(run_id)
            .await?
            .ok_or_else(|| RetryProcessingError::NotFound(run_id.into()))?;
        if !["failed", "interrupted"].contains(&prior.status.as_str()) {
            return Err(RetryProcessingError::Conflict("only failed or interrupted runs can be retried; recover an abandoned running job first".into()));
        }
        let mut request: RunProcessingInput = serde_json::from_value(prior.configuration["request"].clone())
            .map_err(|_| RetryProcessingError::Invalid("this run has no reproducible speech request; cluster edits must be repeated explicitly".into()))?;
        request.model = prior.model.clone();
        if request.recording_id != prior.recording_id {
            return Err(RetryProcessingError::Invalid(
                "stored request belongs to a different recording".into(),
            ));
        }
        let meeting_id = prior.configuration["meeting_finalization_id"].as_str();
        let publication_group: Option<PublicationGroup> = prior
            .configuration
            .get("publication_group")
            .filter(|v| !v.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| {
                RetryProcessingError::Invalid(format!("invalid publication group: {e}"))
            })?;
        Ok(self
            .run_inner(
                request,
                Some(&prior),
                meeting_id.map(|id| (id, prior.model_revision.as_str())),
                publication_group.as_ref(),
                prior.configuration["meeting_analysis_id"]
                    .as_str()
                    .map(|id| (id, prior.model_revision.as_str())),
            )
            .await?)
    }
}
#[derive(Debug, thiserror::Error)]
pub enum TranscribeRecordingError {
    #[error(transparent)]
    Read(#[from] PersistenceError),
    #[error("{0}")]
    Processing(#[from] ProcessingError),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("recording {recording_id} transcription is incomplete; retained source results")]
    Incomplete {
        recording_id: String,
        completed_run_ids: Vec<String>,
        failures: Vec<SourceProcessingFailure>,
    },
}
impl TranscribeRecordingError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Read(_) => "persistence",
            Self::Processing(ProcessingError::Cancelled(_)) => "cancelled",
            Self::Processing(_) => "nested",
            Self::NotFound(_) => "not_found",
            Self::Unavailable(_) => "source",
            Self::Incomplete { .. } => "processing",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceProcessingFailure {
    pub track_id: String,
    pub error: String,
}
impl TranscribeRecordingError {
    pub fn retained(&self) -> Option<Value> {
        match self {
            Self::Incomplete {
                recording_id,
                completed_run_ids,
                failures,
            } => Some(
                json!({"recording_id":recording_id,"completed_run_ids":completed_run_ids,"failures":failures}),
            ),
            _ => None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct TranscribeRecordingInput {
    pub id: String,
    pub model: String,
}
impl ProcessingService {
    /// Full source-aware final pass. Playback is never silently substituted for missing raw sources.
    pub async fn recording(
        &self,
        input: TranscribeRecordingInput,
    ) -> Result<Vec<ProcessingRun>, TranscribeRecordingError> {
        let id = input.id.as_str();
        let model = input.model.as_str();
        let d = self
            .queries
            .detail(id)
            .await?
            .ok_or_else(|| TranscribeRecordingError::NotFound(id.into()))?;
        let tracks: Vec<_> = d
            .tracks
            .iter()
            .filter(|t| t.available && t.kind != TrackKind::Playback)
            .collect();
        if tracks.is_empty() {
            return Err(TranscribeRecordingError::Unavailable(
                "no authoritative source audio".into(),
            ));
        }
        let mut results = vec![];
        let mut failures: Vec<SourceProcessingFailure> = d
            .tracks
            .iter()
            .filter(|t| t.kind != TrackKind::Playback && !t.available)
            .map(|t| SourceProcessingFailure {
                track_id: t.id.clone(),
                error: "authoritative source unavailable".into(),
            })
            .collect();
        let group = PublicationGroup {
            id: new_id(),
            track_ids: d
                .tracks
                .iter()
                .filter(|t| t.kind != TrackKind::Playback)
                .map(|t| t.id.clone())
                .collect(),
        };
        for track in tracks {
            match self
                .run_inner(
                    RunProcessingInput {
                        recording_id: id.into(),
                        track_id: track.id.clone(),
                        model: model.into(),
                        start_ms: 0,
                        duration_ms: None,
                        word_timestamps: false,
                    },
                    None,
                    None,
                    Some(&group),
                    None,
                )
                .await
            {
                Ok(run) => results.push(run),
                Err(error @ ProcessingError::Cancelled(_)) => {
                    return Err(TranscribeRecordingError::Processing(error));
                }
                Err(error) => failures.push(SourceProcessingFailure {
                    track_id: track.id.clone(),
                    error: error.to_string(),
                }),
            }
        }
        if !failures.is_empty() {
            return Err(TranscribeRecordingError::Incomplete {
                recording_id: id.into(),
                completed_run_ids: results.iter().map(|r| r.id.clone()).collect(),
                failures,
            });
        }
        Ok(results)
    }
}
