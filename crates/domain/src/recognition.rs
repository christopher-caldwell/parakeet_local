use crate::{ValidationError, VoiceExample};
use serde::{Deserialize, Serialize};

pub fn cosine(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() || a.iter().chain(b).any(|x| !x.is_finite()) {
        return None;
    }
    let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
    let an: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum();
    let bn: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum();
    if an <= 0. || bn <= 0. {
        None
    } else {
        Some((dot / (an * bn).sqrt()).clamp(-1., 1.))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchPolicy {
    pub minimum_score: f64,
    pub minimum_margin: f64,
    pub minimum_examples: usize,
}
impl MatchPolicy {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !self.minimum_score.is_finite()
            || !(-1.0..=1.0).contains(&self.minimum_score)
            || !self.minimum_margin.is_finite()
            || !(0.0..=2.0).contains(&self.minimum_margin)
            || self.minimum_examples == 0
        {
            return Err(ValidationError("invalid matching policy".into()));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityCandidate {
    pub profile_id: String,
    pub score: f64,
    pub examples: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityDecision {
    pub suggested_profile: Option<String>,
    pub candidates: Vec<IdentityCandidate>,
    pub reason: String,
    /// How candidate scores were computed; scores from different methods are not comparable.
    #[serde(default)]
    pub scoring_method: String,
}
pub const SCORING_METHOD: &str = "centroid-cosine-v1";
/// Suggestions never modify verified examples. Different model versions or capture domains are excluded.
pub fn suggest(
    vector: &[f32],
    model: &str,
    revision: &str,
    capture_domain: &str,
    examples: &[VoiceExample],
    policy: &MatchPolicy,
) -> Result<IdentityDecision, ValidationError> {
    policy.validate()?;
    if cosine(vector, vector).is_none() {
        return Err(ValidationError(
            "embedding must be finite, nonempty and nonzero".into(),
        ));
    }
    let mut grouped = std::collections::BTreeMap::<String, (Vec<f64>, usize)>::new();
    let mut eligible: Vec<_> = examples
        .iter()
        .filter(|e| {
            e.model == model
                && e.revision == revision
                && e.capture_domain == capture_domain
                && !e.verified_at.is_empty()
                && e.start_ms >= 0
                && e.end_ms - e.start_ms >= 3000
        })
        .collect();
    eligible.sort_by(|a, b| {
        (b.end_ms - b.start_ms)
            .cmp(&(a.end_ms - a.start_ms))
            .then(a.id.cmp(&b.id))
    });
    let mut admitted: Vec<&VoiceExample> = vec![];
    for e in eligible {
        if admitted.iter().any(|previous| {
            previous.profile_id == e.profile_id
                && previous.recording_id == e.recording_id
                && previous.track_id == e.track_id
                && previous.start_ms < e.end_ms
                && previous.end_ms > e.start_ms
        }) {
            continue;
        }
        admitted.push(e);
        let Some(norm) = cosine(&e.vector, &e.vector).map(|_| {
            e.vector
                .iter()
                .map(|x| (*x as f64).powi(2))
                .sum::<f64>()
                .sqrt()
        }) else {
            continue;
        };
        let (sum, count) = grouped
            .entry(e.profile_id.clone())
            .or_insert_with(|| (vec![0.; e.vector.len()], 0));
        if sum.len() == e.vector.len() {
            for (total, x) in sum.iter_mut().zip(&e.vector) {
                *total += *x as f64 / norm;
            }
            *count += 1;
        }
    }
    let mut candidates: Vec<_> = grouped
        .into_iter()
        .filter_map(|(profile_id, (sum, examples))| {
            // Score against the re-normalized centroid so varied correct examples
            // do not lower a person's score the way a mean of cosines would.
            let centroid: Vec<f32> = sum.iter().map(|x| *x as f32).collect();
            Some(IdentityCandidate {
                profile_id,
                score: cosine(vector, &centroid)?,
                examples,
            })
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.profile_id.cmp(&b.profile_id))
    });
    let best = candidates.first();
    let margin = best
        .map(|b| b.score - candidates.get(1).map(|c| c.score).unwrap_or(-1.))
        .unwrap_or(0.);
    let accepted = best
        .filter(|b| {
            b.score >= policy.minimum_score
                && b.examples >= policy.minimum_examples
                && margin >= policy.minimum_margin
        })
        .map(|b| b.profile_id.clone());
    Ok(IdentityDecision {
        reason: if accepted.is_some() {
            "meets_explicit_policy"
        } else {
            "unknown_insufficient_or_ambiguous_evidence"
        }
        .into(),
        suggested_profile: accepted,
        candidates,
        scoring_method: SCORING_METHOD.into(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn e(id: &str, domain: &str) -> VoiceExample {
        VoiceExample {
            id: id.into(),
            profile_id: id.into(),
            recording_id: "r".into(),
            track_id: "t".into(),
            start_ms: 0,
            end_ms: 5000,
            model: "m".into(),
            revision: "1".into(),
            capture_domain: domain.into(),
            vector: vec![1., 0.],
            verified_at: "now".into(),
        }
    }
    #[test]
    fn unknown_on_ambiguous_and_incompatible() {
        let p = MatchPolicy {
            minimum_score: 0.9,
            minimum_margin: 0.1,
            minimum_examples: 1,
        };
        let es = vec![e("a", "system"), e("b", "system")];
        assert!(
            suggest(&[1., 0.], "m", "1", "system", &es, &p)
                .unwrap()
                .suggested_profile
                .is_none()
        );
        assert!(
            suggest(&[1., 0.], "m", "2", "system", &es, &p)
                .unwrap()
                .candidates
                .is_empty()
        );
        assert!(
            suggest(&[1., 0.], "m", "1", "microphone", &es, &p)
                .unwrap()
                .candidates
                .is_empty()
        );
    }
    #[test]
    fn overlapping_references_do_not_inflate_evidence_count() {
        let mut first = e("person", "system");
        let mut duplicate = first.clone();
        duplicate.id = "duplicate".into();
        duplicate.start_ms = 1000;
        duplicate.end_ms = 6000;
        first.id = "first".into();
        let decision = suggest(
            &[1., 0.],
            "m",
            "1",
            "system",
            &[first, duplicate],
            &MatchPolicy {
                minimum_score: 0.9,
                minimum_margin: 0.1,
                minimum_examples: 2,
            },
        )
        .unwrap();
        assert!(decision.suggested_profile.is_none());
        assert_eq!(decision.candidates[0].examples, 1);
    }
    fn at(id: &str, profile: &str, start: i64, vector: Vec<f32>) -> VoiceExample {
        let mut x = e(id, "system");
        x.profile_id = profile.into();
        x.start_ms = start;
        x.end_ms = start + 5000;
        x.vector = vector;
        x
    }
    #[test]
    fn diverse_correct_examples_do_not_lower_the_true_score() {
        let policy = MatchPolicy {
            minimum_score: 0.5,
            minimum_margin: 0.0,
            minimum_examples: 1,
        };
        let query = [1., 0., 0.];
        let single = suggest(
            &query,
            "m",
            "1",
            "system",
            &[at("a", "p", 0, vec![1., 0., 0.])],
            &policy,
        )
        .unwrap()
        .candidates[0]
            .score;
        let varied = [
            at("a", "p", 0, vec![1., 0., 0.]),
            at("b", "p", 10_000, vec![0.8, 0.6, 0.]),
            at("c", "p", 20_000, vec![0.8, 0., 0.6]),
        ];
        let many = suggest(&query, "m", "1", "system", &varied, &policy).unwrap();
        assert!(many.candidates[0].score > 0.9 * single);
        assert_eq!(many.scoring_method, SCORING_METHOD);
        let impostor = suggest(&[0., 1., 0.], "m", "1", "system", &varied, &policy)
            .unwrap()
            .candidates[0]
            .score;
        assert!(impostor < many.candidates[0].score);
    }
    #[test]
    fn invalid_vectors_are_rejected() {
        assert!(cosine(&[f32::NAN], &[1.]).is_none());
        assert!(cosine(&[0.], &[0.]).is_none());
    }
}
