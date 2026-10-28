//! Detecting microphone stretches that are mostly the system source heard through speakers.
//!
//! The recordings stay untouched; this only reports where an adaptive filter, given the aligned
//! system audio, can explain most of the microphone's energy. A segment inside such a stretch is
//! far-end speech, so it must not be attributed to the person at the microphone. Windows with
//! local speech on top (double-talk) are explained only partly and are deliberately not flagged,
//! so the owner's own speech is not lost to over-aggressive cancellation.
use crate::vad::SAMPLE_RATE;

const TAPS: usize = 512; // 32 ms of residual echo path after coarse alignment
const STEP: f32 = 0.2;
const REGULARIZATION: f32 = 1e-3;
const WINDOW: usize = SAMPLE_RATE; // one second
const MIN_REFERENCE_RMS: f32 = 0.004;
const MIN_MIC_RMS: f32 = 0.002;
/// Share of microphone energy the system audio must explain before the window counts as bleed.
const EXPLAINED_THRESHOLD: f32 = 0.6;
const MAX_DELAY_MS: usize = 600;

#[derive(Debug, Clone, PartialEq)]
pub struct BleedReport {
    /// Estimated acoustic plus system delay between the two sources, in milliseconds.
    pub delay_ms: i64,
    /// Per-second share of microphone energy explained by the system source (0..1).
    pub explained: Vec<f32>,
    /// Flagged stretches in microphone-local milliseconds, merged.
    pub spans: Vec<(i64, i64)>,
}
fn rms(window: &[f32]) -> f32 {
    (window.iter().map(|s| s * s).sum::<f32>() / window.len().max(1) as f32).sqrt()
}
#[cfg(test)]
/// Aligns `system` onto the microphone's timeline using each source's start offset.
fn align(mic_len: usize, mic_offset_ms: i64, system: &[f32], system_offset_ms: i64) -> Vec<f32> {
    let shift = (system_offset_ms - mic_offset_ms) * (SAMPLE_RATE as i64 / 1000);
    let mut reference = vec![0.; mic_len];
    for (n, slot) in reference.iter_mut().enumerate() {
        let source = n as i64 - shift;
        if source >= 0 && (source as usize) < system.len() {
            *slot = system[source as usize];
        }
    }
    reference
}
/// Coarse delay: whitened (first-difference) signals at 8 kHz, searched over the loudest 20 s.
pub fn estimate_delay(mic: &[f32], reference: &[f32]) -> usize {
    const DECIMATE: usize = 2;
    let prepare = |s: &[f32]| -> Vec<f32> {
        // Pre-emphasis flattens the spectrum so the correlation peak is sharp.
        let whitened: Vec<f32> = s.windows(2).map(|w| w[1] - 0.97 * w[0]).collect();
        whitened.chunks(DECIMATE).map(|c| c[0]).collect()
    };
    let (mic, reference) = (prepare(mic), prepare(reference));
    let span = 20 * SAMPLE_RATE / DECIMATE;
    let max_lag = MAX_DELAY_MS * SAMPLE_RATE / DECIMATE / 1000;
    let energy = |from: usize| {
        reference[from..(from + span).min(reference.len())]
            .iter()
            .map(|s| s * s)
            .sum::<f32>()
    };
    let start = (0..reference.len().saturating_sub(span).max(1))
        .step_by(span / 2)
        .max_by(|a, b| energy(*a).total_cmp(&energy(*b)))
        .unwrap_or(0);
    let end = (start + span).min(reference.len());
    let (mut best, mut best_score) = (0, f32::MIN);
    for lag in 0..=max_lag {
        let score: f32 = (start..end.saturating_sub(lag).max(start))
            .map(|n| reference[n] * mic.get(n + lag).copied().unwrap_or(0.))
            .sum();
        if score > best_score {
            (best, best_score) = (lag, score);
        }
    }
    best * DECIMATE
}
/// Streaming NLMS analysis: feed aligned, delayed blocks whose length is a whole number of
/// seconds; filter state carries across blocks.
pub struct BleedAnalyzer {
    weights: Vec<f32>,
    /// Last `TAPS - 1` reference samples and the running energy of the filter window.
    tail: Vec<f32>,
    norm: f32,
    explained: Vec<f32>,
    delay_samples: usize,
    first_dropped: Option<f32>,
}
impl BleedAnalyzer {
    pub fn new(delay_samples: usize) -> Self {
        Self {
            weights: vec![0.; TAPS],
            tail: vec![0.; TAPS - 1],
            norm: 0.,
            explained: vec![],
            delay_samples,
            first_dropped: None,
        }
    }
    /// `reference` is the system audio already aligned to, and delayed for, this microphone block.
    pub fn push(&mut self, mic: &[f32], reference: &[f32]) {
        let mut history = std::mem::take(&mut self.tail);
        history.extend_from_slice(reference);
        let mut residual = vec![0f32; mic.len()];
        for n in 0..mic.len() {
            // history[n..n + TAPS] is the filter window ending at reference[n].
            let window = &history[n..n + TAPS];
            let newest = window[TAPS - 1];
            let oldest_out = if n == 0 {
                self.tail_first()
            } else {
                history[n - 1]
            };
            self.norm += newest * newest - oldest_out * oldest_out;
            let estimate: f32 = self.weights.iter().zip(window).map(|(w, x)| w * x).sum();
            let error = mic[n] - estimate;
            residual[n] = error;
            let gain = STEP * error / (self.norm.max(0.) + REGULARIZATION);
            for (w, x) in self.weights.iter_mut().zip(window) {
                *w += gain * x;
            }
        }
        for (index, window) in mic.chunks(WINDOW).enumerate() {
            let from = index * WINDOW;
            let to = from + window.len();
            let (mic_rms, reference_rms) = (rms(window), rms(&reference[from..to]));
            self.explained.push(
                if mic_rms >= MIN_MIC_RMS && reference_rms >= MIN_REFERENCE_RMS {
                    (1. - (rms(&residual[from..to]) / mic_rms).powi(2)).clamp(0., 1.)
                } else {
                    0.
                },
            );
        }
        self.tail = history[history.len() - (TAPS - 1)..].to_vec();
        self.first_dropped = Some(history[history.len() - TAPS]);
    }
    fn tail_first(&self) -> f32 {
        self.first_dropped.unwrap_or(0.)
    }
    pub fn finish(self) -> BleedReport {
        let mut spans: Vec<(i64, i64)> = vec![];
        for (index, share) in self.explained.iter().enumerate() {
            if *share >= EXPLAINED_THRESHOLD {
                let (a, b) = (index as i64 * 1000, (index as i64 + 1) * 1000);
                match spans.last_mut() {
                    Some(last) if last.1 == a => last.1 = b,
                    _ => spans.push((a, b)),
                }
            }
        }
        BleedReport {
            delay_ms: (self.delay_samples * 1000 / SAMPLE_RATE) as i64,
            explained: self.explained,
            spans,
        }
    }
}
#[cfg(test)]
fn analyze(mic: &[f32], mic_offset_ms: i64, system: &[f32], system_offset_ms: i64) -> BleedReport {
    let aligned = align(mic.len(), mic_offset_ms, system, system_offset_ms);
    let delay = estimate_delay(mic, &aligned);
    let mut analyzer = BleedAnalyzer::new(delay);
    for (index, block) in mic.chunks(WINDOW * 300).enumerate() {
        let from = index * WINDOW * 300;
        analyzer.push(block, &delayed(&aligned, from, block.len(), delay));
    }
    analyzer.finish()
}
#[cfg(test)]
/// Reference samples for `mic[from..from + len]` after delaying the aligned system audio.
fn delayed(aligned: &[f32], from: usize, len: usize, delay: usize) -> Vec<f32> {
    (from..from + len)
        .map(|n| {
            n.checked_sub(delay)
                .and_then(|i| aligned.get(i))
                .copied()
                .unwrap_or(0.)
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    /// Deterministic noise shaped like syllables, standing in for speech.
    struct Voice(u64);
    impl Voice {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as f32 / (1u64 << 30) as f32 - 1.
        }
        fn speech(&mut self, seconds: usize, level: f32) -> Vec<f32> {
            let mut state = 0.;
            (0..seconds * SAMPLE_RATE)
                .map(|n| {
                    state = 0.9 * state + 0.1 * self.next();
                    let syllables =
                        0.55 + 0.45 * (n as f32 * 2. * std::f32::consts::PI * 3.5 / 16_000.).sin();
                    level * 3. * syllables * state
                })
                .collect()
        }
    }
    fn mix(near: &[f32], far: &[f32], delay: usize, gain: f32) -> Vec<f32> {
        (0..near.len())
            .map(|n| {
                let echo = if n >= delay {
                    far.get(n - delay).copied().unwrap_or(0.)
                } else {
                    0.
                };
                near[n] + gain * echo + 0.0005
            })
            .collect()
    }
    #[test]
    fn leaked_far_end_is_flagged_while_local_and_double_talk_speech_is_kept() {
        let mut voice = Voice(7);
        // Seconds 0-9: far end only (leaks). 10-19: local only. 20-29: both talking.
        let mut far = voice.speech(10, 0.1);
        far.extend(vec![0.0; 10 * SAMPLE_RATE]);
        far.extend(voice.speech(10, 0.1));
        let mut near = vec![0.0; 10 * SAMPLE_RATE];
        near.extend(voice.speech(10, 0.1));
        near.extend(voice.speech(10, 0.1));
        let mic = mix(&near, &far, 1_920, 0.5); // 120 ms delay
        let report = analyze(&mic, 0, &far, 0);
        assert!(
            (100..=140).contains(&report.delay_ms),
            "{}",
            report.delay_ms
        );
        let flagged = |range: std::ops::Range<usize>| {
            report.explained[range.clone()]
                .iter()
                .filter(|e| **e >= EXPLAINED_THRESHOLD)
                .count() as f32
                / range.len() as f32
        };
        let leak = flagged(2..10); // skip filter convergence at the start
        let local = flagged(10..20);
        let double_talk = flagged(20..30);
        // Far-end-only leakage is caught; the owner's speech, alone or over the far end, is not.
        assert!(leak >= 0.9, "leak {leak}");
        assert!(local <= 0.05, "local speech wrongly flagged: {local}");
        assert!(double_talk <= 0.1, "double-talk speech lost: {double_talk}");
    }
    #[test]
    fn offsets_align_a_late_starting_system_track() {
        let mut voice = Voice(11);
        let far = voice.speech(12, 0.1);
        // The system track starts 3 s after the microphone: mic time = system time + 3 s.
        let mut mic_far = vec![0.0; 3 * SAMPLE_RATE];
        mic_far.extend(&far);
        let mic = mix(&vec![0.0; mic_far.len()], &mic_far, 800, 0.5);
        let report = analyze(&mic, 0, &far, 3_000);
        assert!(
            report
                .spans
                .iter()
                .any(|(a, b)| *a <= 6_000 && *b >= 12_000),
            "{:?}",
            report.spans
        );
    }
    #[test]
    fn silence_and_unrelated_speech_are_not_bleed() {
        let mut voice = Voice(3);
        let far = voice.speech(8, 0.1);
        let unrelated = voice.speech(8, 0.1);
        let report = analyze(&unrelated, 0, &far, 0);
        assert!(
            report.spans.is_empty(),
            "{:?} {:?} delay {}",
            report.spans,
            report.explained,
            report.delay_ms
        );
        assert!(
            analyze(&vec![0.0; 8 * SAMPLE_RATE], 0, &far, 0)
                .spans
                .is_empty()
        );
    }
}
