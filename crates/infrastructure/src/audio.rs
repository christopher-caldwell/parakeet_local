use parakeet_application::CapabilityError;
use parakeet_domain::AudioTrack;
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Stdio,
    time::{Duration, UNIX_EPOCH},
};
use tokio::process::Command;

pub async fn probe(path: &Path) -> Result<Value, CapabilityError> {
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-protocol_whitelist",
                "file,pipe",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=codec_name,sample_rate,channels,duration:format=duration",
                "-of",
                "json",
            ])
            .arg(path)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(CapabilityError::new)?
    .map_err(CapabilityError::new)?;
    if !output.status.success() {
        return Err(CapabilityError::message(format!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let v: Value = serde_json::from_slice(&output.stdout).map_err(CapabilityError::new)?;
    let stream = &v["streams"][0];
    if stream.is_null() {
        return Err(CapabilityError::message("no audio stream found"));
    }
    let duration = stream["duration"]
        .as_str()
        .or_else(|| v["format"]["duration"].as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s >= 0.);
    Ok(
        json!({"codec":stream["codec_name"],"channels":stream["channels"],"sample_rate":stream["sample_rate"].as_str().and_then(|s|s.parse::<u32>().ok()),"duration_ms":duration.map(|s|(s*1000.).round() as i64),"raw":v}),
    )
}
pub fn validate_source(track: &AudioTrack) -> Result<(), CapabilityError> {
    let meta = std::fs::metadata(&track.path).map_err(CapabilityError::new)?;
    if track.size_bytes.is_some_and(|size| size != meta.len()) {
        return Err(CapabilityError::message(
            "source size changed since import; re-inspect original before processing",
        ));
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    if track.modified_unix_ms.is_some() && track.modified_unix_ms != modified {
        return Err(CapabilityError::message(
            "source modification time changed since import",
        ));
    }
    Ok(())
}
fn decoder(
    track: &AudioTrack,
    start_ms: i64,
    duration_ms: i64,
) -> Result<Command, CapabilityError> {
    validate_source(track)?;
    if start_ms < 0 || duration_ms <= 0 || duration_ms > 7_200_000 {
        return Err(CapabilityError::message(
            "invalid decode interval (maximum 2 hours)",
        ));
    }
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-nostdin",
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-ss",
        ])
        .arg(format!("{:.3}", start_ms as f64 / 1000.))
        .arg("-i")
        .arg(&track.path)
        .arg("-t")
        .arg(format!("{:.3}", duration_ms as f64 / 1000.))
        .args([
            "-map", "0:a:0", "-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    Ok(command)
}
/// Decode a bounded interval to mono 16kHz PCM. FFmpeg is always read-only on the original.
pub async fn decode(
    track: &AudioTrack,
    start_ms: i64,
    duration_ms: i64,
) -> Result<Vec<f32>, CapabilityError> {
    let mut child = decoder(track, start_ms, duration_ms)?
        .spawn()
        .map_err(CapabilityError::new)?;
    use tokio::io::AsyncReadExt;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CapabilityError::message("decoder stdout unavailable"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| CapabilityError::message("decoder stderr unavailable"))?;
    let error_task = tokio::spawn(async move {
        let mut e = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let keep = n.min(16_384usize.saturating_sub(e.len()));
                    e.extend_from_slice(&buffer[..keep]);
                }
            }
        }
        e
    });
    let maximum = (duration_ms as u64 * 16 * 4) + 64_000;
    let mut bytes = vec![];
    tokio::time::timeout(
        Duration::from_secs(300),
        stdout.take(maximum + 1).read_to_end(&mut bytes),
    )
    .await
    .map_err(CapabilityError::new)?
    .map_err(CapabilityError::new)?;
    if bytes.len() as u64 > maximum {
        let _ = child.kill().await;
        return Err(CapabilityError::message(
            "decoder output exceeded interval bound",
        ));
    }
    let status = tokio::time::timeout(Duration::from_secs(30), child.wait())
        .await
        .map_err(CapabilityError::new)?
        .map_err(CapabilityError::new)?;
    let errors = error_task.await.map_err(CapabilityError::new)?;
    if !status.success() {
        return Err(CapabilityError::message(format!(
            "FFmpeg decoding failed: {}",
            String::from_utf8_lossy(&errors)
        )));
    }
    if bytes.len() % 4 != 0 || bytes.is_empty() {
        return Err(CapabilityError::message(
            "decoder produced empty or truncated PCM",
        ));
    }
    let samples: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();
    if samples.iter().any(|s| !s.is_finite()) {
        return Err(CapabilityError::message("decoder produced nonfinite PCM"));
    }
    Ok(samples)
}

