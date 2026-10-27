use crate::{SpeakerInterval, TranscriptSegment};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentAttribution {
    pub segment_id: String,
    pub state: String,
    pub anonymous_speakers: Vec<String>,
    pub speech_coverage: f64,
    pub overlapping_speech: bool,
    pub overlapping_source_audio: bool,
    pub possible_cross_source_duplicate: bool,
}
/// Only compare intervals on the same authoritative source and common meeting timeline.
/// A segment straddling turns remains ambiguous because segment timing cannot locate individual words.
pub fn reconcile(
    segments: &[TranscriptSegment],
    intervals: &[SpeakerInterval],
) -> Vec<SegmentAttribution> {
    segments
        .iter()
        .map(|s| {
            let mut result = SegmentAttribution {
                segment_id: s.id.clone(),
                state: "unknown".into(),
                anonymous_speakers: vec![],
                speech_coverage: 0.,
                overlapping_speech: false,
                overlapping_source_audio: false,
                possible_cross_source_duplicate: false,
            };
            let (Some(track), Some(start), Some(end)) = (&s.track_id, s.start_ms, s.end_ms) else {
                return result;
            };
            if end <= start {
                return result;
            }
            let words = normalize_words(&s.text);
            for other in segments {
                if other.id == s.id || other.track_id.is_none() || other.track_id == s.track_id {
                    continue;
                }
                if let (Some(a), Some(b)) = (other.start_ms, other.end_ms)
                    && a < end
                    && b > start
                {
                    result.overlapping_source_audio = true;
                    if words.len() >= 4 && words == normalize_words(&other.text) {
                        result.possible_cross_source_duplicate = true;
                    }
                }
            }
            let candidates: Vec<_> = intervals
                .iter()
                .filter(|i| {
                    i.track_id.as_ref() == Some(track) && i.start_ms < end && i.end_ms > start
                })
                .collect();
            let mut by_speaker = BTreeMap::<String, Vec<(i64, i64)>>::new();
            for i in &candidates {
                by_speaker
                    .entry(i.speaker.clone())
                    .or_default()
                    .push((start.max(i.start_ms), end.min(i.end_ms)));
            }
            let mut covered = vec![];
            for times in by_speaker.values() {
                covered.extend_from_slice(times);
            }
            covered.sort();
            let mut union = 0;
            let mut last = None;
            for (a, b) in covered {
                if let Some((begin, finish)) = last {
                    if a <= finish {
                        last = Some((begin, finish.max(b)));
                    } else {
                        union += finish - begin;
                        last = Some((a, b));
                    }
                } else {
                    last = Some((a, b));
                }
            }
            if let Some((a, b)) = last {
                union += b - a;
            }
            result.speech_coverage = union as f64 / (end - start) as f64;
            result.anonymous_speakers = by_speaker.keys().cloned().collect();
            result.overlapping_speech = candidates.iter().enumerate().any(|(n, a)| {
                candidates.iter().skip(n + 1).any(|b| {
                    a.speaker != b.speaker
                        && a.start_ms.max(b.start_ms).max(start) < a.end_ms.min(b.end_ms).min(end)
                })
            });
            if by_speaker.len() == 1 && result.speech_coverage >= 0.8 {
                result.state = "anonymous_model_attribution".into();
            } else if !by_speaker.is_empty() {
                result.state = "ambiguous".into();
            }
            result
        })
        .collect()
}

fn normalize_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|s| !s.is_empty())
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    fn seg(track: Option<&str>) -> TranscriptSegment {
        TranscriptSegment {
            id: "s".into(),
            recording_id: "r".into(),
            run_id: "run".into(),
            track_id: track.map(str::to_owned),
            ordinal: 0,
            start_ms: Some(0),
            end_ms: Some(1000),
            text: "words".into(),
            speaker_claim: None,
            evidence: Value::Null,
        }
    }
    fn interval(a: i64, b: i64, n: &str, t: &str) -> SpeakerInterval {
        SpeakerInterval {
            start_ms: a,
            end_ms: b,
            speaker: n.into(),
            track_id: Some(t.into()),
        }
    }
    #[test]
    fn source_is_not_identity_and_other_source_cannot_attrib() {
        let r = reconcile(
            &[seg(Some("mic"))],
            &[interval(0, 1000, "remote", "system")],
        );
        assert_eq!(r[0].state, "unknown");
        assert_eq!(
            reconcile(&[seg(None)], &[interval(0, 1000, "x", "mic")])[0].state,
            "unknown"
        );
    }
    #[test]
    fn overlap_and_turn_boundaries_stay_ambiguous() {
        let s = seg(Some("mic"));
        let intervals = vec![
            interval(0, 1000, "a", "mic"),
            interval(500, 900, "b", "mic"),
        ];
        let r = reconcile(std::slice::from_ref(&s), &intervals);
        assert_eq!(r[0].state, "ambiguous");
        assert!(r[0].overlapping_speech);
        assert_eq!(
            reconcile(
                &[s],
                &[
                    interval(0, 500, "a", "mic"),
                    interval(500, 1000, "b", "mic")
                ]
            )[0]
            .state,
            "ambiguous"
        );
    }
    #[test]
    fn anonymous_evidence_does_not_create_named_speaker() {
        let r = reconcile(
            &[seg(Some("mic"))],
            &[interval(0, 1000, "cluster-1", "mic")],
        );
        assert_eq!(r[0].state, "anonymous_model_attribution");
        assert_eq!(r[0].anonymous_speakers, vec!["cluster-1"]);
    }
}
