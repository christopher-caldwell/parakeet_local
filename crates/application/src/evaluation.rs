use super::*;
use crate::processing::{ProcessRequest, ProcessingService};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationEntry {
    pub id: String,
    pub audio: String,
    pub reference: Option<String>,
    pub reference_text: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationSuite {
    pub id: String,
    pub dataset: String,
    pub split: String,
    pub labels: String,
    pub entries: Vec<EvaluationEntry>,
    pub models: Vec<String>,
    pub collar_ms: i64,
    pub score_overlap: bool,
    #[serde(default = "exact_words")]
    pub word_normalization: String,
}
fn exact_words() -> String {
    "exact".into()
}
#[async_trait]
pub trait EvaluationArtifacts: Send + Sync {
    async fn load_suite(&self, path: &Path) -> Result<EvaluationSuite, CapabilityError>;
    async fn reference(&self, path: &Path) -> Result<Vec<SpeakerInterval>, CapabilityError>;
    async fn fingerprint(&self, path: &Path) -> Result<String, CapabilityError>;
    async fn save_report(&self, id: &str, report: &Value) -> Result<String, CapabilityError>;
    async fn report(&self, id: &str) -> Result<Value, CapabilityError>;
}
operation_error!(RunEvaluationError,sources{Source:CapabilityError=>"source",Persistence:PersistenceError=>"persistence"},values{InvalidSuite=>"invalid_input"});
operation_error!(ReadEvaluationReportError,sources{Source:CapabilityError=>"source"},values{});
pub struct EvaluationService {
    artifacts: Arc<dyn EvaluationArtifacts>,
    source: Arc<dyn RecordingSource>,
    writes: Arc<dyn LibraryWrites>,
    processing: Arc<ProcessingService>,
}
impl EvaluationService {
    pub fn new(
        artifacts: Arc<dyn EvaluationArtifacts>,
        source: Arc<dyn RecordingSource>,
        writes: Arc<dyn LibraryWrites>,
        processing: Arc<ProcessingService>,
    ) -> Self {
        Self {
            artifacts,
            source,
            writes,
            processing,
        }
    }
    pub async fn run(&self, path: &Path) -> Result<Value, RunEvaluationError> {
        let suite = self.artifacts.load_suite(path).await?;
        if !["exact", "english_case_punctuation"].contains(&suite.word_normalization.as_str()) {
            return Err(RunEvaluationError::InvalidSuite(
                "unsupported word normalization".into(),
            ));
        }
        if suite.entries.is_empty()
            || suite.models.is_empty()
            || !matches!(
                suite.labels.as_str(),
                "verified_public" | "verified_personal" | "unverified_personal" | "synthetic"
            )
        {
            return Err(RunEvaluationError::InvalidSuite(
                "suite requires entries, models, and an explicit label status".into(),
            ));
        }
        if suite.labels == "unverified_personal"
            && suite
                .entries
                .iter()
                .any(|e| e.reference.is_some() || e.reference_text.is_some())
        {
            return Err(RunEvaluationError::InvalidSuite(
                "unverified personal predictions cannot be scored as ground truth".into(),
            ));
        }
        let id = new_id();
        let mut results = vec![];
        let started = now();
        for entry in &suite.entries {
            let bundle = self.source.import_file(Path::new(&entry.audio)).await?;
            self.writes
                .import(&bundle)
                .await
                .map_err(CapabilityError::new)?;
            let track = &bundle.tracks[0];
            let hash = self.artifacts.fingerprint(Path::new(&entry.audio)).await?;
            for model in &suite.models {
                let request = ProcessRequest {
                    recording_id: bundle.recording.id.clone(),
                    track_id: track.id.clone(),
                    model: model.clone(),
                    start_ms: 0,
                    duration_ms: None,
                    word_timestamps: false,
                };
                let result = match self.processing.run(request).await {
                    Ok(run) => {
                        let mut row = json!({"entry":entry.id,"audio_sha256":hash,"processing_run":run,"scores":null});
                        if let Some(reference) = &entry.reference
                            && run.kind == "diarization"
                        {
                            let truth = self.artifacts.reference(Path::new(reference)).await?;
                            let prediction: Vec<SpeakerInterval> =
                                serde_json::from_value(run.result["intervals"].clone())
                                    .map_err(CapabilityError::new)?;
                            let score = parakeet_domain::evaluation::score_diarization(
                                &truth,
                                &prediction,
                                suite.collar_ms,
                                suite.score_overlap,
                            )
                            .map_err(CapabilityError::new)?;
                            row["scores"] = json!(score);
                            row["reference_sha256"] =
                                json!(self.artifacts.fingerprint(Path::new(reference)).await?);
                        }
                        if let Some(reference) = &entry.reference_text
                            && run.kind == "transcription"
                        {
                            let text = run.result["raw"]
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|v| v["text"].as_str())
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                })
                                .unwrap_or_default();
                            let normalized_reference = parakeet_domain::evaluation::normalize_wer(
                                reference,
                                &suite.word_normalization,
                            )
                            .map_err(CapabilityError::new)?;
                            let normalized_prediction = parakeet_domain::evaluation::normalize_wer(
                                &text,
                                &suite.word_normalization,
                            )
                            .map_err(CapabilityError::new)?;
                            row["wer"] = json!(parakeet_domain::evaluation::word_error_rate(
                                &normalized_reference,
                                &normalized_prediction
                            ));
                            row["reference_words"] =
                                json!(normalized_reference.split_whitespace().count());
                            row["hypothesis_words"] =
                                json!(normalized_prediction.split_whitespace().count());
                            if normalized_reference.trim().is_empty() {
                                row["false_recognized_words"] =
                                    json!(normalized_prediction.split_whitespace().count());
                            }
                            row["wer_normalization"] = json!(suite.word_normalization);
                        }
                        row
                    }
                    Err(e) => {
                        json!({"entry":entry.id,"model":model,"audio_sha256":hash,"failure":e.to_string()})
                    }
                };
                results.push(result);
            }
        }
        let mut report = json!({"id":id,"started_at":started,"completed_at":now(),"suite":suite,"results":results,"application":receipt(),"accuracy_claim":"limited_to_explicit_labeled_inputs; no superiority claim"});
        let location = self.artifacts.save_report(&id, &report).await?;
        report["report_path"] = json!(location);
        Ok(report)
    }
    pub async fn report(&self, id: &str) -> Result<Value, ReadEvaluationReportError> {
        Ok(self.artifacts.report(id).await?)
    }
}