/// Owns only a newly generated playback WAV. Cancellation must not leave decoded
/// private audio behind. The blocking writer returns ownership to the async task.
struct PlaybackScratch(std::path::PathBuf);
impl Drop for PlaybackScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
#[derive(Clone)]
pub struct LocalPlayback {
    pub root: std::path::PathBuf,
}
#[async_trait::async_trait]
impl parakeet_application::AudioPlayback for LocalPlayback {
    async fn play(
        &self,
        track: &AudioTrack,
        start_ms: i64,
        duration_ms: i64,
    ) -> Result<(), CapabilityError> {
        let samples = decode(track, start_ms, duration_ms).await?;
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(CapabilityError::new)?;
        let path = self
            .root
            .join(format!("{}.wav", parakeet_application::new_id()));
        let scratch = tokio::task::spawn_blocking(move || {
            let scratch = PlaybackScratch(path);
            let mut w = hound::WavWriter::create(
                &scratch.0,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16000,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Float,
                },
            )
            .map_err(CapabilityError::new)?;
            for s in samples {
                w.write_sample(s).map_err(CapabilityError::new)?;
            }
            w.finalize().map_err(CapabilityError::new)?;
            Ok::<_, CapabilityError>(scratch)
        })
        .await
        .map_err(CapabilityError::new)??;
        let result = tokio::time::timeout(
            Duration::from_secs(35),
            Command::new("/usr/bin/afplay")
                .arg(&scratch.0)
                .kill_on_drop(true)
                .status(),
        )
        .await;
        drop(scratch);
        let status = result
            .map_err(CapabilityError::new)?
            .map_err(CapabilityError::new)?;
        if !status.success() {
            return Err(CapabilityError::message("audio playback failed"));
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct LocalAudioIdentity;
#[async_trait::async_trait]
impl parakeet_application::audio_locations::AudioFileIdentityReader for LocalAudioIdentity {
    async fn identify(
        &self,
        path: &Path,
    ) -> Result<parakeet_application::audio_locations::AudioFileIdentity, CapabilityError> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            use sha2::{Digest, Sha256};
            use std::io::Read;
            let path = path.canonicalize().map_err(CapabilityError::new)?;
            let mut file = std::fs::File::open(&path).map_err(CapabilityError::new)?;
            let before = file.metadata().map_err(CapabilityError::new)?;
            if !before.is_file() || before.len() == 0 {
                return Err(CapabilityError::message(
                    "source must be a nonempty regular file",
                ));
            }
            let mut hash = Sha256::new();
            let mut bytes = [0u8; 1024 * 64];
            loop {
                let n = file.read(&mut bytes).map_err(CapabilityError::new)?;
                if n == 0 {
                    break;
                }
                hash.update(&bytes[..n]);
            }
            let after = file.metadata().map_err(CapabilityError::new)?;
            let current = std::fs::metadata(&path).map_err(CapabilityError::new)?;
            use std::os::unix::fs::MetadataExt;
            if before.len() != after.len()
                || before.modified().ok() != after.modified().ok()
                || before.ino() != current.ino()
                || before.dev() != current.dev()
                || before.len() != current.len()
                || before.modified().ok() != current.modified().ok()
            {
                return Err(CapabilityError::message(
                    "source changed while computing its identity",
                ));
            }
            let modified_unix_ms = after
                .modified()
                .map_err(CapabilityError::new)?
                .duration_since(UNIX_EPOCH)
                .map_err(CapabilityError::new)?
                .as_millis() as u64;
            Ok(parakeet_application::audio_locations::AudioFileIdentity {
                path: path.to_string_lossy().into(),
                size_bytes: after.len(),
                modified_unix_ms,
                sha256: format!("{:x}", hash.finalize()),
            })
        })
        .await
        .map_err(CapabilityError::new)?
    }
}

