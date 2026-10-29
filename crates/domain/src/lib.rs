use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub mod corrections;
pub mod evaluation;
pub mod gestures;
pub mod recognition;
pub mod speaker_wer;
pub mod word_timing;

pub fn stable_id(namespace: &str, key: &str) -> String {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("parakeet:{namespace}:{key}").as_bytes(),
    )
    .to_string()
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ValidationError(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Recording {
    pub id: String,
    pub source_key: String,
    pub source_kind: String,
    pub source_title: String,
    pub title_override: Option<String>,
    pub created_at: String,
    pub duration_ms: Option<i64>,
    pub metadata: Value,
}
impl Recording {
    /// Capture integrity is independent of whether later transcription succeeded.
    pub fn capture_needs_attention(&self) -> bool {
        matches!(
            self.metadata["state"].as_str(),
            Some("partial_needs_inspection" | "recovered_partial")
        )
    }

    pub fn title(&self) -> &str {
        self.title_override.as_deref().unwrap_or(&self.source_title)
    }
    pub fn validate_title(title: &str) -> Result<(), ValidationError> {
        if title.trim().is_empty() || title.chars().count() > 500 || title.contains('\0') {
            return Err(ValidationError(
                "title must contain 1–500 characters and no NUL".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Microphone,
    System,
    Playback,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioTrack {
    pub id: String,
    pub recording_id: String,
    pub kind: TrackKind,
    pub path: String,
    /// None means alignment is unknown, not simultaneous with meeting start.
    pub start_offset_ms: Option<i64>,
    pub sample_rate: Option<u32>,
    pub duration_ms: Option<i64>,
    pub available: bool,
    pub size_bytes: Option<u64>,
    pub modified_unix_ms: Option<u64>,
    pub provenance: Value,
}

impl AudioTrack {
    pub fn timeline_integrity(&self) -> &'static str {
        if self.start_offset_ms.is_none() {
            return "unknown_origin";
        }
        if self
            .start_offset_ms
            .and_then(|offset| {
                offset.checked_add(self.duration_ms.unwrap_or(7_200_000).max(7_200_000))
            })
            .is_none()
        {
            return "invalid_origin";
        }
        if self.provenance["unmapped_capture_gaps"] == true {
            return "unknown_capture_gaps";
        }
        let alignment = &self.provenance["alignment"];
        let expected = alignment["timelineFrameCount"]
            .as_f64()
            .or_else(|| alignment["writtenFrameCount"].as_f64())
            .zip(alignment["sampleRate"].as_f64())
            .filter(|(_, r)| *r > 0.)
            .map(|(f, r)| f * 1000. / r);
        if let (Some(expected), Some(actual)) = (expected, self.duration_ms)
            && (expected - actual as f64).abs() > 2000.
        {
            return "inconsistent_duration";
        }
        "known_origin"
    }
    pub fn meeting_offset_ms(&self) -> Option<i64> {
        if self.timeline_integrity() == "known_origin" {
            self.start_offset_ms
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptSegment {
    pub id: String,
    pub recording_id: String,
    pub run_id: String,
    pub track_id: Option<String>,
    pub ordinal: usize,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub text: String,
    /// An imported label or anonymous model cluster is evidence, never a confirmed person.
    pub speaker_claim: Option<String>,
    pub evidence: Value,
}
impl TranscriptSegment {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match (self.start_ms, self.end_ms) {
            (Some(s), Some(e)) if s >= 0 && e >= s => Ok(()),
            (None, None) => Ok(()),
            _ => Err(ValidationError(
                "segment timing must be a nonnegative interval or entirely untimed".into(),
            )),
        }
    }
}
/// Stable audio ordering; equal timestamps use track identity and original ordinal.
/// Untimed text is retained after timed text without manufactured timestamps.
pub fn order_segments(segments: &mut [TranscriptSegment]) {
    segments.sort_by(|a, b| {
        a.start_ms
            .unwrap_or(i64::MAX)
            .cmp(&b.start_ms.unwrap_or(i64::MAX))
            .then(a.track_id.cmp(&b.track_id))
            .then(a.ordinal.cmp(&b.ordinal))
            .then(a.id.cmp(&b.id))
    });
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBundle {
    pub recording: Recording,
    pub tracks: Vec<AudioTrack>,
    pub segments: Vec<TranscriptSegment>,
    pub original: Value,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerInterval {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker: String,
    pub track_id: Option<String>,
}
impl SpeakerInterval {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.start_ms < 0 || self.end_ms <= self.start_ms || self.speaker.is_empty() {
            return Err(ValidationError(
                "speaker interval requires increasing nonnegative times and a speaker".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingRun {
    pub id: String,
    pub recording_id: String,
    pub kind: String,
    pub model: String,
    pub model_revision: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub configuration: Value,
    pub result: Value,
    pub error: Option<String>,
}
impl ProcessingRun {
    /// A diarization run that covered the whole source (not a bounded experiment).
    fn covers_whole_source(&self) -> bool {
        let request = &self.configuration["request"];
        request["start_ms"].as_i64() == Some(0) && request["duration_ms"].is_null()
    }
}
/// The diarization run that currently defines speaker attribution for each track:
/// the latest completed full-source run, or the latest human cluster edit derived
/// from it. Bounded experiments and edits of superseded runs never become current.
pub fn current_diarization_runs<'a>(
    runs: &'a [ProcessingRun],
) -> std::collections::BTreeMap<String, &'a ProcessingRun> {
    let done = |r: &&'a ProcessingRun| r.kind == "diarization" && r.status == "completed";
    let by_id: std::collections::HashMap<&'a str, &'a ProcessingRun> = runs
        .iter()
        .filter(done)
        .map(|r| (r.id.as_str(), r))
        .collect();
    let root = |mut run: &'a ProcessingRun| -> Option<String> {
        for _ in 0..runs.len() + 1 {
            match run.configuration["human_edit_source_run"].as_str() {
                None => return run.covers_whole_source().then(|| run.id.clone()),
                Some(parent) => run = by_id.get(parent).copied()?,
            }
        }
        None
    };
    let mut full = std::collections::BTreeMap::<String, &'a ProcessingRun>::new();
    for run in runs.iter().filter(done) {
        let Some(track) = run.configuration["track"]["id"].as_str() else {
            continue;
        };
        if run.configuration["human_edit_source_run"].is_null() && run.covers_whole_source() {
            full.insert(track.to_owned(), run);
        }
    }
    let mut current = full.clone();
    for run in runs.iter().filter(done) {
        let Some(track) = run.configuration["track"]["id"].as_str() else {
            continue;
        };
        if run.configuration["human_edit_source_run"].is_string()
            && full
                .get(track)
                .is_some_and(|f| root(run).as_deref() == Some(f.id.as_str()))
        {
            current.insert(track.to_owned(), run);
        }
    }
    current
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerProfile {
    pub id: String,
    pub name: String,
    pub created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceExample {
    pub id: String,
    pub profile_id: String,
    pub recording_id: String,
    pub track_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub model: String,
    pub revision: String,
    pub capture_domain: String,
    pub vector: Vec<f32>,
    pub verified_at: String,
    /// How the user's confirmation produced this example; `None` for a manually chosen interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<ExampleOrigin>,
}
/// Reversible provenance of an example admitted from a user's identification or correction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExampleOrigin {
    /// Always `user_identification`: a model prediction alone never creates an example.
    pub verified_by: String,
    /// `cluster_identification`, `suggestion_resolution` or `segment_correction`.
    pub source_kind: String,
    pub source_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Correction {
    pub id: String,
    pub segment_id: String,
    pub profile_id: Option<String>,
    pub replacement_text: Option<String>,
    pub action: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptProfile {
    pub id: String,
    pub bundle_id: String,
    pub application_name: String,
    pub enabled: bool,
    pub prompt: String,
    pub executable: String,
    pub arguments: Vec<String>,
    /// "local" and "remote" are explicit user declarations, never inferred from executable names.
    pub route: String,
    pub authorized: bool,
    pub timeout_seconds: u64,
}
impl PromptProfile {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.bundle_id.is_empty()
            || !["local", "remote"].contains(&self.route.as_str())
            || self.timeout_seconds == 0
            || self.timeout_seconds > 600
        {
            return Err(ValidationError(
                "profile needs a bundle ID, explicit local/remote route, and 1–600 second timeout"
                    .into(),
            ));
        }
        if !self.executable.starts_with('/') {
            return Err(ValidationError(
                "AI executable must be an absolute path".into(),
            ));
        }
        Ok(())
    }
    pub fn permits(&self, bundle_id: &str) -> bool {
        self.enabled && self.authorized && self.bundle_id == bundle_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn segment(id: &str, start: Option<i64>, track: &str, ordinal: usize) -> TranscriptSegment {
        TranscriptSegment {
            id: id.into(),
            recording_id: "r".into(),
            run_id: "run".into(),
            track_id: Some(track.into()),
            ordinal,
            start_ms: start,
            end_ms: start.map(|s| s + 50),
            text: "again again".into(),
            speaker_claim: None,
            evidence: Value::Null,
        }
    }
    #[test]
    fn chronological_overlap_and_untimed_are_preserved() {
        let mut s = vec![
            segment("late", Some(800), "b", 0),
            segment("untimed", None, "a", 1),
            segment("overlap", Some(10), "b", 1),
            segment("early", Some(10), "a", 0),
        ];
        order_segments(&mut s);
        assert_eq!(
            s.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
            vec!["early", "overlap", "late", "untimed"]
        );
        assert_eq!(s[0].text, "again again");
    }
    #[test]
    fn title_override_wins() {
        let r = Recording {
            id: "r".into(),
            source_key: "s".into(),
            source_kind: "youtube".into(),
            source_title: "video".into(),
            title_override: Some("mine".into()),
            created_at: "".into(),
            duration_ms: None,
            metadata: Value::Null,
        };
        assert_eq!(r.title(), "mine");
        assert!(Recording::validate_title(" ").is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioPreferences {
    pub microphone_uid: Option<String>,
    pub system_audio: bool,
    /// Meetings are transcribed after Stop unless the user turns this off. When no verified
    /// ASR model is installed the audio is still saved and the skipped transcription is reported.
    #[serde(default = "transcribe_by_default")]
    pub transcribe_meetings_after_stop: bool,
    /// Speaker detection and the echo check run after Stop unless turned off: accuracy comes
    /// first, and without the echo check microphone speech is never labeled "Me".
    #[serde(default = "transcribe_by_default")]
    pub analyze_meetings_after_stop: bool,
    /// Offer name suggestions after Stop using the saved calibrated criteria. Suggestions are
    /// never confirmed names. Off until the user enables it.
    #[serde(default)]
    pub suggest_speakers_after_stop: bool,
    #[serde(default)]
    pub matching_policy: Option<SavedMatchingPolicy>,
    /// Voice candidates the user chose to skip for good (for example people who will not come up
    /// again), so they are not offered for naming every time the recording opens.
    #[serde(default)]
    pub skipped_voices: Vec<String>,
}
/// Criteria the user adopted from a calibration report, kept with the evidence behind them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedMatchingPolicy {
    pub policy: recognition::MatchPolicy,
    pub saved_at: String,
    pub provenance: calibration::CalibrationReport,
}
/// Peak amplitude (about -50 dBFS) below which a source that delivered audio is treated as silent.
pub const SILENT_SOURCE_PEAK: f64 = 0.003;
fn transcribe_by_default() -> bool {
    true
}
impl Default for AudioPreferences {
    fn default() -> Self {
        Self {
            microphone_uid: None,
            system_audio: false,
            transcribe_meetings_after_stop: true,
            analyze_meetings_after_stop: true,
            suggest_speakers_after_stop: false,
            matching_policy: None,
            skipped_voices: vec![],
        }
    }
}
impl AudioPreferences {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self
            .microphone_uid
            .as_ref()
            .is_some_and(|uid| uid.trim().is_empty())
        {
            return Err(ValidationError("microphone UID cannot be blank".into()));
        }
        if self.analyze_meetings_after_stop && !self.transcribe_meetings_after_stop {
            return Err(ValidationError(
                "speaker analysis after Stop requires local transcription".into(),
            ));
        }
        if self.suggest_speakers_after_stop
            && (!self.analyze_meetings_after_stop || self.matching_policy.is_none())
        {
            return Err(ValidationError(
                "speaker suggestions after Stop need speaker analysis and a saved calibrated policy"
                    .into(),
            ));
        }
        if let Some(saved) = &self.matching_policy {
            saved.policy.validate()?;
        }
        Ok(())
    }
}
impl AudioPreferences {
    /// The criteria to apply after Stop, only when the user opted in.
    pub fn after_stop_policy(&self) -> Option<recognition::MatchPolicy> {
        self.suggest_speakers_after_stop
            .then(|| {
                self.matching_policy
                    .as_ref()
                    .map(|saved| saved.policy.clone())
            })
            .flatten()
    }
}

pub mod calibration;

pub mod reconciliation;

pub mod cluster_edits;

pub mod storage;

#[cfg(test)]
mod current_diarization_tests {
    use super::*;
    use serde_json::json;
    fn run(id: &str, config: Value) -> ProcessingRun {
        ProcessingRun {
            id: id.into(),
            recording_id: "r".into(),
            kind: "diarization".into(),
            model: "m".into(),
            model_revision: "1".into(),
            status: "completed".into(),
            started_at: String::new(),
            completed_at: None,
            configuration: config,
            result: json!({}),
            error: None,
        }
    }
    fn full(id: &str) -> ProcessingRun {
        run(
            id,
            json!({"track":{"id":"t"},"request":{"start_ms":0,"duration_ms":null}}),
        )
    }
    fn bounded(id: &str) -> ProcessingRun {
        run(
            id,
            json!({"track":{"id":"t"},"request":{"start_ms":5000,"duration_ms":60000}}),
        )
    }
    fn edit(id: &str, source: &str) -> ProcessingRun {
        run(
            id,
            json!({"track":{"id":"t"},"human_edit_source_run":source}),
        )
    }
    fn current(runs: &[ProcessingRun]) -> String {
        current_diarization_runs(runs)["t"].id.clone()
    }
    #[test]
    fn bounded_experiment_never_replaces_full_run() {
        assert_eq!(current(&[full("f"), bounded("b")]), "f");
        assert!(current_diarization_runs(&[bounded("b")]).is_empty());
    }
    #[test]
    fn edit_of_current_full_run_wins_over_later_bounded_run() {
        assert_eq!(current(&[full("f"), edit("e", "f"), bounded("b")]), "e");
        assert_eq!(current(&[full("f"), edit("e", "f"), edit("u", "e")]), "u");
    }
    #[test]
    fn new_full_run_supersedes_edits_of_older_run() {
        assert_eq!(current(&[full("f1"), edit("e", "f1"), full("f2")]), "f2");
    }
}
