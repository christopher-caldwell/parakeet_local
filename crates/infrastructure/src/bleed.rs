//! Streaming bleed detection over a meeting's microphone and system sources.
use crate::{
    audio,
    echo::{BleedAnalyzer, estimate_delay},
    vad::SAMPLE_RATE,
};
use async_trait::async_trait;
use parakeet_application::{CapabilityError, analysis::*};
use parakeet_domain::{AudioTrack, reconciliation::BleedSpan};

const BLOCK_MS: i64 = 300_000;
const MAX_MS: i64 = 7_200_000;
const DELAY_CANDIDATE_BLOCKS: usize = 4;

#[derive(Clone, Default)]
pub struct LocalBleedDetection;

/// Decodes `len` samples of `track` starting at meeting time `meeting_start_ms`, zero-filling
/// before the track starts and after it ends.
async fn samples_at(
    track: &AudioTrack,
    offset: i64,
    meeting_start_ms: i64,
    len: usize,
) -> Result<Vec<f32>, CapabilityError> {
    let local = meeting_start_ms - offset;
    let duration = (len as i64 * 1000 / SAMPLE_RATE as i64) + 1;
    let mut samples = vec![0f32; len];
    let (skip, from) = if local < 0 {
        ((-local) as usize * 16, 0)
    } else {
        (0, local)
    };
    if skip >= len {
        return Ok(samples);
    }
    let decoded = audio::decode(track, from, duration - (skip as i64 / 16)).await?;
    for (slot, value) in samples[skip..].iter_mut().zip(decoded) {
        *slot = value;
    }
    Ok(samples)
}
#[async_trait]
impl BleedDetection for LocalBleedDetection {
    async fn detect(
        &self,
        mic: &AudioTrack,
        system: &AudioTrack,
    ) -> Result<BleedSummary, CapabilityError> {
        let (Some(mic_offset), Some(system_offset)) =
            (mic.meeting_offset_ms(), system.meeting_offset_ms())
        else {
            return Err(CapabilityError::message(
                "both sources need resolved timing for bleed detection",
            ));
        };
        let total = mic
            .duration_ms
            .ok_or_else(|| CapabilityError::message("unknown microphone duration"))?
            .min(MAX_MS);
        let blocks = (total + BLOCK_MS - 1) / BLOCK_MS;
        let block_len = |index: i64| ((total - index * BLOCK_MS).min(BLOCK_MS) as usize) * 16;
        // The delay comes from the early block where the system source is loudest.
        let mut best: Option<(f32, usize)> = None;
        for index in 0..blocks.min(DELAY_CANDIDATE_BLOCKS as i64) {
            let start = index * BLOCK_MS;
            let len = block_len(index);
            let reference = samples_at(system, system_offset, mic_offset + start, len).await?;
            let energy = reference.iter().map(|s| s * s).sum::<f32>();
            if best.is_none_or(|(e, _)| energy > e) {
                let mic_block = samples_at(mic, mic_offset, mic_offset + start, len).await?;
                let delay =
                    tokio::task::spawn_blocking(move || estimate_delay(&mic_block, &reference))
                        .await
                        .map_err(CapabilityError::new)?;
                best = Some((energy, delay));
            }
        }
        let delay = best.map(|(_, d)| d).unwrap_or(0);
        let delay_ms = (delay * 1000 / SAMPLE_RATE) as i64;
        let mut analyzer = BleedAnalyzer::new(delay);
        for index in 0..blocks {
            let start = index * BLOCK_MS;
            let len = block_len(index);
            let mic_block = samples_at(mic, mic_offset, mic_offset + start, len).await?;
            // Microphone time t hears the system source from t - delay.
            let reference =
                samples_at(system, system_offset, mic_offset + start - delay_ms, len).await?;
            analyzer = tokio::task::spawn_blocking(move || {
                analyzer.push(&mic_block, &reference);
                analyzer
            })
            .await
            .map_err(CapabilityError::new)?;
        }
        let report = analyzer.finish();
        let spans: Vec<BleedSpan> = report
            .spans
            .iter()
            .map(|(a, b)| BleedSpan {
                track_id: mic.id.clone(),
                start_ms: mic_offset + a,
                end_ms: mic_offset + b,
            })
            .collect();
        Ok(BleedSummary {
            method: "nlms-512-explained-energy-v1".into(),
            delay_ms: report.delay_ms,
            analyzed_seconds: total as f64 / 1000.,
            flagged_seconds: report
                .spans
                .iter()
                .map(|(a, b)| (b - a) as f64 / 1000.)
                .sum(),
            spans,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parakeet_domain::TrackKind;

    fn wav(path: &std::path::Path, samples: &[f32]) {
        let mut bytes = Vec::new();
        let data = samples.len() as u32 * 2;
        bytes.extend(b"RIFF");
        bytes.extend((36 + data).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(16_000u32.to_le_bytes());
        bytes.extend(32_000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(data.to_le_bytes());
        for s in samples {
            bytes.extend(((s.clamp(-1., 1.) * 32767.) as i16).to_le_bytes());
        }
        std::fs::write(path, bytes).unwrap();
    }
    fn track(path: &std::path::Path, kind: TrackKind, offset: i64, seconds: i64) -> AudioTrack {
        AudioTrack {
            id: format!("{kind:?}"),
            recording_id: "r".into(),
            kind,
            path: path.to_string_lossy().into(),
            start_offset_ms: Some(offset),
            sample_rate: Some(16_000),
            duration_ms: Some(seconds * 1000),
            available: true,
            size_bytes: None,
            modified_unix_ms: None,
            provenance: serde_json::Value::Null,
        }
    }
    fn speechlike(seed: u64, seconds: usize, level: f32) -> Vec<f32> {
        let (mut state, mut lcg) = (0f32, seed);
        (0..seconds * 16_000)
            .map(|n| {
                lcg = lcg
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let noise = (lcg >> 33) as f32 / (1u64 << 30) as f32 - 1.;
                state = 0.9 * state + 0.1 * noise;
                let syllables =
                    0.55 + 0.45 * (n as f32 * 2. * std::f32::consts::PI * 3.5 / 16_000.).sin();
                level * 3. * syllables * state
            })
            .collect()
    }
    #[tokio::test]
    async fn decoded_tracks_with_different_offsets_report_leaked_far_end_in_meeting_time() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let far = speechlike(5, 14, 0.1);
        // System starts at meeting time 2 s; the microphone starts at 0 and hears it 90 ms late,
        // quietly, while the owner is silent the whole time.
        let delay = 1_440;
        let mut mic = vec![0.0005; 2 * 16_000 + delay];
        mic.extend(far.iter().map(|s| 0.5 * s + 0.0005));
        mic.truncate(16 * 16_000);
        wav(&dir.path().join("mic.wav"), &mic);
        wav(&dir.path().join("sys.wav"), &far);
        let summary = LocalBleedDetection
            .detect(
                &track(&dir.path().join("mic.wav"), TrackKind::Microphone, 0, 16),
                &track(&dir.path().join("sys.wav"), TrackKind::System, 2_000, 14),
            )
            .await
            .unwrap();
        assert!(
            (60..=130).contains(&summary.delay_ms),
            "{}",
            summary.delay_ms
        );
        assert!(
            summary
                .spans
                .iter()
                .any(|s| s.start_ms <= 5_000 && s.end_ms >= 14_000),
            "{:?}",
            summary.spans
        );
        assert!(summary.spans.iter().all(|s| s.track_id == "Microphone"));
    }
}
