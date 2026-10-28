use super::*;
use parakeet_domain::speaker_wer::{SpeakerText, SpeakerWordScore};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerTranscript {
    pub dataset: String,
    pub labels: String,
    pub audio_sha256: String,
    /// Each text is already concatenated in chronological order within a voice.
    pub streams: Vec<SpeakerText>,
    #[serde(default)]
    pub provenance: Value,
}
pub struct SpeakerTranscriptEvidence {
    pub transcript: SpeakerTranscript,
    pub sha256: String,
}
#[async_trait]
pub trait ScoringInputReader: Send + Sync {
    async fn speaker_transcript(
        &self,
        path: &Path,
    ) -> Result<SpeakerTranscriptEvidence, CapabilityError>;
    async fn intervals(&self, path: &Path) -> Result<Vec<SpeakerInterval>, CapabilityError>;
}
#[derive(Debug, Clone)]
pub struct ScoreSpeakerWordsInput {
    pub reference: PathBuf,
    pub prediction: PathBuf,
    pub normalization: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerWordsReport {
    pub score: SpeakerWordScore,
    pub dataset: String,
    pub labels: String,
    pub audio_sha256: String,
    pub reference_sha256: String,
    pub prediction_sha256: String,
    pub reference_provenance: Value,
    pub prediction_provenance: Value,
    pub limitations: String,
    pub application: Value,
}
#[derive(Debug, Clone)]
pub struct ScoreDiarizationInput {
    pub reference: PathBuf,
    pub prediction: PathBuf,
    pub collar_ms: i64,
    pub score_overlap: bool,
}
operation_error!(ScoreDiarizationError,sources{Source:CapabilityError=>"source",Invalid:ValidationError=>"invalid_input"},values{});
operation_error!(ScoreSpeakerWordsError,sources{Source:CapabilityError=>"source",Invalid:ValidationError=>"invalid_input"},values{});
pub struct ScoringService {
    reader: Arc<dyn ScoringInputReader>,
}
impl ScoringService {
    pub fn new(reader: Arc<dyn ScoringInputReader>) -> Self {
        Self { reader }
    }
    pub async fn diarization(
        &self,
        input: ScoreDiarizationInput,
    ) -> Result<parakeet_domain::evaluation::DiarizationScore, ScoreDiarizationError> {
        let reference = self.reader.intervals(&input.reference).await?;
        let prediction = self.reader.intervals(&input.prediction).await?;
        Ok(parakeet_domain::evaluation::score_diarization(
            &reference,
            &prediction,
            input.collar_ms,
            input.score_overlap,
        )?)
    }
    pub async fn speaker_words(
        &self,
        input: ScoreSpeakerWordsInput,
    ) -> Result<SpeakerWordsReport, ScoreSpeakerWordsError> {
        let reference = self.reader.speaker_transcript(&input.reference).await?;
        let prediction = self.reader.speaker_transcript(&input.prediction).await?;
        let r = reference.transcript;
        let h = prediction.transcript;
        if !matches!(
            r.labels.as_str(),
            "verified_public" | "verified_personal" | "synthetic"
        ) || r.dataset.trim().is_empty()
        {
            return Err(ValidationError("speaker-word references require an explicit verified/synthetic label status and dataset".into()).into());
        }
        if r.audio_sha256.len() != 64
            || !r.audio_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || r.audio_sha256 != h.audio_sha256
        {
            return Err(ValidationError(
                "speaker-word inputs must declare the same SHA-256 audio identity".into(),
            )
            .into());
        }
        let score = parakeet_domain::speaker_wer::score_speaker_words(
            &r.streams,
            &h.streams,
            &input.normalization,
        )?;
        Ok(SpeakerWordsReport {
            score, dataset:r.dataset, labels:r.labels, audio_sha256:r.audio_sha256,
            reference_sha256:reference.sha256, prediction_sha256:prediction.sha256,
            reference_provenance:r.provenance, prediction_provenance:h.provenance,
            limitations:"anonymous-stream cpWER; no named-identity or word-timestamp accuracy claim; no time constraints; an unattributed stream may map to one reference voice, so inspect unattributed-word counts separately".into(),
            application:receipt(),
        })
    }
}
