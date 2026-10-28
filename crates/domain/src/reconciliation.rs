use crate::{AudioTrack, SpeakerInterval, TrackKind, TranscriptSegment};
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
                    if similar_text(&words, &normalize_words(&other.text)) {
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

/// Speech heard on both sources is usually the same words, not necessarily identical ones:
/// echo and separate recognition runs drop, add or alter a few words.
fn similar_text(a: &[String], b: &[String]) -> bool {
    if a.len() < 3 || b.len() < 3 {
        return false;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut remaining = long.to_vec();
    let mut shared = 0;
    for word in short {
        if let Some(position) = remaining.iter().position(|w| w == word) {
            remaining.swap_remove(position);
            shared += 1;
        }
    }
    shared as f64 / short.len() as f64 >= 0.7
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
/// Where the audio came from: a source category, never a person.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceAttribution {
    pub segment_id: String,
    /// `me` (microphone), `them` (system audio) or `unknown`.
    pub category: String,
    /// Why the category is `unknown`, or what supports `me`/`them`.
    pub reasons: Vec<String>,
}
/// A stretch of a source in which most of the signal is explained by the other source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BleedSpan {
    pub track_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
}
/// Me / Them / Unknown per segment. Microphone speech is `me` only while nothing undermines
/// it: bleed from the system source, a duplicate of system speech, another voice on the
/// microphone, overlapping or ambiguous speakers, or speech the diarizer did not see.
pub fn attribute_sources(
    segments: &[TranscriptSegment],
    attributions: &[SegmentAttribution],
    intervals: &[SpeakerInterval],
    tracks: &[AudioTrack],
    bleed: &[BleedSpan],
) -> Vec<SourceAttribution> {
    // The dominant voice on a microphone is the best candidate for its owner.
    let mut speaking = BTreeMap::<(&str, &str), i64>::new();
    for (segment, attribution) in segments.iter().zip(attributions) {
        if let (Some(track), [speaker], "anonymous_model_attribution") = (
            segment.track_id.as_deref(),
            attribution.anonymous_speakers.as_slice(),
            attribution.state.as_str(),
        ) {
            *speaking.entry((track, speaker)).or_default() +=
                segment.end_ms.unwrap_or(0) - segment.start_ms.unwrap_or(0);
        }
    }
    let dominant = |track: &str| -> Option<&str> {
        speaking
            .iter()
            .filter(|((t, _), _)| *t == track)
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.1.cmp(a.0.1)))
            .map(|((_, speaker), _)| *speaker)
    };
    segments
        .iter()
        .zip(attributions)
        .map(|(segment, attribution)| {
            let mut result = SourceAttribution {
                segment_id: segment.id.clone(),
                category: "unknown".into(),
                reasons: vec![],
            };
            let Some(track) = segment
                .track_id
                .as_ref()
                .and_then(|id| tracks.iter().find(|t| &t.id == id))
            else {
                result.reasons.push("unresolved_source".into());
                return result;
            };
            if segment.start_ms.is_none() || track.meeting_offset_ms().is_none() {
                result.reasons.push("unresolved_timing".into());
                return result;
            }
            match track.kind {
                TrackKind::System => {
                    result.category = "them".into();
                    result.reasons.push("system_audio".into());
                }
                TrackKind::Microphone => {
                    let diarized = intervals
                        .iter()
                        .any(|i| i.track_id.as_deref() == Some(track.id.as_str()));
                    let mut doubts = vec![];
                    if attribution.possible_cross_source_duplicate {
                        doubts.push("duplicate_of_system_audio");
                    }
                    if is_bleed_dominated(segment, &track.id, bleed) {
                        doubts.push("system_audio_bleed");
                    }
                    if segment.evidence["uncertain_speech"] == true {
                        doubts.push("uncertain_speech");
                    }
                    if diarized {
                        match (
                            attribution.state.as_str(),
                            attribution.anonymous_speakers.as_slice(),
                        ) {
                            ("anonymous_model_attribution", [speaker])
                                if !attribution.overlapping_speech =>
                            {
                                if dominant(&track.id) != Some(speaker.as_str()) {
                                    doubts.push("another_voice_on_microphone");
                                }
                            }
                            ("unknown", _) => doubts.push("no_speech_detected_by_diarization"),
                            _ => doubts.push("overlapping_or_ambiguous_speakers"),
                        }
                    }
                    if doubts.is_empty() {
                        result.category = "me".into();
                        result.reasons.push(if diarized {
                            "dominant_microphone_voice".into()
                        } else {
                            "microphone_without_diarization".into()
                        });
                    } else {
                        result.reasons = doubts.into_iter().map(str::to_owned).collect();
                    }
                }
                // Files and mixed playback carry no source-side evidence of who is speaking.
                TrackKind::Playback | TrackKind::File => {
                    result.reasons.push("not_a_capture_source".into())
                }
            }
            result
        })
        .collect()
}
fn is_bleed_dominated(segment: &TranscriptSegment, track_id: &str, bleed: &[BleedSpan]) -> bool {
    let (Some(start), Some(end)) = (segment.start_ms, segment.end_ms) else {
        return false;
    };
    if end <= start {
        return false;
    }
    let covered: i64 = bleed
        .iter()
        .filter(|b| b.track_id == track_id)
        .map(|b| (end.min(b.end_ms) - start.max(b.start_ms)).max(0))
        .sum();
    covered * 2 >= end - start
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
    fn track(id: &str, kind: TrackKind) -> AudioTrack {
        AudioTrack {
            id: id.into(),
            recording_id: "r".into(),
            kind,
            path: "p".into(),
            start_offset_ms: Some(0),
            sample_rate: Some(16000),
            duration_ms: Some(100_000),
            available: true,
            size_bytes: None,
            modified_unix_ms: None,
            provenance: Value::Null,
        }
    }
    fn spoken(id: &str, track: &str, start: i64, end: i64, text: &str) -> TranscriptSegment {
        let mut s = seg(Some(track));
        s.id = id.into();
        s.start_ms = Some(start);
        s.end_ms = Some(end);
        s.text = text.into();
        s
    }
    fn categories(
        segments: &[TranscriptSegment],
        intervals: &[SpeakerInterval],
        bleed: &[BleedSpan],
    ) -> Vec<(String, Vec<String>)> {
        let tracks = [
            track("mic", TrackKind::Microphone),
            track("sys", TrackKind::System),
        ];
        let attributions = reconcile(segments, intervals);
        attribute_sources(segments, &attributions, intervals, &tracks, bleed)
            .into_iter()
            .map(|a| (a.category, a.reasons))
            .collect()
    }
    #[test]
    fn clean_single_voice_mic_is_me_and_system_is_them() {
        let segments = [
            spoken(
                "a",
                "mic",
                0,
                4000,
                "I think we should ship the release on friday",
            ),
            spoken("b", "sys", 5000, 9000, "that works for me thanks"),
        ];
        let intervals = [
            interval(0, 4000, "m1", "mic"),
            interval(5000, 9000, "s1", "sys"),
        ];
        let result = categories(&segments, &intervals, &[]);
        assert_eq!(result[0].0, "me");
        assert_eq!(result[1].0, "them");
    }
    #[test]
    fn leaked_far_end_speech_is_unknown_never_me() {
        let segments = [
            spoken(
                "echo",
                "mic",
                1000,
                5000,
                "we should ship the release on friday morning",
            ),
            spoken(
                "far",
                "sys",
                900,
                5000,
                "we should ship the release on friday",
            ),
        ];
        let intervals = [
            interval(1000, 5000, "m1", "mic"),
            interval(900, 5000, "s1", "sys"),
        ];
        let result = categories(&segments, &intervals, &[]);
        assert_eq!(result[0].0, "unknown");
        assert!(
            result[0]
                .1
                .contains(&"duplicate_of_system_audio".to_owned())
        );
        assert_eq!(result[1].0, "them");
        // Bleed evidence alone, with different recognized words, also withholds "Me".
        let lone = [spoken("echo", "mic", 1000, 5000, "garbled words entirely")];
        let bleed = [BleedSpan {
            track_id: "mic".into(),
            start_ms: 0,
            end_ms: 6000,
        }];
        assert_eq!(
            categories(&lone, &[interval(1000, 5000, "m1", "mic")], &bleed)[0].0,
            "unknown"
        );
    }
    #[test]
    fn a_second_voice_in_the_room_is_not_me() {
        let segments = [
            spoken("owner1", "mic", 0, 5000, "my own words go here today"),
            spoken("owner2", "mic", 6000, 11000, "more of my words go here"),
            spoken("guest", "mic", 12000, 15000, "hello from the next desk"),
        ];
        let intervals = [
            interval(0, 5000, "m1", "mic"),
            interval(6000, 11000, "m1", "mic"),
            interval(12000, 15000, "m2", "mic"),
        ];
        let result = categories(&segments, &intervals, &[]);
        assert_eq!(
            result.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
            ["me", "me", "unknown"]
        );
        assert!(
            result[2]
                .1
                .contains(&"another_voice_on_microphone".to_owned())
        );
    }
    #[test]
    fn overlap_missing_timing_and_unseen_speech_are_unknown() {
        let segments = [spoken("a", "mic", 0, 4000, "words in the overlap here")];
        let overlap = [
            interval(0, 4000, "m1", "mic"),
            interval(1000, 3000, "m2", "mic"),
        ];
        assert_eq!(categories(&segments, &overlap, &[])[0].0, "unknown");
        let elsewhere = [interval(60000, 64000, "m1", "mic")];
        assert_eq!(categories(&segments, &elsewhere, &[])[0].0, "unknown");
        let mut untimed = segments[0].clone();
        untimed.start_ms = None;
        assert_eq!(categories(&[untimed], &[], &[])[0].0, "unknown");
        // Without any diarization the source alone supports "Me" only for a clean mic.
        assert_eq!(categories(&segments, &[], &[])[0].0, "me");
    }
    #[test]
    fn near_duplicate_text_is_detected_but_distinct_speech_is_not() {
        let a = normalize_words("we should ship it on friday morning");
        let b = normalize_words("we should ship on friday");
        assert!(similar_text(&a, &b));
        assert!(!similar_text(
            &a,
            &normalize_words("completely different sentence about lunch")
        ));
        assert!(!similar_text(
            &normalize_words("yes ok"),
            &normalize_words("yes ok")
        ));
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
