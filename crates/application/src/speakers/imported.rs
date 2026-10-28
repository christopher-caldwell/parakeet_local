//! Candidate clips proposed from labels imported from another application.
//!
//! Imported labels are claims, not truth: they only help the user find clips worth listening to.
//! Nothing here enrolls anything, and the labels are never used to measure accuracy.
use super::*;

const MAX_CLIPS_PER_LABEL: usize = 5;
operation_error!(ImportedCandidatesError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedLabelClip {
    pub segment_id: String,
    pub track_id: String,
    pub local_start_ms: i64,
    pub local_end_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedLabelCandidates {
    /// The label exactly as imported; an unverified claim.
    pub claim: String,
    /// `Me` seeds the user's own profile, again only by per-clip confirmation.
    pub is_self: bool,
    pub clips: Vec<ImportedLabelClip>,
}
fn is_anonymous_label(claim: &str) -> bool {
    let lower = claim.to_lowercase();
    lower.starts_with("others")
        || lower.starts_with("speaker")
        || lower == "system"
        || lower == "microphone"
        || lower.is_empty()
}
impl SpeakerService {
    /// Per imported name, a few clean single-speaker clips for the user to confirm one by one.
    pub async fn imported_label_candidates(
        &self,
        recording_id: &str,
    ) -> Result<Vec<ImportedLabelCandidates>, ImportedCandidatesError> {
        let detail = self
            .library
            .detail(recording_id)
            .await?
            .ok_or_else(|| ImportedCandidatesError::NotFound(recording_id.into()))?;
        let imported: Vec<_> = detail
            .segments
            .iter()
            .filter(|s| s.evidence["origin"] == "imported_unverified")
            .collect();
        let mut by_claim = std::collections::BTreeMap::<String, Vec<ImportedLabelClip>>::new();
        for segment in &imported {
            let (Some(claim), Some(track_id)) =
                (segment.speaker_claim.as_deref(), &segment.track_id)
            else {
                continue;
            };
            if is_anonymous_label(claim) {
                continue;
            }
            let Some(track) = detail
                .tracks
                .iter()
                .find(|t| &t.id == track_id && t.available)
            else {
                continue;
            };
            let Some((start, end)) = identification::segment_local_range(segment, &detail) else {
                continue;
            };
            // Trim the edges and keep a bounded window; clean means nobody else is labelled
            // on that source during the clip.
            let (start, end) = (start + 250, (end - 250).min(start + 250 + 20_000));
            if !(3_000..=20_000).contains(&(end - start)) {
                continue;
            }
            let shared = imported.iter().any(|other| {
                other.id != segment.id
                    && other.track_id.as_ref() == Some(track_id)
                    && other.speaker_claim != segment.speaker_claim
                    && identification::segment_local_range(other, &detail)
                        .is_some_and(|(a, b)| a < end && b > start)
            });
            if shared {
                continue;
            }
            by_claim
                .entry(claim.to_owned())
                .or_default()
                .push(ImportedLabelClip {
                    segment_id: segment.id.clone(),
                    track_id: track_id.clone(),
                    local_start_ms: start,
                    local_end_ms: end,
                });
        }
        Ok(by_claim
            .into_iter()
            .map(|(claim, mut clips)| {
                // Longest clips first, then spread across the meeting rather than clustering.
                clips.sort_by(|a, b| {
                    (b.local_end_ms - b.local_start_ms)
                        .cmp(&(a.local_end_ms - a.local_start_ms))
                        .then(a.local_start_ms.cmp(&b.local_start_ms))
                });
                clips.truncate(MAX_CLIPS_PER_LABEL);
                clips.sort_by_key(|c| c.local_start_ms);
                ImportedLabelCandidates {
                    is_self: claim.eq_ignore_ascii_case("me"),
                    claim,
                    clips,
                }
            })
            .collect())
    }
}
