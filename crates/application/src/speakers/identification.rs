//! Naming a whole anonymous cluster, and admitting the user's confirmations as voice examples.
//!
//! A model prediction or imported label never reaches this module's examples: every example
//! carries an `ExampleOrigin` naming the explicit user action that admitted it, and each action
//! can be undone as one unit.
use super::*;
use parakeet_domain::ExampleOrigin;

/// Most examples one identification or correction may admit, and per person per recording.
const EXAMPLES_PER_IDENTIFICATION: usize = 3;
const EXAMPLES_PER_PERSON_PER_RECORDING: usize = 5;
const EXAMPLE_SPEECH_MS: std::ops::RangeInclusive<i64> = 3_000..=20_000;

operation_error!(IdentifyClusterError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found",ConfirmationRequired=>"confirmation_required",Conflict=>"conflict"});
operation_error!(UndoIdentificationError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found",Conflict=>"conflict"});
operation_error!(ListIdentificationsError,sources{Persistence:PersistenceError=>"persistence"},values{});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentifiedSegment {
    pub segment_id: String,
    pub correction_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedSegment {
    pub segment_id: String,
    /// `ambiguous_or_overlapping` or `already_corrected`.
    pub reason: String,
}
/// Durable record of one explicit "this cluster is Person X" decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterIdentification {
    pub id: String,
    pub recording_id: String,
    pub track_id: String,
    pub run_id: String,
    pub cluster: String,
    pub profile_id: String,
    pub created_at: String,
    pub segments: Vec<IdentifiedSegment>,
    pub skipped: Vec<SkippedSegment>,
    /// Examples this identification admitted; undo removes exactly these.
    pub example_ids: Vec<String>,
    /// Why fewer examples than possible were admitted (no model, too little clean speech, cap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example_note: Option<String>,
    #[serde(default)]
    pub undone_at: Option<String>,
}
#[derive(Debug, Clone)]
pub struct IdentifyClusterInput {
    pub recording_id: String,
    pub track_id: String,
    /// The diarization run the cluster label belongs to; must still be current for the source.
    pub run_id: String,
    pub cluster: String,
    pub person: PersonChoice,
    /// The user listened and decided; required, like enrollment.
    pub confirmed: bool,
    /// Source-local ranges the user actually played. Only speech inside them becomes a voice
    /// example; when empty, only the cluster's longest clean turn (the clip offered first) does.
    pub heard: Vec<(i64, i64)>,
}
#[derive(Debug, Clone)]
pub enum PersonChoice {
    Existing(String),
    New(String),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoIdentificationReport {
    pub identification: ClusterIdentification,
    pub reverted_segments: usize,
    /// Segments edited again after the identification; their newer corrections stay.
    pub left_in_place: Vec<String>,
    pub removed_examples: usize,
}
fn local_range(segment: &TranscriptSegment, offset: i64) -> Option<(i64, i64)> {
    let start = segment.evidence["local_start_ms"]
        .as_i64()
        .or_else(|| segment.start_ms.map(|t| t - offset))?;
    let end = segment.evidence["local_end_ms"]
        .as_i64()
        .or_else(|| segment.end_ms.map(|t| t - offset))?;
    (start >= 0 && end > start).then_some((start, end))
}
impl SpeakerService {
    pub async fn identify_cluster(
        &self,
        input: IdentifyClusterInput,
    ) -> Result<ClusterIdentification, IdentifyClusterError> {
        if !input.confirmed {
            return Err(IdentifyClusterError::ConfirmationRequired(
                "naming a cluster requires explicit confirmation after listening".into(),
            ));
        }
        let detail = self
            .library
            .detail(&input.recording_id)
            .await?
            .ok_or_else(|| IdentifyClusterError::NotFound(input.recording_id.clone()))?;
        let current = current_diarization_runs(&detail.runs);
        let run = current
            .get(&input.track_id)
            .filter(|run| run.id == input.run_id)
            .ok_or_else(|| {
                IdentifyClusterError::Conflict(
                    "that diarization run is not the current result for this source; reload".into(),
                )
            })?;
        let track = detail
            .tracks
            .iter()
            .find(|t| t.id == input.track_id)
            .ok_or_else(|| IdentifyClusterError::NotFound(input.track_id.clone()))?;
        let intervals: Vec<SpeakerInterval> =
            serde_json::from_value(run.result["intervals"].clone())
                .map_err(|e| ValidationError(e.to_string()))?;
        if !intervals.iter().any(|i| i.speaker == input.cluster) {
            return Err(IdentifyClusterError::NotFound(input.cluster));
        }
        let profiles = self.queries.profiles().await?;
        if let PersonChoice::Existing(id) = &input.person
            && !profiles.iter().any(|p| p.id == *id)
        {
            return Err(IdentifyClusterError::NotFound(id.clone()));
        }
        if let PersonChoice::New(name) = &input.person {
            self.unique_name(name, None).await??;
        }
        // Segments the cluster speaks alone; everything else is listed, never guessed.
        let attributions = parakeet_domain::reconciliation::reconcile(&detail.segments, &intervals);
        let mut targets = vec![];
        let mut skipped = vec![];
        for (segment, attribution) in detail.segments.iter().zip(&attributions) {
            if segment.track_id.as_deref() != Some(input.track_id.as_str())
                || !attribution.anonymous_speakers.contains(&input.cluster)
            {
                continue;
            }
            if attribution.state != "anonymous_model_attribution"
                || attribution.anonymous_speakers.len() != 1
                || attribution.overlapping_speech
            {
                skipped.push(SkippedSegment {
                    segment_id: segment.id.clone(),
                    reason: "ambiguous_or_overlapping".into(),
                });
            } else if !self.queries.corrections(&segment.id).await?.is_empty() {
                skipped.push(SkippedSegment {
                    segment_id: segment.id.clone(),
                    reason: "already_corrected".into(),
                });
            } else {
                targets.push(segment.id.clone());
            }
        }
        // Embed clean windows before any write so a failing model leaves nothing half-done.
        let offset = track.meeting_offset_ms().unwrap_or(0);
        let (profile_id, creating) = match &input.person {
            PersonChoice::Existing(id) => (id.clone(), false),
            PersonChoice::New(_) => (new_id(), true),
        };
        let identification_id = new_id();
        let origin = ExampleOrigin {
            verified_by: "user_identification".into(),
            source_kind: "cluster_identification".into(),
            source_id: identification_id.clone(),
        };
        let windows = heard_windows(
            clean_windows(&intervals, offset)
                .remove(&input.cluster)
                .unwrap_or_default(),
            &input.heard,
            &contradicted_ranges(&detail, &input.track_id, offset, &profile_id),
        );
        let (examples, example_note) = self
            .admit_cluster_examples(&profile_id, &detail, track, windows, origin, &[])
            .await;
        if let PersonChoice::New(name) = &input.person {
            Recording::validate_title(name)?;
            if creating {
                self.writes
                    .create_profile(&SpeakerProfile {
                        id: profile_id.clone(),
                        name: name.clone(),
                        created_at: now(),
                    })
                    .await?;
            }
        }
        let corrections: Vec<_> = targets
            .iter()
            .map(|segment_id| {
                (
                    Correction {
                        id: new_id(),
                        segment_id: segment_id.clone(),
                        profile_id: Some(profile_id.clone()),
                        replacement_text: None,
                        action: "assign".into(),
                        created_at: now(),
                    },
                    0usize,
                )
            })
            .collect();
        let identification = ClusterIdentification {
            id: identification_id,
            recording_id: input.recording_id,
            track_id: input.track_id,
            run_id: input.run_id,
            cluster: input.cluster,
            profile_id,
            created_at: now(),
            segments: corrections
                .iter()
                .map(|(c, _)| IdentifiedSegment {
                    segment_id: c.segment_id.clone(),
                    correction_id: c.id.clone(),
                })
                .collect(),
            skipped,
            example_ids: examples.iter().map(|e| e.id.clone()).collect(),
            example_note,
            undone_at: None,
        };
        if !self
            .writes
            .save_identification(&identification, &corrections, &examples)
            .await?
        {
            return Err(IdentifyClusterError::Conflict(
                "a segment was corrected while naming the cluster; reread and retry".into(),
            ));
        }
        Ok(identification)
    }
    pub async fn undo_identification(
        &self,
        id: &str,
    ) -> Result<UndoIdentificationReport, UndoIdentificationError> {
        let mut identification = self
            .queries
            .identification(id)
            .await?
            .ok_or_else(|| UndoIdentificationError::NotFound(id.into()))?;
        if identification.undone_at.is_some() {
            return Err(UndoIdentificationError::Conflict(
                "this identification was already undone".into(),
            ));
        }
        let mut undo = vec![];
        let mut left = vec![];
        let mut removed = identification.example_ids.clone();
        for item in &identification.segments {
            let history = self.queries.corrections(&item.segment_id).await?;
            let mut stack = super::active_decisions(&history);
            // Only a line whose current decision is still this name reverts; later edits stay.
            if stack.last().map(|c| &c.id) != Some(&item.correction_id) {
                left.push(item.segment_id.clone());
                continue;
            }
            stack.pop();
            let prior = stack.last();
            let c = Correction {
                id: new_id(),
                segment_id: item.segment_id.clone(),
                profile_id: prior.and_then(|c| c.profile_id.clone()),
                replacement_text: prior.and_then(|c| c.replacement_text.clone()),
                action: "undo".into(),
                created_at: now(),
            };
            // Examples of this person later re-admitted from these lines go with the name too.
            let mut after = history.clone();
            after.push(c.clone());
            let effect = self
                .learning_effect(
                    &item.segment_id,
                    &after,
                    None,
                    Some(&identification.profile_id),
                )
                .await?;
            for example in effect.remove {
                if !removed.contains(&example) {
                    removed.push(example);
                }
            }
            undo.push((c, history.len()));
        }
        identification.undone_at = Some(now());
        if !self
            .writes
            .undo_identification(&identification, &undo, &removed)
            .await?
        {
            return Err(UndoIdentificationError::Conflict(
                "corrections changed while undoing; reread and retry".into(),
            ));
        }
        Ok(UndoIdentificationReport {
            identification,
            reverted_segments: undo.len(),
            left_in_place: left,
            removed_examples: removed.len(),
        })
    }
    pub async fn identifications(
        &self,
        recording: Option<&str>,
    ) -> Result<Vec<ClusterIdentification>, ListIdentificationsError> {
        Ok(self.queries.identifications(recording).await?)
    }

    /// Embeds up to a few clean windows of a confirmed speaker as reversible examples.
    /// Failure to embed never blocks the user's decision; it is reported in the note.
    pub(super) async fn admit_cluster_examples(
        &self,
        profile_id: &str,
        detail: &RecordingDetail,
        track: &AudioTrack,
        windows: Vec<(i64, i64)>,
        origin: ExampleOrigin,
        removing: &[String],
    ) -> (Vec<VoiceExample>, Option<String>) {
        let mut usable: Vec<_> = windows
            .into_iter()
            .map(|(a, b)| (a, b.min(a + *EXAMPLE_SPEECH_MS.end())))
            .filter(|(a, b)| EXAMPLE_SPEECH_MS.contains(&(b - a)))
            .collect();
        if usable.is_empty() {
            return (
                vec![],
                Some("no clean single turn of at least three seconds to admit".into()),
            );
        }
        usable.sort_by(|a, b| (b.1 - b.0).cmp(&(a.1 - a.0)).then(a.0.cmp(&b.0)));
        // Examples the same decision removes no longer count against room or overlap.
        let existing: Vec<VoiceExample> = match self.queries.examples(Some(profile_id)).await {
            Ok(examples) => examples
                .into_iter()
                .filter(|e| !removing.contains(&e.id))
                .collect(),
            Err(error) => return (vec![], Some(error.to_string())),
        };
        let admitted_here = existing
            .iter()
            .filter(|e| e.recording_id == detail.recording.id && e.origin.is_some())
            .count();
        let room = EXAMPLES_PER_PERSON_PER_RECORDING
            .saturating_sub(admitted_here)
            .min(EXAMPLES_PER_IDENTIFICATION);
        if room == 0 {
            return (
                vec![],
                Some(
                    "this person already has the maximum admitted examples for this recording"
                        .into(),
                ),
            );
        }
        let model = match self.models.selection().await {
            Ok(selection) => selection.embedding,
            Err(error) => return (vec![], Some(error.to_string())),
        };
        let mut examples: Vec<VoiceExample> = vec![];
        for (start_ms, end_ms) in usable {
            if examples.len() >= room {
                break;
            }
            if examples
                .iter()
                .chain(existing.iter().filter(|e| e.track_id == track.id))
                .any(|e| e.start_ms < end_ms && e.end_ms > start_ms)
            {
                continue;
            }
            let output = match self
                .engine
                .embed(&EmbeddingRequest {
                    track: track.clone(),
                    start_ms,
                    end_ms,
                    more_ranges: vec![],
                    model: model.clone(),
                })
                .await
            {
                Ok(output) if recognition::cosine(&output.vector, &output.vector).is_some() => {
                    output
                }
                Ok(_) => continue,
                Err(error) => {
                    return (examples, Some(format!("embedding unavailable: {error}")));
                }
            };
            examples.push(VoiceExample {
                id: stable_id(
                    "voice-example",
                    &format!(
                        "{profile_id}:{}:{}:{start_ms}:{end_ms}:{}:{}",
                        detail.recording.id, track.id, output.model, output.revision
                    ),
                ),
                profile_id: profile_id.into(),
                recording_id: detail.recording.id.clone(),
                track_id: track.id.clone(),
                start_ms,
                end_ms,
                model: output.model,
                revision: output.revision,
                capture_domain: format!("{:?}", track.kind).to_lowercase(),
                vector: output.vector,
                verified_at: now(),
                origin: Some(origin.clone()),
            });
        }
        let note = examples
            .is_empty()
            .then(|| "no example could be embedded from the clean speech".to_owned());
        (examples, note)
    }
}
/// Source-local ranges of segments on `track_id` whose current speaker is someone other than
/// `profile_id` or Unknown: the user already said that speech is not this person.
fn contradicted_ranges(
    detail: &RecordingDetail,
    track_id: &str,
    offset: i64,
    profile_id: &str,
) -> Vec<(i64, i64)> {
    use parakeet_domain::corrections::{SpeakerOverride, resolve};
    detail
        .segments
        .iter()
        .filter(|s| s.track_id.as_deref() == Some(track_id))
        .filter(|s| match resolve(&s.id, &detail.corrections).speaker {
            Some(SpeakerOverride::Profile(p)) => p != profile_id,
            Some(SpeakerOverride::Unknown) => true,
            None => false,
        })
        .filter_map(|s| local_range(s, offset))
        .collect()
}
/// Clean windows cut to what the user heard, minus speech they assigned to someone else.
pub(crate) fn heard_windows(
    windows: Vec<(i64, i64)>,
    heard: &[(i64, i64)],
    contradicted: &[(i64, i64)],
) -> Vec<(i64, i64)> {
    let overlaps_others = |&(a, b): &(i64, i64)| contradicted.iter().any(|&(x, y)| x < b && y > a);
    let heard: Vec<(i64, i64)> = if heard.is_empty() {
        windows
            .iter()
            .filter(|w| !overlaps_others(w))
            .max_by(|a, b| (a.1 - a.0).cmp(&(b.1 - b.0)).then(b.0.cmp(&a.0)))
            .map(|&(a, b)| (a, b.min(a + *EXAMPLE_SPEECH_MS.end())))
            .into_iter()
            .collect()
    } else {
        heard.to_vec()
    };
    windows
        .into_iter()
        .flat_map(|(a, b)| {
            heard
                .iter()
                .map(move |&(x, y)| (a.max(x), b.min(y)))
                .filter(|(a, b)| b > a)
        })
        .filter(|w| !overlaps_others(w))
        .collect()
}
pub(super) fn segment_local_range(
    segment: &TranscriptSegment,
    detail: &RecordingDetail,
) -> Option<(i64, i64)> {
    let offset = detail
        .tracks
        .iter()
        .find(|t| Some(&t.id) == segment.track_id.as_ref())?
        .meeting_offset_ms()?;
    local_range(segment, offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_heard_speech_not_assigned_to_others_is_admitted() {
        let windows = vec![(0, 8_000), (10_000, 30_000), (40_000, 45_000)];
        // Nothing explicitly heard: only the longest turn, the clip offered first.
        assert_eq!(
            heard_windows(windows.clone(), &[], &[]),
            vec![(10_000, 30_000)]
        );
        // The longest turn holds a line assigned to someone else: the next clean one is used.
        assert_eq!(
            heard_windows(windows.clone(), &[], &[(12_000, 14_000)]),
            vec![(0, 8_000)]
        );
        // Two clips heard; a turn the user assigned to someone else is excluded.
        assert_eq!(
            heard_windows(
                windows,
                &[(0, 8_000), (40_000, 45_000)],
                &[(41_000, 43_000)]
            ),
            vec![(0, 8_000)]
        );
    }
}
