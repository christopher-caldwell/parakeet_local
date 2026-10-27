use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AudioPreferences {
    pub microphone_uid: Option<String>,
    pub system_audio: bool,
}

pub mod reconciliation;

pub mod cluster_edits;
