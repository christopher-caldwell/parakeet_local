use super::*;
use crate::processing::{EmbeddingRequest, VoiceEmbeddings};
use parakeet_domain::recognition::{IdentityDecision, MatchPolicy, suggest};
mod identification;
pub use identification::*;
operation_error!(ListSpeakersError,sources{Persistence:PersistenceError=>"persistence"},values{});
operation_error!(ListVoiceExamplesError,sources{Persistence:PersistenceError=>"persistence"},values{});
operation_error!(CreateSpeakerError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence"},values{});
operation_error!(RenameSpeakerError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});
operation_error!(DeleteSpeakerError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});
operation_error!(RemoveVoiceExampleError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});
operation_error!(EnrollSpeakerError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence",Embedding:CapabilityError=>"model"},values{NotFound=>"not_found",ConfirmationRequired=>"confirmation_required"});
operation_error!(SuggestSpeakerError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence",Embedding:CapabilityError=>"model"},values{NotFound=>"not_found"});
operation_error!(CorrectSegmentError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found",Conflict=>"conflict"});
operation_error!(UndoCorrectionError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found",Conflict=>"conflict"});
operation_error!(EnrollmentCandidatesError,sources{Persistence:PersistenceError=>"persistence",Embedding:CapabilityError=>"model"},values{NotFound=>"not_found"});
#[derive(Debug, Clone)]
pub struct RenameSpeakerInput {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone)]
pub struct SuggestSpeakerInput {
    pub recording_id: String,
    pub track_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// Further clean ranges of the same cluster pooled into one embedding.
    pub more_ranges: Vec<(i64, i64)>,
    pub policy: MatchPolicy,
}
/// A saved correction plus the verified example it admitted, if any.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionOutcome {
    #[serde(flatten)]
    pub correction: Correction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<VoiceExampleSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example_note: Option<String>,
}
#[derive(Debug, Clone)]
pub struct CorrectSegmentInput {
    pub segment_id: String,
    pub profile_id: Option<String>,
    pub text: Option<String>,
    pub action: String,
}
#[async_trait]
pub trait SpeakerQueries: Send + Sync {
    async fn profiles(&self) -> Result<Vec<SpeakerProfile>, PersistenceError>;
    async fn examples(&self, profile: Option<&str>) -> Result<Vec<VoiceExample>, PersistenceError>;
    async fn segment(&self, id: &str) -> Result<Option<TranscriptSegment>, PersistenceError>;
    async fn corrections(&self, segment_id: &str) -> Result<Vec<Correction>, PersistenceError>;
    async fn suggestions(
        &self,
        recording_id: Option<&str>,
    ) -> Result<Vec<SpeakerSuggestion>, PersistenceError>;
    async fn suggestion(&self, id: &str) -> Result<Option<SpeakerSuggestion>, PersistenceError>;
    async fn identification(
        &self,
        id: &str,
    ) -> Result<Option<ClusterIdentification>, PersistenceError>;
    async fn identifications(
        &self,
        recording_id: Option<&str>,
    ) -> Result<Vec<ClusterIdentification>, PersistenceError>;
}
#[async_trait]
pub trait SpeakerWrites: Send + Sync {
    async fn save_suggestion(&self, suggestion: &SpeakerSuggestion)
    -> Result<(), PersistenceError>;
    async fn resolve_suggestion(
        &self,
        resolution: &SuggestionResolution,
        expected_revision: usize,
        correction: Option<&Correction>,
        expected_correction_revision: usize,
    ) -> Result<bool, PersistenceError>;
    async fn create_profile(&self, profile: &SpeakerProfile) -> Result<(), PersistenceError>;
    async fn rename_profile(&self, id: &str, name: &str) -> Result<bool, PersistenceError>;
    async fn delete_profile(&self, id: &str) -> Result<bool, PersistenceError>;
    async fn enroll(&self, example: &VoiceExample) -> Result<(), PersistenceError>;
    async fn remove_example(&self, id: &str) -> Result<bool, PersistenceError>;
    /// Atomically records a cluster identification, appends its segment corrections (each
    /// against the expected revision) and inserts the examples it admitted. `false` on conflict.
    async fn save_identification(
        &self,
        identification: &ClusterIdentification,
        corrections: &[(Correction, usize)],
        examples: &[VoiceExample],
    ) -> Result<bool, PersistenceError>;
    /// Atomically marks the identification undone, appends the reverting corrections and
    /// deletes the examples it admitted. `false` if anything changed meanwhile.
    async fn undo_identification(
        &self,
        identification: &ClusterIdentification,
        corrections: &[(Correction, usize)],
        example_ids: &[String],
    ) -> Result<bool, PersistenceError>;
    async fn correct(
        &self,
        correction: &Correction,
        expected_revision: usize,
    ) -> Result<bool, PersistenceError>;
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollSpeakerInput {
    pub profile_id: String,
    pub recording_id: String,
    pub track_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confirmed: bool,
}
/// Presentation/CLI projection deliberately excludes biometric vector contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceExampleSummary {
    pub id: String,
    pub profile_id: String,
    pub recording_id: String,
    pub track_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub model: String,
    pub revision: String,
    pub capture_domain: String,
    pub verified_at: String,
    pub vector_dimensions: usize,
    /// Present when the example was admitted from a user's identification or correction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<parakeet_domain::ExampleOrigin>,
}
impl From<VoiceExample> for VoiceExampleSummary {
    fn from(e: VoiceExample) -> Self {
        Self {
            id: e.id,
            profile_id: e.profile_id,
            recording_id: e.recording_id,
            track_id: e.track_id,
            start_ms: e.start_ms,
            end_ms: e.end_ms,
            model: e.model,
            revision: e.revision,
            capture_domain: e.capture_domain,
            verified_at: e.verified_at,
            vector_dimensions: e.vector.len(),
            origin: e.origin,
        }
    }
}
pub struct SpeakerService {
    queries: Arc<dyn SpeakerQueries>,
    writes: Arc<dyn SpeakerWrites>,
    library: Arc<dyn LibraryQueries>,
    engine: Arc<dyn VoiceEmbeddings>,
    models: Arc<dyn LocalModels>,
}
impl SpeakerService {
    pub fn new(
        queries: Arc<dyn SpeakerQueries>,
        writes: Arc<dyn SpeakerWrites>,
        library: Arc<dyn LibraryQueries>,
        engine: Arc<dyn VoiceEmbeddings>,
        models: Arc<dyn LocalModels>,
    ) -> Self {
        Self {
            queries,
            writes,
            library,
            engine,
            models,
        }
    }
    pub async fn profiles(&self) -> Result<Vec<SpeakerProfile>, ListSpeakersError> {
        Ok(self.queries.profiles().await?)
    }
    pub async fn examples(
        &self,
        id: Option<&str>,
    ) -> Result<Vec<VoiceExampleSummary>, ListVoiceExamplesError> {
        Ok(self
            .queries
            .examples(id)
            .await?
            .into_iter()
            .map(VoiceExampleSummary::from)
            .collect())
    }
    pub async fn create(&self, name: &str) -> Result<SpeakerProfile, CreateSpeakerError> {
        Recording::validate_title(name)?;
        let p = SpeakerProfile {
            id: new_id(),
            name: name.into(),
            created_at: now(),
        };
        self.writes.create_profile(&p).await?;
        Ok(p)
    }
    pub async fn rename(&self, input: RenameSpeakerInput) -> Result<(), RenameSpeakerError> {
        let id = input.id.as_str();
        let name = input.name.as_str();
        Recording::validate_title(name)?;
        if !self.writes.rename_profile(id, name).await? {
            return Err(RenameSpeakerError::NotFound(id.into()));
        }
        Ok(())
    }
    pub async fn delete(&self, id: &str) -> Result<(), DeleteSpeakerError> {
        if !self.writes.delete_profile(id).await? {
            return Err(DeleteSpeakerError::NotFound(id.into()));
        }
        Ok(())
    }
    pub async fn remove_example(&self, id: &str) -> Result<(), RemoveVoiceExampleError> {
        if !self.writes.remove_example(id).await? {
            return Err(RemoveVoiceExampleError::NotFound(id.into()));
        }
        Ok(())
    }
    pub async fn enroll(
        &self,
        input: EnrollSpeakerInput,
    ) -> Result<VoiceExampleSummary, EnrollSpeakerError> {
        if !input.confirmed {
            return Err(EnrollSpeakerError::ConfirmationRequired(
                "EnrollSpeakerInput requires explicit identity confirmation after listening".into(),
            ));
        }
        if input.start_ms < 0
            || input.end_ms - input.start_ms < 3_000
            || input.end_ms - input.start_ms > 30_000
        {
            return Err(
                ValidationError("use a verified, clean 3–30 second speech sample".into()).into(),
            );
        }
        if !self
            .queries
            .profiles()
            .await?
            .iter()
            .any(|p| p.id == input.profile_id)
        {
            return Err(EnrollSpeakerError::NotFound(input.profile_id));
        }
        let detail = self
            .library
            .detail(&input.recording_id)
            .await?
            .ok_or_else(|| EnrollSpeakerError::NotFound(input.recording_id.clone()))?;
        let track = detail
            .tracks
            .iter()
            .find(|t| t.id == input.track_id)
            .ok_or_else(|| EnrollSpeakerError::NotFound(input.track_id.clone()))?;
        let output = self
            .engine
            .embed(&EmbeddingRequest {
                track: track.clone(),
                start_ms: input.start_ms,
                end_ms: input.end_ms,
                more_ranges: vec![],
                model: self.models.selection().await?.embedding,
            })
            .await?;
        if recognition::cosine(&output.vector, &output.vector).is_none() {
            return Err(ValidationError("invalid embedding".into()).into());
        }
        if let Some(existing) = self
            .queries
            .examples(Some(&input.profile_id))
            .await?
            .into_iter()
            .find(|e| {
                e.recording_id == input.recording_id
                    && e.track_id == input.track_id
                    && e.start_ms == input.start_ms
                    && e.end_ms == input.end_ms
                    && e.model == output.model
                    && e.revision == output.revision
            })
        {
            return Ok(existing.into());
        }
        let example_id = stable_id(
            "voice-example",
            &format!(
                "{}:{}:{}:{}:{}:{}:{}",
                input.profile_id,
                input.recording_id,
                input.track_id,
                input.start_ms,
                input.end_ms,
                output.model,
                output.revision
            ),
        );
        let example = VoiceExample {
            id: example_id,
            profile_id: input.profile_id,
            recording_id: input.recording_id,
            track_id: input.track_id,
            start_ms: input.start_ms,
            end_ms: input.end_ms,
            model: output.model,
            revision: output.revision,
            capture_domain: format!("{:?}", track.kind).to_lowercase(),
            vector: output.vector,
            verified_at: now(),
            origin: None,
        };
        self.writes.enroll(&example).await?;
        Ok(example.into())
    }
    pub async fn suggest(
        &self,
        input: SuggestSpeakerInput,
    ) -> Result<SpeakerSuggestion, SuggestSpeakerError> {
        self.suggest_inner(input, None, None).await
    }
    pub(crate) async fn suggest_for_analysis(
        &self,
        input: SuggestSpeakerInput,
        model: &crate::analysis::AnalysisModel,
        id: &str,
    ) -> Result<SpeakerSuggestion, SuggestSpeakerError> {
        if let Some(existing) = self.queries.suggestion(id).await? {
            if existing.recording_id == input.recording_id
                && existing.track_id == input.track_id
                && existing.start_ms == input.start_ms
                && existing.end_ms == input.end_ms
                && existing.more_ranges == input.more_ranges
                && existing.source_model.as_ref() == Some(model)
                && json!(existing.policy) == json!(input.policy)
            {
                return Ok(existing);
            }
            return Err(ValidationError(
                "saved suggestion does not match analysis evidence".into(),
            )
            .into());
        }
        if self.models.readiness(&model.id).await?.revision != model.revision {
            return Err(ValidationError("analysis embedding model revision changed".into()).into());
        }
        self.suggest_inner(input, Some(model), Some(id)).await
    }
    async fn suggest_inner(
        &self,
        input: SuggestSpeakerInput,
        model: Option<&crate::analysis::AnalysisModel>,
        id: Option<&str>,
    ) -> Result<SpeakerSuggestion, SuggestSpeakerError> {
        let recording_id = input.recording_id.as_str();
        let track_id = input.track_id.as_str();
        let start_ms = input.start_ms;
        let end_ms = input.end_ms;
        let more_ranges = input.more_ranges;
        let policy = input.policy;
        policy.validate()?;
        let ranges = || std::iter::once((start_ms, end_ms)).chain(more_ranges.iter().copied());
        let total: i64 = ranges().map(|(a, b)| b - a).sum();
        if ranges().any(|(a, b)| a < 0 || b <= a || b - a > 30_000)
            || !(3000..=90_000).contains(&total)
        {
            return Err(ValidationError(
                "suggestion needs 3–90 seconds of clean speech in ranges of at most 30 seconds"
                    .into(),
            )
            .into());
        }
        let d = self
            .library
            .detail(recording_id)
            .await?
            .ok_or_else(|| SuggestSpeakerError::NotFound(recording_id.into()))?;
        let t = d
            .tracks
            .iter()
            .find(|t| t.id == track_id)
            .ok_or_else(|| SuggestSpeakerError::NotFound(track_id.into()))?;
        let e = self
            .engine
            .embed(&EmbeddingRequest {
                track: t.clone(),
                start_ms,
                end_ms,
                more_ranges: more_ranges.clone(),
                model: if let Some(model) = model {
                    model.id.clone()
                } else {
                    self.models.selection().await?.embedding
                },
            })
            .await?;
        let decision = suggest(
            &e.vector,
            &e.model,
            &e.revision,
            &format!("{:?}", t.kind).to_lowercase(),
            &self.queries.examples(None).await?,
            &policy,
        )?;
        let receipt = SpeakerSuggestion {
            id: id.map(str::to_owned).unwrap_or_else(new_id),
            recording_id: recording_id.into(),
            track_id: track_id.into(),
            start_ms,
            end_ms,
            more_ranges,
            model: e.model,
            revision: e.revision,
            source_model: model.cloned(),
            policy,
            decision,
            created_at: now(),
            resolutions: vec![],
        };
        self.writes.save_suggestion(&receipt).await?;
        Ok(receipt)
    }
    pub async fn correct(
        &self,
        input: CorrectSegmentInput,
    ) -> Result<CorrectionOutcome, CorrectSegmentError> {
        let segment_id = input.segment_id.as_str();
        let profile_id = input.profile_id;
        let text = input.text;
        let action = input.action.as_str();
        if !["assign", "reject", "unknown", "text"].contains(&action) {
            return Err(ValidationError("unsupported correction action".into()).into());
        }
        if self.queries.segment(segment_id).await?.is_none() {
            return Err(CorrectSegmentError::NotFound(segment_id.into()));
        }
        if let Some(id) = &profile_id
            && !self.queries.profiles().await?.iter().any(|p| p.id == *id)
        {
            return Err(CorrectSegmentError::NotFound(id.clone()));
        }
        let previous = self.queries.corrections(segment_id).await?;
        let current = previous.last();
        let c = Correction {
            id: new_id(),
            segment_id: segment_id.into(),
            profile_id: if action == "text" {
                current.and_then(|c| c.profile_id.clone())
            } else {
                profile_id
            },
            replacement_text: if action == "text" {
                text
            } else {
                current.and_then(|c| c.replacement_text.clone())
            },
            action: action.into(),
            created_at: now(),
        };
        if !self.writes.correct(&c, previous.len()).await? {
            return Err(CorrectSegmentError::Conflict(
                "Another correction was saved; reread before applying this change".into(),
            ));
        }
        let (example, example_note) = match (&c.action[..], &c.profile_id) {
            ("assign", Some(profile)) => self.admit_for_correction(&c, profile).await,
            _ => (None, None),
        };
        Ok(CorrectionOutcome {
            correction: c,
            example,
            example_note,
        })
    }
    /// The user assigned a person to a segment: if that segment is clean single-speaker speech
    /// long enough to embed, keep it as a reversible verified example.
    async fn admit_for_correction(
        &self,
        correction: &Correction,
        profile: &str,
    ) -> (Option<VoiceExampleSummary>, Option<String>) {
        let note = |why: &str| (None, Some(format!("no example admitted: {why}")));
        let Ok(Some(segment)) = self.queries.segment(&correction.segment_id).await else {
            return note("segment unavailable");
        };
        let Some(track_id) = segment.track_id.clone() else {
            return note("segment has no source");
        };
        let Ok(Some(detail)) = self.library.detail(&segment.recording_id).await else {
            return note("recording unavailable");
        };
        let Some(range) = identification::segment_local_range(&segment, &detail) else {
            return note("segment timing is unresolved");
        };
        let Some(diarization) = current_diarization_runs(&detail.runs).remove(track_id.as_str())
        else {
            return note("no current diarization to confirm the speech is clean");
        };
        let intervals: Vec<SpeakerInterval> =
            serde_json::from_value(diarization.result["intervals"].clone()).unwrap_or_default();
        let attribution =
            parakeet_domain::reconciliation::reconcile(std::slice::from_ref(&segment), &intervals);
        if attribution
            .first()
            .is_none_or(|a| a.state != "anonymous_model_attribution" || a.overlapping_speech)
        {
            return note("segment is not clean single-speaker speech");
        }
        let (example, why) = self
            .admit_confirmed_interval(
                profile,
                &segment.recording_id,
                &track_id,
                range,
                parakeet_domain::ExampleOrigin {
                    verified_by: "user_identification".into(),
                    source_kind: "segment_correction".into(),
                    source_id: correction.id.clone(),
                },
            )
            .await;
        match example {
            Some(example) => match self.writes.enroll(&example).await {
                Ok(()) => (Some(example.into()), None),
                Err(error) => (None, Some(format!("no example admitted: {error}"))),
            },
            None => (None, why.map(|why| format!("no example admitted: {why}"))),
        }
    }
    /// Removes examples a user action admitted; used when that action is undone.
    async fn remove_admitted_examples(
        &self,
        source_kind: &str,
        source_id: &str,
    ) -> Result<(), PersistenceError> {
        for example in self.queries.examples(None).await? {
            if example
                .origin
                .as_ref()
                .is_some_and(|o| o.source_kind == source_kind && o.source_id == source_id)
            {
                self.writes.remove_example(&example.id).await?;
            }
        }
        Ok(())
    }
    pub async fn undo(&self, segment_id: &str) -> Result<Correction, UndoCorrectionError> {
        let history = self.queries.corrections(segment_id).await?;
        if history.is_empty() {
            return Err(UndoCorrectionError::NotFound(
                "no correction to undo".into(),
            ));
        }
        let mut stack = Vec::new();
        for event in &history {
            if event.action == "undo" {
                stack.pop();
            } else {
                stack.push(event);
            }
        }
        if stack.is_empty() {
            return Err(UndoCorrectionError::NotFound(
                "no correction to undo".into(),
            ));
        }
        let undone = stack.pop().map(|c| c.id.clone());
        let prior = stack.last();
        let c = Correction {
            id: new_id(),
            segment_id: segment_id.into(),
            profile_id: prior.and_then(|x| x.profile_id.clone()),
            replacement_text: prior.and_then(|x| x.replacement_text.clone()),
            action: "undo".into(),
            created_at: now(),
        };
        if !self.writes.correct(&c, history.len()).await? {
            return Err(UndoCorrectionError::Conflict(
                "Correction history changed; reread before undoing".into(),
            ));
        }
        if let Some(undone) = undone {
            self.remove_admitted_examples("segment_correction", &undone)
                .await?;
        }
        Ok(c)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentCandidate {
    pub id: String,
    pub recording_id: String,
    pub run_id: String,
    pub track_id: String,
    pub anonymous_speaker: String,
    pub local_start_ms: i64,
    pub local_end_ms: i64,
    /// Other clean ranges of the same cluster, pooled with the representative clip.
    #[serde(default)]
    pub more_ranges: Vec<(i64, i64)>,
    /// Total clean, non-overlapping speech pooled for this cluster.
    #[serde(default)]
    pub speech_ms: i64,
    pub note: String,
}
/// Shortest clean turn (after trimming its edges) that contributes to a pooled cluster embedding.
const MIN_CLEAN_WINDOW_MS: i64 = 1500;
const MAX_WINDOW_MS: i64 = 20_000;
const POOLED_SPEECH_BUDGET_MS: i64 = 60_000;
/// Clean (no other speaker active), edge-trimmed turns per anonymous speaker in source-local time.
pub(crate) fn clean_windows(
    intervals: &[SpeakerInterval],
    offset: i64,
) -> std::collections::BTreeMap<String, Vec<(i64, i64)>> {
    let mut clean = std::collections::BTreeMap::<String, Vec<(i64, i64)>>::new();
    for i in intervals {
        if intervals
            .iter()
            .any(|j| j.speaker != i.speaker && j.start_ms < i.end_ms && j.end_ms > i.start_ms)
        {
            continue;
        }
        let start = i.start_ms - offset + 250;
        let end = i.end_ms - offset - 250;
        if start >= 0 && end - start >= MIN_CLEAN_WINDOW_MS {
            clean
                .entry(i.speaker.clone())
                .or_default()
                .push((start, end));
        }
    }
    clean
}
/// Longest turns first within a speech budget, so pooling favours the most reliable speech.
pub(crate) fn pool_windows(mut windows: Vec<(i64, i64)>) -> (Vec<(i64, i64)>, i64) {
    windows.sort_by(|a, b| (b.1 - b.0).cmp(&(a.1 - a.0)).then(a.0.cmp(&b.0)));
    let mut pooled = vec![];
    let mut speech_ms = 0;
    for (start, end) in windows {
        let end = end.min(start + MAX_WINDOW_MS);
        if speech_ms + (end - start) > POOLED_SPEECH_BUDGET_MS {
            continue;
        }
        speech_ms += end - start;
        pooled.push((start, end));
    }
    (pooled, speech_ms)
}
impl SpeakerService {
    pub async fn candidates(
        &self,
        recording_id: &str,
    ) -> Result<Vec<EnrollmentCandidate>, EnrollmentCandidatesError> {
        self.candidates_inner(recording_id, None).await
    }
    pub(crate) async fn candidates_for_analysis(
        &self,
        id: &str,
        runs: &[String],
    ) -> Result<Vec<EnrollmentCandidate>, EnrollmentCandidatesError> {
        self.candidates_inner(id, Some(runs)).await
    }
    async fn candidates_inner(
        &self,
        recording_id: &str,
        run_ids: Option<&[String]>,
    ) -> Result<Vec<EnrollmentCandidate>, EnrollmentCandidatesError> {
        let d = self
            .library
            .detail(recording_id)
            .await?
            .ok_or_else(|| EnrollmentCandidatesError::NotFound(recording_id.into()))?;
        let latest = current_diarization_runs(&d.runs)
            .into_iter()
            .filter(|(_, r)| run_ids.is_none_or(|ids| ids.contains(&r.id)))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut selected =
            std::collections::BTreeMap::<(String, String), EnrollmentCandidate>::new();
        for (track_id, run) in latest {
            let Some(track) = d.tracks.iter().find(|t| t.id == track_id && t.available) else {
                continue;
            };
            let Some(offset) = track.meeting_offset_ms() else {
                continue;
            };
            let intervals: Vec<SpeakerInterval> =
                serde_json::from_value(run.result["intervals"].clone())
                    .map_err(|e| EnrollmentCandidatesError::Embedding(CapabilityError::new(e)))?;
            let clean = clean_windows(&intervals, offset);
            for (speaker, windows) in clean {
                let (pooled, speech_ms) = pool_windows(windows);
                if speech_ms < 3000 || pooled.is_empty() {
                    continue;
                }
                let (start, end) = pooled[0];
                let mut more_ranges = pooled[1..].to_vec();
                more_ranges.sort();
                selected.insert(
                    (track_id.clone(), speaker.clone()),
                    EnrollmentCandidate {
                        id: stable_id("candidate", &format!("{}:{speaker}", run.id)),
                        recording_id: recording_id.into(),
                        run_id: run.id.clone(),
                        track_id: track_id.clone(),
                        anonymous_speaker: speaker,
                        local_start_ms: start,
                        local_end_ms: end,
                        more_ranges,
                        speech_ms,
                        note: "Model predicts one nonoverlapping voice; listen before confirming identity".into(),
                    },
                );
            }
        }
        Ok(selected.into_values().collect())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerSuggestion {
    /// Runtime catalog identity is distinct from the embedding vector-space identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_model: Option<crate::analysis::AnalysisModel>,
    pub id: String,
    pub recording_id: String,
    pub track_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// Additional pooled ranges; empty for a single-interval comparison.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more_ranges: Vec<(i64, i64)>,
    pub model: String,
    pub revision: String,
    pub policy: MatchPolicy,
    pub decision: IdentityDecision,
    pub created_at: String,
    #[serde(default)]
    pub resolutions: Vec<SuggestionResolution>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestionResolution {
    pub id: String,
    pub suggestion_id: String,
    pub action: String,
    pub profile_id: Option<String>,
    pub segment_id: Option<String>,
    pub correction_id: Option<String>,
    pub created_at: String,
    /// Reported on the call that created the resolution; the example is also listed under the
    /// person and removed again when the decision is undone. Not stored with the resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admitted_example_id: Option<String>,
}
#[derive(Debug, Clone)]
pub struct ResolveSuggestionInput {
    pub id: String,
    pub action: String,
    pub segment_id: Option<String>,
    pub profile_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerSuggestionReview {
    pub suggestion: SpeakerSuggestion,
    pub eligible_segments: Vec<TranscriptSegment>,
}
operation_error!(ReviewSuggestionError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});
fn covered_by_suggestion(
    segment: &TranscriptSegment,
    detail: &RecordingDetail,
    suggestion: &SpeakerSuggestion,
) -> bool {
    if segment.recording_id != suggestion.recording_id
        || segment.track_id.as_ref() != Some(&suggestion.track_id)
    {
        return false;
    }
    let offset = detail
        .tracks
        .iter()
        .find(|t| t.id == suggestion.track_id)
        .and_then(AudioTrack::meeting_offset_ms);
    let start = segment.evidence["local_start_ms"].as_i64().or_else(|| {
        segment
            .start_ms
            .zip(offset)
            .and_then(|(t, o)| t.checked_sub(o))
    });
    let end = segment.evidence["local_end_ms"].as_i64().or_else(|| {
        segment
            .end_ms
            .zip(offset)
            .and_then(|(t, o)| t.checked_sub(o))
    });
    std::iter::once((suggestion.start_ms, suggestion.end_ms))
        .chain(suggestion.more_ranges.iter().copied())
        .any(|(from, to)| matches!((start,end),(Some(start),Some(end)) if start>=from && end<=to && end>start))
}
operation_error!(ListSuggestionsError,sources{Persistence:PersistenceError=>"persistence"},values{});
operation_error!(ResolveSuggestionError,sources{Persistence:PersistenceError=>"persistence",Invalid:ValidationError=>"invalid_input"},values{NotFound=>"not_found",Conflict=>"conflict"});
impl SpeakerService {
    /// Only current transcript segments fully covered by the compared source
    /// interval are offered. UI and resolution use the same evidence rule.
    pub async fn review_suggestion(
        &self,
        id: &str,
    ) -> Result<SpeakerSuggestionReview, ReviewSuggestionError> {
        let suggestion = self
            .queries
            .suggestion(id)
            .await?
            .ok_or_else(|| ReviewSuggestionError::NotFound(id.into()))?;
        let detail = self
            .library
            .detail(&suggestion.recording_id)
            .await?
            .ok_or_else(|| ReviewSuggestionError::NotFound(suggestion.recording_id.clone()))?;
        let eligible_segments = detail
            .segments
            .iter()
            .filter(|s| covered_by_suggestion(s, &detail, &suggestion))
            .cloned()
            .collect();
        Ok(SpeakerSuggestionReview {
            suggestion,
            eligible_segments,
        })
    }
    pub async fn suggestions(
        &self,
        recording: Option<&str>,
    ) -> Result<Vec<SpeakerSuggestion>, ListSuggestionsError> {
        Ok(self.queries.suggestions(recording).await?)
    }
    pub async fn resolve_suggestion(
        &self,
        input: ResolveSuggestionInput,
    ) -> Result<SuggestionResolution, ResolveSuggestionError> {
        if !["accept", "reject", "replace", "undo"].contains(&input.action.as_str()) {
            return Err(ValidationError("choose accept, reject, replace, or undo".into()).into());
        }
        let suggestion = self
            .queries
            .suggestion(&input.id)
            .await?
            .ok_or_else(|| ResolveSuggestionError::NotFound(input.id.clone()))?;
        let latest = suggestion.resolutions.last();
        if input.action != "undo" && latest.is_some_and(|r| r.action != "undo") {
            return Err(ResolveSuggestionError::Conflict(
                "suggestion is resolved; undo that decision before replacing it".into(),
            ));
        }
        if input.action == "undo" && latest.is_none_or(|r| r.action == "undo") {
            return Err(ResolveSuggestionError::Conflict(
                "no active suggestion decision to undo".into(),
            ));
        }
        let profile = if input.action == "accept" {
            Some(
                suggestion
                    .decision
                    .suggested_profile
                    .clone()
                    .ok_or_else(|| {
                        ValidationError("Unknown has no predicted identity to accept".into())
                    })?,
            )
        } else if input.action == "replace" {
            Some(input.profile_id.clone().ok_or_else(|| {
                ValidationError("replacement needs an explicitly chosen profile".into())
            })?)
        } else {
            None
        };
        if let Some(id) = &profile
            && !self.queries.profiles().await?.iter().any(|p| p.id == *id)
        {
            return Err(ResolveSuggestionError::NotFound(id.clone()));
        }
        let mut expected_correction = 0;
        let mut correction = None;
        if let Some(profile_id) = &profile {
            let segment_id = input.segment_id.as_ref().ok_or_else(|| {
                ValidationError(
                    "acceptance requires an explicitly chosen transcript segment".into(),
                )
            })?;
            let segment = self
                .queries
                .segment(segment_id)
                .await?
                .ok_or_else(|| ResolveSuggestionError::NotFound(segment_id.clone()))?;
            if segment.recording_id != suggestion.recording_id
                || segment.track_id.as_ref() != Some(&suggestion.track_id)
            {
                return Err(ValidationError(
                    "segment must belong to the suggested recording and source".into(),
                )
                .into());
            }
            let detail = self
                .library
                .detail(&suggestion.recording_id)
                .await?
                .ok_or_else(|| ResolveSuggestionError::NotFound(suggestion.recording_id.clone()))?;
            if !covered_by_suggestion(&segment, &detail, &suggestion) {
                return Err(ValidationError("suggestion evidence must cover the entire selected segment; listen and use a separate manual assignment for other intervals".into()).into());
            }
            let history = self.queries.corrections(segment_id).await?;
            expected_correction = history.len();
            correction = Some(Correction {
                id: new_id(),
                segment_id: segment_id.clone(),
                profile_id: Some(profile_id.clone()),
                replacement_text: history.last().and_then(|c| c.replacement_text.clone()),
                action: "assign".into(),
                created_at: now(),
            });
        }
        if input.action == "undo" {
            let latest = latest.expect("validated latest resolution");
            if let (Some(correction_id), Some(segment_id)) =
                (&latest.correction_id, &latest.segment_id)
            {
                let history = self.queries.corrections(segment_id).await?;
                if history.last().map(|c| &c.id) != Some(correction_id) {
                    return Err(ResolveSuggestionError::Conflict(
                        "newer transcript corrections exist; inspect and undo those first".into(),
                    ));
                }
                expected_correction = history.len();
                let mut active = Vec::new();
                for event in &history {
                    if event.action == "undo" {
                        active.pop();
                    } else {
                        active.push(event);
                    }
                }
                active.pop();
                let prior = active.last();
                correction = Some(Correction {
                    id: new_id(),
                    segment_id: segment_id.clone(),
                    profile_id: prior.and_then(|c| c.profile_id.clone()),
                    replacement_text: prior.and_then(|c| c.replacement_text.clone()),
                    action: "undo".into(),
                    created_at: now(),
                });
            }
        }
        let resolution = SuggestionResolution {
            id: new_id(),
            suggestion_id: input.id,
            action: input.action,
            profile_id: profile,
            segment_id: correction.as_ref().map(|c| c.segment_id.clone()),
            correction_id: correction.as_ref().map(|c| c.id.clone()),
            created_at: now(),
            admitted_example_id: None,
        };
        if !self
            .writes
            .resolve_suggestion(
                &resolution,
                suggestion.resolutions.len(),
                correction.as_ref(),
                expected_correction,
            )
            .await?
        {
            return Err(ResolveSuggestionError::Conflict(
                "suggestion or correction changed; inspect before resolving again".into(),
            ));
        }
        let mut resolution = resolution;
        if let Some(profile) = &resolution.profile_id {
            // The user confirmed this person for this evidence, so keep it as a reversible example.
            let (example, _) = self
                .admit_confirmed_interval(
                    profile,
                    &suggestion.recording_id,
                    &suggestion.track_id,
                    (suggestion.start_ms, suggestion.end_ms),
                    parakeet_domain::ExampleOrigin {
                        verified_by: "user_identification".into(),
                        source_kind: "suggestion_resolution".into(),
                        source_id: resolution.id.clone(),
                    },
                )
                .await;
            if let Some(example) = example {
                self.writes.enroll(&example).await?;
                resolution.admitted_example_id = Some(example.id);
            }
        } else if resolution.action == "undo"
            && let Some(undone) = latest
        {
            self.remove_admitted_examples("suggestion_resolution", &undone.id)
                .await?;
        }
        Ok(resolution)
    }
}

#[cfg(test)]
mod pooling_tests {
    use super::*;
    fn interval(speaker: &str, start_ms: i64, end_ms: i64) -> SpeakerInterval {
        SpeakerInterval {
            start_ms,
            end_ms,
            speaker: speaker.into(),
            track_id: Some("t".into()),
        }
    }
    #[test]
    fn a_cluster_of_only_short_turns_pools_enough_clean_speech() {
        let intervals: Vec<_> = (0..4)
            .map(|n| interval("a", n * 10_000, n * 10_000 + 2_500))
            .collect();
        let windows = clean_windows(&intervals, 0).remove("a").unwrap();
        assert_eq!(windows.len(), 4);
        let (pooled, speech_ms) = pool_windows(windows);
        assert_eq!(pooled.len(), 4);
        assert_eq!(speech_ms, 4 * 2_000);
    }
    #[test]
    fn overlapping_and_too_short_turns_are_not_clean() {
        let intervals = vec![
            interval("a", 0, 5_000),
            interval("b", 4_000, 9_000),
            interval("a", 20_000, 20_800),
        ];
        let clean = clean_windows(&intervals, 0);
        assert!(clean.is_empty());
    }
    #[test]
    fn more_than_twelve_clusters_are_all_kept_and_pooling_is_bounded() {
        let intervals: Vec<_> = (0..20)
            .map(|n| interval(&format!("s{n:02}"), n * 10_000, n * 10_000 + 5_000))
            .collect();
        assert_eq!(clean_windows(&intervals, 0).len(), 20);
        let (pooled, speech_ms) =
            pool_windows((0..10).map(|n| (n * 30_000, n * 30_000 + 20_000)).collect());
        assert!(speech_ms <= POOLED_SPEECH_BUDGET_MS);
        assert_eq!(pooled.len(), 3);
    }
}
