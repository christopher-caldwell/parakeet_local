//! Concatenated minimum-permutation WER. Text is already chronological within
//! each anonymous speaker stream. No inter-speaker ordering or word times are
//! invented. This metric evaluates anonymous stream separation, not named identity.
use crate::{
    ValidationError,
    evaluation::{assignment, normalize_wer, word_error_count},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerText {
    pub speaker: String,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerWordMatch {
    pub reference: Option<String>,
    pub prediction: Option<String>,
    pub errors: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerWordScore {
    pub metric: String,
    pub normalization: String,
    pub reference_words: usize,
    pub prediction_words: usize,
    pub errors: usize,
    pub cpwer: Option<f64>,
    pub reference_speakers: usize,
    pub predicted_speakers: usize,
    pub mapping: Vec<SpeakerWordMatch>,
}

pub fn score_speaker_words(
    reference: &[SpeakerText],
    prediction: &[SpeakerText],
    normalization: &str,
) -> Result<SpeakerWordScore, ValidationError> {
    fn validate(streams: &[SpeakerText]) -> Result<(), ValidationError> {
        let mut ids = BTreeSet::new();
        if streams.len() > 64 {
            return Err(ValidationError(
                "cpWER supports at most 64 streams per side".into(),
            ));
        }
        for stream in streams {
            if stream.speaker.trim().is_empty() || !ids.insert(&stream.speaker) {
                return Err(ValidationError(
                    "speaker streams need distinct nonempty IDs".into(),
                ));
            }
        }
        Ok(())
    }
    validate(reference)?;
    validate(prediction)?;
    // Keep mapping deterministic even when several assignments have equal cost.
    let mut reference: Vec<_> = reference.iter().collect();
    let mut prediction: Vec<_> = prediction.iter().collect();
    reference.sort_by_key(|s| &s.speaker);
    prediction.sort_by_key(|s| &s.speaker);
    let normalize = |streams: &[&SpeakerText]| -> Result<Vec<String>, ValidationError> {
        streams
            .iter()
            .map(|s| normalize_wer(&s.text, normalization))
            .collect()
    };
    let rtext = normalize(&reference)?;
    let htext = normalize(&prediction)?;
    let r: Vec<Vec<_>> = rtext
        .iter()
        .map(|s| s.split_whitespace().collect())
        .collect();
    let h: Vec<Vec<_>> = htext
        .iter()
        .map(|s| s.split_whitespace().collect())
        .collect();
    let reference_words: usize = r.iter().map(Vec::len).sum();
    let prediction_words: usize = h.iter().map(Vec::len).sum();
    if reference_words > 20_000
        || prediction_words > 20_000
        || reference_words.saturating_mul(prediction_words) > 25_000_000
    {
        return Err(ValidationError(
            "cpWER input exceeds the bounded scoring budget; score shorter independent recordings"
                .into(),
        ));
    }
    let n = r.len().max(h.len());
    let mut costs = vec![vec![0usize; n]; n];
    for (i, row) in costs.iter_mut().enumerate() {
        for (j, cost) in row.iter_mut().enumerate() {
            *cost = word_error_count(
                r.get(i).map(Vec::as_slice).unwrap_or(&[]),
                h.get(j).map(Vec::as_slice).unwrap_or(&[]),
            );
        }
    }
    // Padded dummy streams have real deletion/insertion costs. Zero padding the
    // matching weights would incorrectly make missed/extra speech free.
    let weights: Vec<Vec<_>> = costs
        .iter()
        .map(|row| row.iter().map(|&v| -(v as f64)).collect())
        .collect();
    let mapping = assignment(&weights, n)
        .into_iter()
        .enumerate()
        .map(|(i, j)| {
            let j = j.expect("square assignment is complete");
            SpeakerWordMatch {
                reference: reference.get(i).map(|s| s.speaker.clone()),
                prediction: prediction.get(j).map(|s| s.speaker.clone()),
                errors: costs[i][j],
            }
        })
        .collect::<Vec<_>>();
    let errors = mapping.iter().map(|m| m.errors).sum::<usize>();
    Ok(SpeakerWordScore {
        metric: "concatenated_minimum_permutation_wer".into(),
        normalization: normalization.into(),
        reference_words,
        prediction_words,
        errors,
        cpwer: (reference_words > 0).then(|| errors as f64 / reference_words as f64),
        reference_speakers: reference.len(),
        predicted_speakers: prediction.len(),
        mapping,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stream(id: &str, text: &str) -> SpeakerText {
        SpeakerText {
            speaker: id.into(),
            text: text.into(),
        }
    }
    #[test]
    fn anonymous_permutation_is_free_but_turn_contamination_is_not() {
        let r = [stream("a", "one three"), stream("b", "two four")];
        assert_eq!(
            score_speaker_words(
                &r,
                &[stream("x", "two four"), stream("y", "one three")],
                "exact"
            )
            .unwrap()
            .cpwer,
            Some(0.)
        );
        // Ordinary chronological text WER can be zero despite these mixed voices.
        let mixed = score_speaker_words(
            &r,
            &[stream("x", "one two"), stream("y", "three four")],
            "exact",
        )
        .unwrap();
        assert_eq!(mixed.errors, 2);
        assert_eq!(mixed.cpwer, Some(0.5));
    }
    #[test]
    fn missing_extra_and_empty_streams_are_not_free() {
        let r = [stream("a", "a b"), stream("b", "c d")];
        assert_eq!(
            score_speaker_words(&r, &[stream("x", "a b")], "exact")
                .unwrap()
                .errors,
            2
        );
        let extra = score_speaker_words(
            &r,
            &[stream("x", "a b"), stream("y", "c d"), stream("z", "e f")],
            "exact",
        )
        .unwrap();
        assert_eq!(extra.errors, 2);
        assert_eq!(
            extra
                .mapping
                .iter()
                .filter(|m| m.reference.is_none())
                .count(),
            1
        );
        assert_eq!(
            score_speaker_words(&[], &[stream("x", "extra words")], "exact")
                .unwrap()
                .cpwer,
            None
        );
        assert_eq!(
            score_speaker_words(&[], &[stream("x", "extra words")], "exact")
                .unwrap()
                .errors,
            2
        );
        assert_eq!(
            score_speaker_words(&r, &[], "exact").unwrap().cpwer,
            Some(1.)
        );
        assert!(
            score_speaker_words(&[], &[], "exact")
                .unwrap()
                .mapping
                .is_empty()
        );
    }
    #[test]
    fn declared_normalization_validation_and_resource_limits() {
        let r = [stream("a", "HELLO, world!")];
        let h = [stream("x", "hello world")];
        assert_eq!(
            score_speaker_words(&r, &h, "english_case_punctuation")
                .unwrap()
                .cpwer,
            Some(0.)
        );
        assert!(score_speaker_words(&r, &h, "invented").is_err());
        assert!(score_speaker_words(&[stream("a", "x"), stream("a", "y")], &h, "exact").is_err());
        assert!(score_speaker_words(&[stream("", "x")], &h, "exact").is_err());
        assert!(
            score_speaker_words(
                &[stream("a", &"word ".repeat(6000))],
                &[stream("x", &"word ".repeat(6000))],
                "exact"
            )
            .is_err()
        );
    }
}
