use super::*;
use crate::processing::{EmbeddingRequest, VoiceEmbeddings};
use parakeet_domain::recognition::{IdentityDecision, MatchPolicy, suggest};
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
    pub policy: MatchPolicy,
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
        let policy = input.policy;
        policy.validate()?;
        if start_ms < 0 || end_ms - start_ms < 3000 || end_ms - start_ms > 30_000 {
            return Err(
                ValidationError("suggestion needs a 3–30 second clean sample".into()).into(),
            );
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
    ) -> Result<Correction, CorrectSegmentError> {
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
        Ok(c)
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
        stack.pop();
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
    pub note: String,
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
            for (ordinal, i) in intervals.iter().enumerate() {
                if i.end_ms - i.start_ms < 4000 {
                    continue;
                }
                if intervals.iter().any(|j| {
                    j.speaker != i.speaker && j.start_ms < i.end_ms && j.end_ms > i.start_ms
                }) {
                    continue;
                }
                let start = i.start_ms - offset + 500;
                let end = (i.end_ms - offset - 500).min(start + 20_000);
                if start < 0 || end - start < 3000 {
                    continue;
                }
                let candidate = EnrollmentCandidate {
                    id: stable_id("candidate", &format!("{}:{ordinal}", run.id)),
                    recording_id: recording_id.into(),
                    run_id: run.id.clone(),
                    track_id: track_id.clone(),
                    anonymous_speaker: i.speaker.clone(),
                    local_start_ms: start,
                    local_end_ms: end,
                    note:
                        "Model predicts one nonoverlapping voice; listen before confirming identity"
                            .into(),
                };
                let key = (track_id.clone(), i.speaker.clone());
                let replace = selected
                    .get(&key)
                    .is_none_or(|old| old.local_end_ms - old.local_start_ms < end - start);
                if replace {
                    selected.insert(key, candidate);
                }
            }
        }
        Ok(selected.into_values().take(12).collect())
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
    matches!((start,end),(Some(start),Some(end)) if start>=suggestion.start_ms && end<=suggestion.end_ms && end>start)
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
        Ok(resolution)
    }
}
