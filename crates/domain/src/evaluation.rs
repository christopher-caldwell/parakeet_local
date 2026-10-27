use crate::{SpeakerInterval, ValidationError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiarizationScore {
    pub reference_speaker_ms: f64,
    pub missed_speaker_ms: f64,
    pub false_speaker_ms: f64,
    pub confused_speaker_ms: f64,
    pub der: Option<f64>,
    pub jer: Option<f64>,
    pub reference_speakers: usize,
    pub predicted_speakers: usize,
    pub collar_ms: i64,
    pub score_overlap: bool,
}
// Maximum weight one-to-one assignment (Hungarian algorithm), padded with unmatched speakers.
pub(crate) fn assignment(weights: &[Vec<f64>], columns: usize) -> Vec<Option<usize>> {
    let n = weights.len().max(columns);
    if n == 0 {
        return vec![];
    }
    let mut u = vec![0.; n + 1];
    let mut v = vec![0.; n + 1];
    let mut p = vec![0; n + 1];
    let mut way = vec![0; n + 1];
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0;
        let mut min = vec![f64::INFINITY; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = f64::INFINITY;
            let mut j1 = 0;
            for j in 1..=n {
                if !used[j] {
                    let w = weights
                        .get(i0 - 1)
                        .and_then(|r| r.get(j - 1))
                        .copied()
                        .unwrap_or(0.);
                    let cur = -w - u[i0] - v[j];
                    if cur < min[j] {
                        min[j] = cur;
                        way[j] = j0;
                    }
                    if min[j] < delta {
                        delta = min[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=n {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    min[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut out = vec![None; weights.len()];
    for j in 1..=n {
        if p[j] > 0 && p[j] <= out.len() && j <= columns {
            out[p[j] - 1] = Some(j - 1);
        }
    }
    out
}
/// Scores one common timeline, with optimal recording-wide speaker mapping.
/// Input must already be restricted to the evaluation region; collars mask both sides of reference boundaries.
pub fn score_diarization(
    reference: &[SpeakerInterval],
    prediction: &[SpeakerInterval],
    collar_ms: i64,
    score_overlap: bool,
) -> Result<DiarizationScore, ValidationError> {
    if collar_ms < 0 {
        return Err(ValidationError("collar must be nonnegative".into()));
    }
    for s in reference.iter().chain(prediction) {
        s.validate()?;
    }
    let rnames: Vec<_> = reference
        .iter()
        .map(|s| s.speaker.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let hnames: Vec<_> = prediction
        .iter()
        .map(|s| s.speaker.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let ri: BTreeMap<_, _> = rnames
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let hi: BTreeMap<_, _> = hnames
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let mut edges = BTreeSet::new();
    let mut collars = vec![];
    for s in reference.iter().chain(prediction) {
        edges.insert(s.start_ms);
        edges.insert(s.end_ms);
    }
    if collar_ms > 0 {
        for s in reference {
            for t in [s.start_ms, s.end_ms] {
                let a = (t - collar_ms).max(0);
                let b = t.saturating_add(collar_ms);
                collars.push((a, b));
                edges.insert(a);
                edges.insert(b);
            }
        }
    }
    let edges: Vec<_> = edges.into_iter().collect();
    let mut intervals = vec![];
    let mut weights = vec![vec![0.; hnames.len()]; rnames.len()];
    let mut rd = vec![0.; rnames.len()];
    let mut hd = vec![0.; hnames.len()];
    for w in edges.windows(2) {
        let a = w[0];
        let b = w[1];
        if collars.iter().any(|(s, e)| a < *e && b > *s) {
            continue;
        }
        let r: BTreeSet<_> = reference
            .iter()
            .filter(|s| s.start_ms < b && s.end_ms > a)
            .map(|s| ri[s.speaker.as_str()])
            .collect();
        let h: BTreeSet<_> = prediction
            .iter()
            .filter(|s| s.start_ms < b && s.end_ms > a)
            .map(|s| hi[s.speaker.as_str()])
            .collect();
        if !score_overlap && r.len() > 1 {
            continue;
        }
        let duration = (b - a) as f64;
        for &i in &r {
            rd[i] += duration;
            for &j in &h {
                weights[i][j] += duration;
            }
        }
        for &j in &h {
            hd[j] += duration;
        }
        intervals.push((duration, r, h));
    }
    let mapping = assignment(&weights, hnames.len());
    let mut total = 0.;
    let mut missed = 0.;
    let mut false_alarm = 0.;
    let mut confusion = 0.;
    for (d, r, h) in intervals {
        let matched = r
            .iter()
            .filter(|&&i| mapping[i].is_some_and(|j| h.contains(&j)))
            .count();
        total += d * r.len() as f64;
        missed += d * r.len().saturating_sub(h.len()) as f64;
        false_alarm += d * h.len().saturating_sub(r.len()) as f64;
        confusion += d * (r.len().min(h.len()) - matched) as f64;
    }
    let ious: Vec<Vec<f64>> = weights
        .iter()
        .enumerate()
        .map(|(i, row)| {
            row.iter()
                .enumerate()
                .map(|(j, &intersection)| {
                    let union = rd[i] + hd[j] - intersection;
                    if union > 0. { intersection / union } else { 0. }
                })
                .collect()
        })
        .collect();
    let jm = assignment(&ious, hnames.len());
    let jer = if rnames.is_empty() {
        None
    } else {
        Some(
            jm.iter()
                .enumerate()
                .map(|(i, j)| 1. - j.map(|j| ious[i][j]).unwrap_or(0.))
                .sum::<f64>()
                / rnames.len() as f64,
        )
    };
    Ok(DiarizationScore {
        reference_speaker_ms: total,
        missed_speaker_ms: missed,
        false_speaker_ms: false_alarm,
        confused_speaker_ms: confusion,
        der: if total > 0. {
            Some((missed + false_alarm + confusion) / total)
        } else {
            None
        },
        jer,
        reference_speakers: rnames.len(),
        predicted_speakers: hnames.len(),
        collar_ms,
        score_overlap,
    })
}
pub fn word_error_rate(reference: &str, hypothesis: &str) -> Option<f64> {
    let r: Vec<_> = reference.split_whitespace().collect();
    let h: Vec<_> = hypothesis.split_whitespace().collect();
    if r.is_empty() {
        return None;
    }
    Some(word_error_count(&r, &h) as f64 / r.len() as f64)
}
pub(crate) fn word_error_count(r: &[&str], h: &[&str]) -> usize {
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, word) in r.iter().enumerate() {
        let mut row = vec![i + 1; h.len() + 1];
        for j in 1..=h.len() {
            row[j] = (prev[j] + 1)
                .min(row[j - 1] + 1)
                .min(prev[j - 1] + usize::from(*word != h[j - 1]));
        }
        prev = row;
    }
    prev[h.len()]
}
/// Reproducible orthographic normalization only: no semantic or number rewriting.
pub fn normalize_wer(text: &str, protocol: &str) -> Result<String, ValidationError> {
    match protocol {
        "exact" => Ok(text.to_owned()),
        "english_case_punctuation" => Ok(text
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '\'' {
                    c
                } else if c == '’' {
                    '\''
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")),
        _ => Err(ValidationError("unsupported word normalization".into())),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn s(a: i64, b: i64, n: &str) -> SpeakerInterval {
        SpeakerInterval {
            start_ms: a,
            end_ms: b,
            speaker: n.into(),
            track_id: None,
        }
    }
    #[test]
    fn permutation_and_overlap() {
        let r = vec![s(0, 1000, "a"), s(500, 1500, "b")];
        let h = vec![s(0, 1000, "y"), s(500, 1500, "x")];
        let v = score_diarization(&r, &h, 0, true).unwrap();
        assert_eq!(v.der, Some(0.));
        assert_eq!(v.jer, Some(0.));
        assert_eq!(v.reference_speaker_ms, 2000.);
    }
    #[test]
    fn false_alarm_miss_and_confusion() {
        let r = vec![s(0, 1000, "a"), s(1000, 2000, "b")];
        let h = vec![s(500, 2500, "x")];
        let v = score_diarization(&r, &h, 0, true).unwrap();
        assert_eq!(v.missed_speaker_ms, 500.);
        assert_eq!(v.false_speaker_ms, 500.);
        assert_eq!(v.confused_speaker_ms, 500.);
        assert_eq!(v.der, Some(0.75));
    }
    #[test]
    fn word_normalization_is_declared_and_does_not_rewrite_numbers() {
        assert_eq!(
            normalize_wer("HELLO, world! Don't 12.", "english_case_punctuation").unwrap(),
            "hello world don't 12"
        );
        assert_eq!(normalize_wer("Case!", "exact").unwrap(), "Case!");
        assert!(normalize_wer("x", "guess").is_err());
    }
    #[test]
    fn collar_and_wer() {
        assert_eq!(
            score_diarization(&[s(0, 1000, "a")], &[s(50, 950, "x")], 100, true)
                .unwrap()
                .der,
            Some(0.)
        );
        assert_eq!(
            word_error_rate("one two three", "one four three"),
            Some(1. / 3.)
        );
    }
}
