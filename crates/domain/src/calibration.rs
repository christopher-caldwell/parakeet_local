//! Proposing matching criteria from the user's own verified examples.
//!
//! Only verified examples are used. Recordings are split deterministically into tuning and
//! held-out sets; criteria are chosen on the tuning set alone, and the held-out set is only
//! ever scored. Trials never compare an example with references from its own recording.
use crate::VoiceExample;
use crate::recognition::{MatchPolicy, suggest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TrialMetrics {
    pub trials: usize,
    /// Queries whose person has references in another recording.
    pub known_trials: usize,
    pub correct_accepts: usize,
    /// Known person left as Unknown.
    pub missed_known: usize,
    /// Known person named as somebody else.
    pub false_matches: usize,
    /// Queries whose person has no reference elsewhere (an unfamiliar voice).
    pub unfamiliar_trials: usize,
    pub unfamiliar_accepts: usize,
}
impl TrialMetrics {
    pub fn false_accepts(&self) -> usize {
        self.false_matches + self.unfamiliar_accepts
    }
    pub fn false_accept_rate(&self) -> Option<f64> {
        (self.trials > 0).then(|| self.false_accepts() as f64 / self.trials as f64)
    }
    pub fn correct_accept_rate(&self) -> Option<f64> {
        (self.known_trials > 0).then(|| self.correct_accepts as f64 / self.known_trials as f64)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationReport {
    /// `proposed`, or `insufficient_evidence` with the reason.
    pub status: String,
    pub reason: String,
    pub policy: Option<MatchPolicy>,
    pub tuning: TrialMetrics,
    pub held_out: TrialMetrics,
    pub examples: usize,
    pub profiles: usize,
    pub tuning_recordings: usize,
    pub held_out_recordings: usize,
    pub method: String,
}
pub const METHOD: &str = "leave-recording-out grid v1";

fn side(recording_id: &str) -> bool {
    recording_id.bytes().map(u32::from).sum::<u32>() % 2 == 0
}
pub fn run_trials(
    queries: &[VoiceExample],
    references: &[VoiceExample],
    policy: &MatchPolicy,
) -> TrialMetrics {
    let mut metrics = TrialMetrics::default();
    for query in queries {
        let usable: Vec<VoiceExample> = references
            .iter()
            .filter(|r| r.recording_id != query.recording_id)
            .cloned()
            .collect();
        let known = usable.iter().any(|r| {
            r.profile_id == query.profile_id
                && r.model == query.model
                && r.revision == query.revision
                && r.capture_domain == query.capture_domain
        });
        let Ok(decision) = suggest(
            &query.vector,
            &query.model,
            &query.revision,
            &query.capture_domain,
            &usable,
            policy,
        ) else {
            continue;
        };
        metrics.trials += 1;
        match (&decision.suggested_profile, known) {
            (Some(p), true) if *p == query.profile_id => {
                metrics.known_trials += 1;
                metrics.correct_accepts += 1;
            }
            (Some(_), true) => {
                metrics.known_trials += 1;
                metrics.false_matches += 1;
            }
            (None, true) => {
                metrics.known_trials += 1;
                metrics.missed_known += 1;
            }
            (Some(_), false) => {
                metrics.unfamiliar_trials += 1;
                metrics.unfamiliar_accepts += 1;
            }
            (None, false) => metrics.unfamiliar_trials += 1,
        }
    }
    metrics
}
fn grid() -> Vec<MatchPolicy> {
    let mut grid = vec![];
    for score in (50..=95).step_by(5) {
        for margin in (0..=50).step_by(5) {
            for examples in 1..=3 {
                grid.push(MatchPolicy {
                    minimum_score: score as f64 / 100.,
                    minimum_margin: margin as f64 / 100.,
                    minimum_examples: examples,
                });
            }
        }
    }
    grid
}
pub fn calibrate(all: &[VoiceExample]) -> CalibrationReport {
    let verified: Vec<VoiceExample> = all
        .iter()
        .filter(|e| !e.verified_at.is_empty())
        .cloned()
        .collect();
    let profiles = verified
        .iter()
        .map(|e| e.profile_id.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let (tuning, held_out): (Vec<_>, Vec<_>) = verified
        .iter()
        .cloned()
        .partition(|e| side(&e.recording_id));
    let count = |set: &[VoiceExample]| {
        set.iter()
            .map(|e| e.recording_id.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    let mut report = CalibrationReport {
        status: "insufficient_evidence".into(),
        reason: String::new(),
        policy: None,
        tuning: TrialMetrics::default(),
        held_out: TrialMetrics::default(),
        examples: verified.len(),
        profiles,
        tuning_recordings: count(&tuning),
        held_out_recordings: count(&held_out),
        method: METHOD.into(),
    };
    if profiles < 2 || report.tuning_recordings < 2 || report.held_out_recordings < 1 {
        report.reason = "needs verified examples for at least two people, from two or more tuning recordings and at least one held-out recording".into();
        return report;
    }
    // Choose on tuning only: no false accepts, then the most correct accepts, then the strictest.
    let best = grid()
        .into_iter()
        .map(|policy| {
            let metrics = run_trials(&tuning, &tuning, &policy);
            (policy, metrics)
        })
        .filter(|(_, m)| m.false_accepts() == 0 && m.correct_accepts > 0)
        .max_by(|(a, am), (b, bm)| {
            am.correct_accepts
                .cmp(&bm.correct_accepts)
                .then(a.minimum_score.total_cmp(&b.minimum_score))
                .then(a.minimum_margin.total_cmp(&b.minimum_margin))
                .then(a.minimum_examples.cmp(&b.minimum_examples))
        });
    let Some((policy, tuning_metrics)) = best else {
        report.reason =
            "no criteria on the tuning recordings accept a known person without a false accept"
                .into();
        return report;
    };
    report.held_out = run_trials(&held_out, &tuning, &policy);
    report.tuning = tuning_metrics;
    report.policy = Some(policy);
    report.status = "proposed".into();
    report.reason = "chosen on tuning recordings only; held-out recordings were scored once".into();
    report
}
#[cfg(test)]
mod tests {
    use super::*;
    fn example(profile: &str, recording: &str, vector: Vec<f32>) -> VoiceExample {
        VoiceExample {
            id: format!("{profile}-{recording}"),
            profile_id: profile.into(),
            recording_id: recording.into(),
            track_id: "t".into(),
            start_ms: 0,
            end_ms: 5000,
            model: "m".into(),
            revision: "1".into(),
            capture_domain: "system".into(),
            vector,
            verified_at: "now".into(),
            origin: None,
        }
    }
    #[test]
    fn too_little_evidence_yields_no_policy() {
        let report = calibrate(&[example("a", "r1", vec![1., 0.])]);
        assert_eq!(report.status, "insufficient_evidence");
        assert!(report.policy.is_none());
    }
    #[test]
    fn separable_people_across_recordings_get_a_policy_scored_on_held_out_recordings() {
        // Recording ids chosen so both sides of the deterministic split are populated.
        let ids: Vec<String> = (0..12).map(|n| format!("rec-{n}")).collect();
        let mut examples = vec![];
        for (n, rec) in ids.iter().enumerate() {
            let wobble = (n as f32) * 0.01;
            examples.push(example("a", rec, vec![1., wobble, 0.]));
            examples.push(example("b", rec, vec![0., 1., wobble]));
        }
        let report = calibrate(&examples);
        assert_eq!(report.status, "proposed", "{}", report.reason);
        let policy = report.policy.unwrap();
        assert!(policy.minimum_score >= 0.5);
        assert_eq!(report.tuning.false_accepts(), 0);
        assert!(report.held_out.trials > 0);
        assert_eq!(report.held_out.false_accepts(), 0);
        assert_eq!(
            calibrate(&examples).policy.unwrap().minimum_score,
            policy.minimum_score
        );
    }
    #[test]
    fn identical_voices_for_different_people_are_never_accepted() {
        let mut examples = vec![];
        for n in 0..12 {
            let rec = format!("rec-{n}");
            examples.push(example("a", &rec, vec![1., 0.]));
            examples.push(example("b", &rec, vec![1., 0.]));
        }
        let report = calibrate(&examples);
        assert_eq!(report.status, "insufficient_evidence");
    }
}
