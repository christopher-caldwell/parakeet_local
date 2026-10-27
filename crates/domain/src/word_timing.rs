use crate::{
    AudioTrack, SpeakerInterval, TranscriptSegment,
    reconciliation::{SegmentAttribution, reconcile},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecognizedWord {
    pub text: String,
    pub local_start_ms: Option<i64>,
    pub local_end_ms: Option<i64>,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub timing_quality: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordAttribution {
    pub segment_id: String,
    pub ordinal: usize,
    pub word: RecognizedWord,
    pub attribution: SegmentAttribution,
}
/// Model-estimated words remain separate from segment evidence and corrections.
/// A missing/invalid timestamp stays unknown. No named identity is produced.
pub fn reconcile_words(
    segments: &[TranscriptSegment],
    intervals: &[SpeakerInterval],
    parent_attributions: &[SegmentAttribution],
    tracks: &[AudioTrack],
) -> Vec<WordAttribution> {
    let mut result = vec![];
    for segment in segments {
        if segment.evidence["word_timing_version"] != 1 {
            continue;
        }
        let Ok(words) =
            serde_json::from_value::<Vec<RecognizedWord>>(segment.evidence["words"].clone())
        else {
            continue;
        };
        let parent = parent_attributions
            .iter()
            .find(|a| a.segment_id == segment.id);
        if words
            .iter()
            .flat_map(|w| w.text.split_whitespace())
            .ne(segment.text.split_whitespace())
        {
            continue;
        }
        let track = tracks
            .iter()
            .find(|t| Some(&t.id) == segment.track_id.as_ref());
        for (ordinal, mut word) in words.into_iter().enumerate() {
            let local = match (word.local_start_ms, word.local_end_ms) {
                (Some(a), Some(b))
                    if a >= 0
                        && b > a
                        && track.is_some_and(|t| t.duration_ms.is_some_and(|d| b <= d))
                        && word.timing_quality == "experimental_whisper_token_estimate" =>
                {
                    Some((a, b))
                }
                _ => None,
            };
            let common = local.and_then(|(a, b)| {
                Some((
                    track?.meeting_offset_ms()?.checked_add(a)?,
                    track?.meeting_offset_ms()?.checked_add(b)?,
                ))
            });
            word.start_ms = common.map(|x| x.0);
            word.end_ms = common.map(|x| x.1);
            if local.is_none() {
                word.local_start_ms = None;
                word.local_end_ms = None;
                word.timing_quality = "untimed".into();
            }
            let mut proxy = TranscriptSegment {
                id: String::new(),
                recording_id: segment.recording_id.clone(),
                run_id: segment.run_id.clone(),
                track_id: segment.track_id.clone(),
                ordinal,
                start_ms: None,
                end_ms: None,
                text: String::new(),
                speaker_claim: None,
                evidence: serde_json::Value::Null,
            };
            proxy.id = format!("{}:estimated-word:{ordinal}", segment.id);
            proxy.text = word.text.clone();
            proxy.start_ms = word.start_ms;
            proxy.end_ms = word.end_ms;
            let mut attribution = reconcile(std::slice::from_ref(&proxy), intervals).remove(0);
            if let Some(parent) = parent {
                attribution.overlapping_source_audio = parent.overlapping_source_audio;
                attribution.possible_cross_source_duplicate =
                    parent.possible_cross_source_duplicate;
            }
            if segment.evidence["uncertain_speech"] == true {
                attribution.state = "unknown".into();
                attribution.anonymous_speakers.clear();
            }
            result.push(WordAttribution {
                segment_id: segment.id.clone(),
                ordinal,
                word,
                attribution,
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn turn_crossing_segment_can_have_separate_anonymous_words_without_naming() {
        let word = |text: &str, a, b| RecognizedWord {
            text: text.into(),
            start_ms: a,
            end_ms: b,
            local_start_ms: a,
            local_end_ms: b,
            timing_quality: "experimental_whisper_token_estimate".into(),
        };
        let segment = TranscriptSegment {
            id: "s".into(),
            recording_id: "r".into(),
            run_id: "run".into(),
            track_id: Some("mic".into()),
            ordinal: 0,
            start_ms: Some(0),
            end_ms: Some(2000),
            text: "one two untimed".into(),
            speaker_claim: None,
            evidence: json!({"word_timing_version":1,"words":[word("one",Some(100),Some(800)),word("two",Some(1200),Some(1800)),word("untimed",None,None)]}),
        };
        let intervals = vec![
            SpeakerInterval {
                start_ms: 0,
                end_ms: 1000,
                speaker: "a".into(),
                track_id: Some("mic".into()),
            },
            SpeakerInterval {
                start_ms: 1000,
                end_ms: 2000,
                speaker: "b".into(),
                track_id: Some("mic".into()),
            },
        ];
        let track = AudioTrack {
            id: "mic".into(),
            recording_id: "r".into(),
            kind: crate::TrackKind::Microphone,
            path: "fixture".into(),
            start_offset_ms: Some(0),
            sample_rate: Some(16000),
            duration_ms: Some(2000),
            available: true,
            size_bytes: None,
            modified_unix_ms: None,
            provenance: json!({}),
        };
        let tracks = [track];
        let parents = reconcile(std::slice::from_ref(&segment), &intervals);
        assert_eq!(parents[0].state, "ambiguous");
        let words = reconcile_words(
            std::slice::from_ref(&segment),
            &intervals,
            &parents,
            &tracks,
        );
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].attribution.anonymous_speakers, vec!["a"]);
        assert_eq!(words[1].attribution.anonymous_speakers, vec!["b"]);
        assert_eq!(words[2].attribution.state, "unknown");
        let mut unaligned = tracks.clone();
        unaligned[0].start_offset_ms = None;
        assert!(
            reconcile_words(
                std::slice::from_ref(&segment),
                &intervals,
                &parents,
                &unaligned
            )
            .iter()
            .all(|w| w.word.start_ms.is_none() && w.attribution.state == "unknown")
        );
        let mut forged = segment.clone();
        forged.evidence["words"][0]["start_ms"] = json!(99_999);
        assert_eq!(
            reconcile_words(&[forged], &intervals, &parents, &tracks)[0]
                .word
                .start_ms,
            Some(100)
        );
        let mut out_of_range = tracks.clone();
        out_of_range[0].duration_ms = Some(100);
        assert!(
            reconcile_words(
                std::slice::from_ref(&segment),
                &intervals,
                &parents,
                &out_of_range
            )
            .iter()
            .all(|w| w.attribution.state == "unknown")
        );
        let wrong_source = intervals
            .iter()
            .cloned()
            .map(|mut i| {
                i.track_id = Some("system".into());
                i
            })
            .collect::<Vec<_>>();
        assert!(
            reconcile_words(
                std::slice::from_ref(&segment),
                &wrong_source,
                &parents,
                &tracks
            )
            .iter()
            .all(|w| w.attribution.state == "unknown")
        );
        let mut uncertain = segment;
        uncertain.evidence["uncertain_speech"] = json!(true);
        assert!(
            reconcile_words(&[uncertain], &intervals, &parents, &tracks)
                .iter()
                .all(|w| w.attribution.state == "unknown")
        );
    }
}