/// Normalize directly to this job's new private PCM file with a fixed buffer.
/// A partial file is retained on failure; the final path is published without
/// replacing an existing file only after complete finite PCM has been checked.
pub(crate) async fn decode_to_file(
    track: &AudioTrack,
    start_ms: i64,
    duration_ms: i64,
    destination: &Path,
) -> Result<u64, CapabilityError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut command = decoder(track, start_ms, duration_ms)?;
    if tokio::fs::try_exists(destination)
        .await
        .map_err(CapabilityError::new)?
    {
        return Err(CapabilityError::message(
            "refusing to replace existing decoded audio",
        ));
    }
    let name = destination
        .file_name()
        .ok_or_else(|| CapabilityError::message("PCM destination has no filename"))?;
    let mut partial_name = name.to_os_string();
    partial_name.push(".partial");
    let partial = destination.with_file_name(partial_name);
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&partial)
        .await
        .map_err(CapabilityError::new)?;
    let mut child = command.spawn().map_err(CapabilityError::new)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| CapabilityError::message("decoder stdout unavailable"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| CapabilityError::message("decoder stderr unavailable"))?;
    let error_task = tokio::spawn(async move {
        let mut errors = vec![];
        let mut buffer = [0u8; 4096];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let keep = n.min(16384usize.saturating_sub(errors.len()));
                    errors.extend_from_slice(&buffer[..keep]);
                }
            }
        }
        errors
    });
    let maximum = duration_ms as u64 * 16 * 4 + 64000;
    let count = tokio::time::timeout(Duration::from_secs(300), async {
        let mut buffer = [0u8; 65536];
        let mut carry = 0;
        let mut count = 0u64;
        loop {
            let n = stdout
                .read(&mut buffer[carry..])
                .await
                .map_err(CapabilityError::new)?;
            if n == 0 {
                if carry != 0 {
                    return Err(CapabilityError::message("decoder produced truncated PCM"));
                }
                break;
            }
            let total = carry + n;
            let complete = total / 4 * 4;
            count += complete as u64;
            if count > maximum {
                return Err(CapabilityError::message(
                    "decoder output exceeded interval bound",
                ));
            }
            if buffer[..complete]
                .as_chunks::<4>()
                .0
                .iter()
                .any(|b| !f32::from_le_bytes(*b).is_finite())
            {
                return Err(CapabilityError::message("decoder produced nonfinite PCM"));
            }
            file.write_all(&buffer[..complete])
                .await
                .map_err(CapabilityError::new)?;
            buffer.copy_within(complete..total, 0);
            carry = total - complete;
        }
        if count == 0 {
            return Err(CapabilityError::message("decoder produced empty PCM"));
        }
        Ok(count)
    })
    .await
    .map_err(CapabilityError::new)??;
    let status = tokio::time::timeout(Duration::from_secs(30), child.wait())
        .await
        .map_err(CapabilityError::new)?
        .map_err(CapabilityError::new)?;
    let errors = error_task.await.map_err(CapabilityError::new)?;
    if !status.success() {
        return Err(CapabilityError::message(format!(
            "FFmpeg decoding failed: {}",
            String::from_utf8_lossy(&errors)
        )));
    }
    file.flush().await.map_err(CapabilityError::new)?;
    file.sync_all().await.map_err(CapabilityError::new)?;
    drop(file);
    tokio::fs::hard_link(&partial, destination)
        .await
        .map_err(CapabilityError::new)?;
    tokio::fs::remove_file(&partial)
        .await
        .map_err(CapabilityError::new)?;
    Ok(count / 4)
}

