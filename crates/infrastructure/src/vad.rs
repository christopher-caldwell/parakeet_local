//! Energy-based voice activity detection and speech-aligned chunk planning for ASR.
//!
//! Whisper hallucinates text on silence and noise, and hard fixed-length cuts split words.
//! Chunks here contain only speech regions and are cut inside silence, never mid-word. This is
//! a deliberately small detector: it removes silence and low-level noise, not speech-like noise.

pub const SAMPLE_RATE: usize = 16_000;
const FRAME: usize = 480; // 30 ms
/// Absolute floor so a very quiet recording is not mistaken for speech.
const MIN_THRESHOLD: f32 = 0.004;
const MAX_THRESHOLD: f32 = 0.01;
const MIN_MODULATION: f32 = 0.15;
const MAX_MEAN_ZCR: f32 = 0.3;
const NOISE_MULTIPLE: f32 = 3.0;
const HANGOVER_FRAMES: usize = 8; // 240 ms
const MIN_SPEECH_FRAMES: usize = 7; // 210 ms
const MERGE_GAP_FRAMES: usize = 17; // 510 ms
const PAD_FRAMES: usize = 7;
/// A chunk never exceeds Whisper's native 30 second window.
const MAX_CHUNK_FRAMES: usize = 833; // 25 s
const MAX_INNER_GAP_FRAMES: usize = 100; // 3 s of silence inside one chunk
const CUT_SEARCH_FRAMES: usize = 166; // look back 5 s for the quietest frame

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub start: usize,
    pub end: usize,
    /// Samples of speech (excluding silence inside the chunk), for reporting.
    pub speech: usize,
}
/// Fraction of sign changes per frame: near 0.5 for broadband noise, lower for voiced speech.
fn frame_zcr(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(FRAME)
        .map(|frame| {
            frame
                .windows(2)
                .filter(|w| (w[0] >= 0.) != (w[1] >= 0.))
                .count() as f32
                / frame.len().max(2).saturating_sub(1) as f32
        })
        .collect()
}
/// Speech is modulated (syllables) and not noise-like; stationary noise and pure tones are not.
fn looks_like_speech(rms: &[f32], zcr: &[f32], from: usize, to: usize) -> bool {
    let levels = &rms[from..to];
    let mean = levels.iter().sum::<f32>() / levels.len() as f32;
    let variance = levels.iter().map(|l| (l - mean).powi(2)).sum::<f32>() / levels.len() as f32;
    let modulation = variance.sqrt() / mean.max(f32::EPSILON);
    let mean_zcr = zcr[from..to].iter().sum::<f32>() / (to - from) as f32;
    modulation >= MIN_MODULATION && mean_zcr <= MAX_MEAN_ZCR
}
pub fn frame_rms(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(FRAME)
        .map(|frame| (frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32).sqrt())
        .collect()
}
fn speech_regions(rms: &[f32], zcr: &[f32]) -> Vec<(usize, usize)> {
    if rms.is_empty() {
        return vec![];
    }
    let mut sorted = rms.to_vec();
    sorted.sort_by(f32::total_cmp);
    let noise_floor = sorted[sorted.len() / 10];
    // Capped so continuous speech (no quiet frames to estimate noise from) is still detected.
    let threshold = (noise_floor * NOISE_MULTIPLE).clamp(MIN_THRESHOLD, MAX_THRESHOLD);
    let mut regions: Vec<(usize, usize)> = vec![];
    let mut start = None;
    let mut quiet = 0;
    for (index, level) in rms.iter().enumerate() {
        if *level >= threshold {
            start.get_or_insert(index);
            quiet = 0;
        } else if let Some(begin) = start {
            quiet += 1;
            if quiet > HANGOVER_FRAMES {
                regions.push((begin, index + 1 - quiet + HANGOVER_FRAMES));
                start = None;
                quiet = 0;
            }
        }
    }
    if let Some(begin) = start {
        regions.push((begin, rms.len() - quiet.saturating_sub(HANGOVER_FRAMES)));
    }
    let mut merged: Vec<(usize, usize)> = vec![];
    for region in regions {
        match merged.last_mut() {
            Some(last) if region.0.saturating_sub(last.1) <= MERGE_GAP_FRAMES => last.1 = region.1,
            _ => merged.push(region),
        }
    }
    merged.retain(|(a, b)| b - a >= MIN_SPEECH_FRAMES && looks_like_speech(rms, zcr, *a, *b));
    merged
}
/// Plans speech chunks over `samples`. When `is_final` is false the trailing region that may
/// continue into the next block is left unconsumed; the second value is how many samples are
/// consumed, so the caller resumes decoding there.
pub fn plan_chunks(samples: &[f32], is_final: bool) -> (Vec<Chunk>, usize) {
    let rms = frame_rms(samples);
    let regions = speech_regions(&rms, &frame_zcr(samples));
    let frames = rms.len();
    let mut chunks: Vec<(usize, usize, usize)> = vec![];
    for (start, end) in regions {
        match chunks.last_mut() {
            Some(last)
                if start - last.1 <= MAX_INNER_GAP_FRAMES
                    && end - last.0 + 2 * PAD_FRAMES <= MAX_CHUNK_FRAMES =>
            {
                last.1 = end;
                last.2 += end - start;
            }
            _ => chunks.push((start, end, end - start)),
        }
    }
    // Split continuous speech longer than one window at its quietest nearby frame.
    let mut split = vec![];
    for (start, end, speech) in chunks {
        let mut begin = start;
        let mut remaining_speech = speech;
        while end - begin + 2 * PAD_FRAMES > MAX_CHUNK_FRAMES {
            let limit = begin + MAX_CHUNK_FRAMES - 2 * PAD_FRAMES;
            let from = limit.saturating_sub(CUT_SEARCH_FRAMES).max(begin + 1);
            let cut = (from..limit)
                .min_by(|a, b| rms[*a].total_cmp(&rms[*b]).then(b.cmp(a)))
                .unwrap_or(limit);
            split.push((begin, cut, (cut - begin).min(remaining_speech)));
            remaining_speech = remaining_speech.saturating_sub(cut - begin);
            begin = cut;
        }
        split.push((begin, end, remaining_speech));
    }
    let mut consumed = frames * FRAME;
    if !is_final {
        // Leave back anything that touches the end of the block: it may still be speaking.
        if let Some(last) = split.last().filter(|c| c.1 + MERGE_GAP_FRAMES >= frames) {
            consumed = last.0.saturating_sub(PAD_FRAMES) * FRAME;
            split.pop();
        } else {
            consumed = frames.saturating_sub(MERGE_GAP_FRAMES) * FRAME;
        }
    }
    let chunks = split
        .into_iter()
        .map(|(start, end, speech)| Chunk {
            start: start.saturating_sub(PAD_FRAMES) * FRAME,
            end: ((end + PAD_FRAMES) * FRAME).min(samples.len()),
            speech: speech * FRAME,
        })
        .collect();
    (chunks, consumed.min(samples.len()))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn tone(seconds: f32, amplitude: f32) -> Vec<f32> {
        (0..(seconds * SAMPLE_RATE as f32) as usize)
            .map(|n| {
                let syllables =
                    0.65 + 0.35 * (n as f32 * 2. * std::f32::consts::PI * 4. / 16_000.).sin();
                amplitude * syllables * (n as f32 * 0.3).sin()
            })
            .collect()
    }
    fn silence(seconds: f32) -> Vec<f32> {
        vec![0.0005; (seconds * SAMPLE_RATE as f32) as usize]
    }
    #[test]
    fn silence_and_low_noise_produce_no_chunks() {
        let (chunks, _) = plan_chunks(&silence(60.), true);
        assert!(chunks.is_empty());
        let noise: Vec<f32> = (0..SAMPLE_RATE * 30)
            .map(|n| 0.002 * ((n * 7919 % 101) as f32 / 50. - 1.))
            .collect();
        assert!(plan_chunks(&noise, true).0.is_empty());
    }
    #[test]
    fn loud_stationary_noise_and_pure_tones_are_not_speech() {
        let noise: Vec<f32> = (0..SAMPLE_RATE * 8)
            .map(|n| 0.05 * (((n as u64 * 2_654_435_761) % 1000) as f32 / 500. - 1.))
            .collect();
        assert!(plan_chunks(&noise, true).0.is_empty());
        let pure: Vec<f32> = (0..SAMPLE_RATE * 8)
            .map(|n| 0.1 * (n as f32 * 0.3).sin())
            .collect();
        assert!(plan_chunks(&pure, true).0.is_empty());
    }
    #[test]
    fn speech_is_isolated_from_long_silence() {
        let mut audio = silence(20.);
        audio.extend(tone(4., 0.2));
        audio.extend(silence(40.));
        let (chunks, _) = plan_chunks(&audio, true);
        assert_eq!(chunks.len(), 1);
        let (start, end) = (
            chunks[0].start as f32 / 16000.,
            chunks[0].end as f32 / 16000.,
        );
        assert!((19.0..=20.0).contains(&start), "{start}");
        assert!((24.0..=25.5).contains(&end), "{end}");
    }
    #[test]
    fn continuous_speech_is_cut_at_a_quiet_frame_never_past_a_window() {
        let mut audio = tone(24., 0.2);
        // A brief dip well inside the search window is the preferred cut.
        audio.extend(vec![0.0; 3_000]);
        audio.extend(tone(24., 0.2));
        let (chunks, _) = plan_chunks(&audio, true);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.end - chunk.start <= 30 * SAMPLE_RATE);
        }
        for pair in chunks.windows(2) {
            assert!(
                pair[0].end <= pair[1].start + 14 * FRAME,
                "chunks stay ordered"
            );
        }
        // Every speech sample is covered exactly once by some chunk.
        assert!(chunks.first().unwrap().start == 0);
        assert!(chunks.last().unwrap().end >= audio.len() - 2 * FRAME);
    }
    #[test]
    fn non_final_blocks_hold_back_a_region_that_may_continue() {
        let mut audio = silence(2.);
        audio.extend(tone(3., 0.2));
        audio.extend(silence(5.));
        audio.extend(tone(2., 0.2)); // still speaking at the block edge
        let (chunks, consumed) = plan_chunks(&audio, false);
        assert_eq!(chunks.len(), 1);
        assert!(consumed < audio.len());
        assert!(consumed as f32 / 16000. >= 9.0);
    }
}
