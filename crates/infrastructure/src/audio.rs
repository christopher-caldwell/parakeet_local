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
/// Decode a bounded interval to mono 16kHz PCM. FFmpeg is always read-only on the original.
pub async fn decode(
    track: &AudioTrack,
    start_ms: i64,
    duration_ms: i64,
) -> Result<Vec<f32>, CapabilityError> {
    validate_source(track)?;
    if start_ms < 0 || duration_ms <= 0 || duration_ms > 7_200_000 {
        return Err(CapabilityError::message(
            "invalid decode interval (maximum 2 hours)",
        ));
    }
    let mut child = Command::new("ffmpeg")
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
        .args([
            "-t",
            &format!("{:.3}", duration_ms as f64 / 1000.),
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
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
