use super::*;
use crate::jobs::{OWNERSHIP_PROTOCOL, ProcessingOwnership, ProcessingQueries};

#[derive(Debug, thiserror::Error)]
pub enum ProcessingError {
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
pub struct ProcessRequest {
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
    async fn process(
        &self,
        request: &ProcessRequest,
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
pub struct ProcessingService {
    queries: Arc<dyn LibraryQueries>,
    writes: Arc<dyn RunWrites>,
    models: Arc<dyn LocalModels>,
    engine: Arc<dyn SpeechProcessing>,
    runs: Arc<dyn ProcessingQueries>,
    ownership: Arc<dyn ProcessingOwnership>,
}
impl ProcessingService {
    pub fn new(
        queries: Arc<dyn LibraryQueries>,
        writes: Arc<dyn RunWrites>,
        models: Arc<dyn LocalModels>,
        engine: Arc<dyn SpeechProcessing>,
        runs: Arc<dyn ProcessingQueries>,
        ownership: Arc<dyn ProcessingOwnership>,
    ) -> Self {
        Self {
            queries,
            writes,
            models,
            engine,
            runs,
            ownership,
        }
    }
    pub async fn run(&self, request: ProcessRequest) -> Result<ProcessingRun, ProcessingError> {
        self.run_inner(request, None).await
    }
    async fn run_inner(
        &self,
        mut request: ProcessRequest,
        retry_of: Option<&ProcessingRun>,
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
        let model = self.models.readiness(&request.model).await?;
        if request.word_timestamps && !model.experimental_word_timestamps {
            return Err(ValidationError(
                "selected model does not support experimental word timestamps".into(),
            )
            .into());
        }
        if retry_of.is_some_and(|old| old.model_revision != model.revision) {
            return Err(ProcessingError::Conflict("model revision changed; start an explicit new transcription instead of retrying old settings".into()));
        }
        if !model.verified {
            return Err(CapabilityError::message(
                "model is not verified; explicitly install/verify it first",
            )
            .into());
        }
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
            configuration: json!({"request":request,"track":track,"receipt":receipt(),"ownership_protocol":OWNERSHIP_PROTOCOL,"retry_of_run_id":retry_of.map(|r| &r.id)}),
            result: Value::Null,
            error: None,
        };
        let _lease =
            self.ownership.try_acquire(&run.id).await?.ok_or_else(|| {
                ProcessingError::Conflict("new run ownership is already held".into())
            })?;
        self.writes.start(&run).await?;
        let output = self
            .engine
            .process(&request, track, &run)
            .await
            .and_then(|output| {
                let mut ids=std::collections::BTreeSet::new();
                for s in &output.segments {
                    s.validate().map_err(CapabilityError::new)?;
                    if s.recording_id!=run.recording_id || s.run_id!=run.id || s.track_id.as_deref()!=Some(track.id.as_str()) || !ids.insert(&s.id) {
                        return Err(CapabilityError::message("engine returned a segment outside its recording/run/track or a duplicate ID"));
                    }
                }
                for i in &output.intervals {
                    i.validate().map_err(CapabilityError::new)?;
                    if i.track_id.as_deref()!=Some(track.id.as_str()) {return Err(CapabilityError::message("engine returned speaker activity for a different track"));}
                }
                Ok(output)
            });
        match output {
            Ok(output) => {
                run.status = "completed".into();
                run.completed_at = Some(now());
                run.result =
                    json!({"raw":output.raw,"intervals":output.intervals,"metrics":output.metrics});
                self.writes.finish(&run, &output.segments).await?;
                Ok(run)
            }
            Err(error) => {
                run.status = "failed".into();
                run.completed_at = Some(now());
                run.error = Some(error.to_string());
                self.writes.finish(&run, &[]).await?;
                Err(error.into())
            }
        }
    }
}
operation_error!(RetryProcessingError,sources{Persistence:PersistenceError=>"persistence",Processing:ProcessingError=>"nested"},values{NotFound=>"not_found",Conflict=>"conflict",Invalid=>"invalid_input"});
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
        let mut request: ProcessRequest = serde_json::from_value(prior.configuration["request"].clone())
            .map_err(|_| RetryProcessingError::Invalid("this run has no reproducible speech request; cluster edits must be repeated explicitly".into()))?;
        request.model = prior.model.clone();
        if request.recording_id != prior.recording_id {
            return Err(RetryProcessingError::Invalid(
                "stored request belongs to a different recording".into(),
            ));
        }
        Ok(self.run_inner(request, Some(&prior)).await?)
    }
}
operation_error!(TranscribeRecordingError,sources{Read:PersistenceError=>"persistence",Processing:ProcessingError=>"nested"},values{NotFound=>"not_found",Unavailable=>"source",Incomplete=>"processing"});
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
        let mut failures = vec![];
        for track in tracks {
            match self
                .run(ProcessRequest {
                    recording_id: id.into(),
                    track_id: track.id.clone(),
                    model: model.into(),
                    start_ms: 0,
                    duration_ms: None,
                    word_timestamps: false,
                })
                .await
            {
                Ok(run) => results.push(run),
                Err(error) => failures.push(json!({"track_id":track.id,"error":error.to_string()})),
            }
        }
        if !failures.is_empty() {
            return Err(TranscribeRecordingError::Incomplete(json!({"recording_id":id,"completed_run_ids":results.iter().map(|r|r.id.as_str()).collect::<Vec<_>>(),"failures":failures,"evidence":"all source results remain in recording history"}).to_string()));
        }
        Ok(results)
    }
}