#[cfg(test)]
mod decoder_tests {
    use super::*;
    fn source(path: &Path) -> AudioTrack {
        AudioTrack {
            id: "pcm".into(),
            recording_id: "fixture".into(),
            kind: parakeet_domain::TrackKind::File,
            path: path.to_string_lossy().into(),
            start_offset_ms: Some(1250),
            sample_rate: Some(16000),
            duration_ms: Some(3000),
            available: true,
            size_bytes: None,
            modified_unix_ms: None,
            provenance: json!({}),
        }
    }
    #[tokio::test]
    async fn streamed_pcm_matches_memory_decode_and_never_replaces_input() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("source.wav");
        let mut wav = hound::WavWriter::create(
            &input,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..48000 {
            wav.write_sample((i as f32 / 30.).sin() * 0.2).unwrap();
        }
        wav.finalize().unwrap();
        let track = source(&input);
        let original = std::fs::read(&input).unwrap();
        let expected = decode(&track, 117, 1234).await.unwrap();
        let output = tmp.path().join("audio.f32");
        let samples = decode_to_file(&track, 117, 1234, &output).await.unwrap();
        let bytes = std::fs::read(&output).unwrap();
        assert_eq!(samples, expected.len() as u64);
        assert_eq!(
            bytes,
            expected
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<_>>()
        );
        assert!(!tmp.path().join("audio.f32.partial").exists());
        assert!(decode_to_file(&track, 117, 1234, &output).await.is_err());
        assert_eq!(std::fs::read(output).unwrap(), bytes);
        assert!(decode_to_file(&track, 0, 1000, &input).await.is_err());
        assert_eq!(std::fs::read(input).unwrap(), original);
    }
    #[tokio::test]
    async fn decoder_failure_keeps_final_output_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("broken.wav");
        std::fs::write(&input, b"not audio").unwrap();
        let output = tmp.path().join("audio.f32");
        assert!(
            decode_to_file(&source(&input), 0, 1000, &output)
                .await
                .is_err()
        );
        assert!(!output.exists());
        assert!(tmp.path().join("audio.f32.partial").exists());
        let invalid = tmp.path().join("invalid.f32");
        assert!(
            decode_to_file(&source(&input), -1, 1000, &invalid)
                .await
                .is_err()
        );
        assert!(!tmp.path().join("invalid.f32.partial").exists());
    }
    #[tokio::test]
    async fn nonfinite_pcm_is_not_published_to_the_model() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("nonfinite.wav");
        let mut wav = hound::WavWriter::create(
            &input,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for _ in 0..16000 {
            wav.write_sample(f32::NAN).unwrap();
        }
        wav.finalize().unwrap();
        let output = tmp.path().join("audio.f32");
        assert!(
            decode_to_file(&source(&input), 0, 1000, &output)
                .await
                .is_err()
        );
        assert!(!output.exists());
    }
    /// Explicit local resource probe; no downloads, inference or personal audio.
    #[tokio::test]
    #[ignore = "requires an explicitly selected public/synthetic PCM probe input"]
    async fn decoder_resource_probe() {
        let path = std::env::var_os("PARAKEET_PCM_PROBE_INPUT").expect("explicit probe input");
        let mode = std::env::var("PARAKEET_PCM_PROBE_MODE").expect("probe mode");
        let mut track = source(Path::new(&path));
        let duration = std::env::var("PARAKEET_PCM_PROBE_MS")
            .unwrap()
            .parse()
            .unwrap();
        track.duration_ms = Some(duration);
        let started = std::time::Instant::now();
        let samples = if mode == "memory" {
            decode(&track, 0, duration).await.unwrap().len() as u64
        } else {
            assert_eq!(mode, "disk");
            let output = std::env::var_os("PARAKEET_PCM_PROBE_OUTPUT").expect("new output path");
            decode_to_file(&track, 0, duration, Path::new(&output))
                .await
                .unwrap()
        };
        eprintln!(
            "PCM_PROBE {}",
            json!({"mode":mode,"samples":samples,"wall_seconds":started.elapsed().as_secs_f64()})
        );
    }
}
