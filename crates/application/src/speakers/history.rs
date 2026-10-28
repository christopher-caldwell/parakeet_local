//! A person's notes, enrollment history and recognition history, assembled from stored records.
use super::*;

operation_error!(ProfileHistoryError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});
operation_error!(SaveNotesError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecognitionEvent {
    pub suggestion_id: String,
    pub recording_id: String,
    pub created_at: String,
    pub score: f64,
    /// `suggested`, `accepted`, `rejected`, `replaced` or `undone`; only the user's decisions
    /// confirm anything.
    pub outcome: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileHistory {
    pub profile: SpeakerProfile,
    pub notes: String,
    pub enrollments: Vec<VoiceExampleSummary>,
    pub recognitions: Vec<RecognitionEvent>,
}
impl SpeakerService {
    pub async fn history(&self, profile_id: &str) -> Result<ProfileHistory, ProfileHistoryError> {
        let profile = self
            .queries
            .profiles()
            .await?
            .into_iter()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| ProfileHistoryError::NotFound(profile_id.into()))?;
        let mut enrollments: Vec<VoiceExampleSummary> = self
            .queries
            .examples(Some(profile_id))
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        enrollments.sort_by(|a, b| a.verified_at.cmp(&b.verified_at).then(a.id.cmp(&b.id)));
        let mut recognitions = vec![];
        for suggestion in self.queries.suggestions(None).await? {
            let candidate = suggestion
                .decision
                .candidates
                .iter()
                .find(|c| c.profile_id == profile_id);
            let latest = suggestion.resolutions.last();
            let decided_for_profile =
                latest.is_some_and(|r| r.profile_id.as_deref() == Some(profile_id));
            let outcome = match latest {
                Some(r) if r.action == "undo" => "undone",
                Some(r)
                    if r.action == "reject"
                        && suggestion.decision.suggested_profile.as_deref() == Some(profile_id) =>
                {
                    "rejected"
                }
                Some(r) if decided_for_profile && r.action == "accept" => "accepted",
                Some(r) if decided_for_profile && r.action == "replace" => "replaced",
                _ if suggestion.decision.suggested_profile.as_deref() == Some(profile_id) => {
                    "suggested"
                }
                _ => continue,
            };
            recognitions.push(RecognitionEvent {
                suggestion_id: suggestion.id.clone(),
                recording_id: suggestion.recording_id.clone(),
                created_at: suggestion.created_at.clone(),
                score: candidate.map(|c| c.score).unwrap_or_default(),
                outcome: outcome.into(),
            });
        }
        Ok(ProfileHistory {
            profile,
            notes: self.queries.notes(profile_id).await?,
            enrollments,
            recognitions,
        })
    }
    pub async fn save_notes(&self, profile_id: &str, notes: &str) -> Result<(), SaveNotesError> {
        if notes.len() > 4_000 {
            return Err(SaveNotesError::NotFound(
                "notes are limited to 4000 characters".into(),
            ));
        }
        if !self.writes.save_notes(profile_id, notes).await? {
            return Err(SaveNotesError::NotFound(profile_id.into()));
        }
        Ok(())
    }
}
