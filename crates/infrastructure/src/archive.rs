//! Lossless WavPack copies of raw captures, verified sample by sample before anything is replaced.
use async_trait::async_trait;
use parakeet_application::{CapabilityError, audio_archive::AudioArchiver};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub struct FfmpegArchiver;

fn decode_command(path: &Path) -> Command {
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-nostdin",
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(path)
        .args(["-map", "0:a:0", "-vn", "-f", "f32le", "pipe:1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}
fn read_block(reader: &mut impl Read, block: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < block.len() {
        let n = reader.read(&mut block[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}
/// Decodes both files natively (no resampling, same channels) and compares every sample.
fn same_samples(original: &Path, copy: &Path) -> Result<(), CapabilityError> {
    let mut a = decode_command(original)
        .spawn()
        .map_err(CapabilityError::new)?;
    let mut b = decode_command(copy).spawn().map_err(CapabilityError::new)?;
    let result = (|| {
        let (mut left, mut right) = (
            a.stdout
                .take()
                .ok_or_else(|| CapabilityError::message("decoder output missing"))?,
            b.stdout
                .take()
                .ok_or_else(|| CapabilityError::message("decoder output missing"))?,
        );
        let (mut x, mut y) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
        let mut total = 0usize;
        loop {
            let n = read_block(&mut left, &mut x).map_err(CapabilityError::new)?;
            let m = read_block(&mut right, &mut y).map_err(CapabilityError::new)?;
            if n != m {
                return Err(CapabilityError::message(
                    "archived copy has a different length",
                ));
            }
            if n == 0 {
                break;
            }
            if x[..n] != y[..n] {
                return Err(CapabilityError::message(
                    "archived copy differs from the original",
                ));
            }
            total += n;
        }
        if total == 0 {
            return Err(CapabilityError::message("original decoded to no audio"));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = a.kill();
        let _ = b.kill();
    }
    let left = a.wait().map_err(CapabilityError::new)?;
    let right = b.wait().map_err(CapabilityError::new)?;
    result?;
    if !left.success() || !right.success() {
        return Err(CapabilityError::message(
            "archive verification decoder failed",
        ));
    }
    Ok(())
}
fn encode(source: &Path) -> Result<PathBuf, CapabilityError> {
    // Each attempt owns its copy. If another process wins the locator CAS, cleanup
    // must never remove the winner's file or its in-progress encoder output.
    let finished = source.with_extension(format!("{}.wv", uuid::Uuid::new_v4()));
    if finished.exists() {
        return Err(CapabilityError::message(
            "a .wv copy already exists; nothing overwritten",
        ));
    }
    let partial = finished.with_extension("wv.partial");
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(source)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-c:a",
            "wavpack",
            "-sample_fmt",
            "fltp",
            "-f",
            "wv",
        ])
        .arg(&partial)
        .stdin(Stdio::null())
        .status()
        .map_err(CapabilityError::new)?;
    if !status.success() {
        let _ = std::fs::remove_file(&partial);
        return Err(CapabilityError::message(
            "FFmpeg could not encode the lossless copy",
        ));
    }
    if let Err(e) = same_samples(source, &partial) {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    std::fs::File::open(&partial)
        .and_then(|f| f.sync_all())
        .map_err(CapabilityError::new)?;
    std::fs::rename(&partial, &finished).map_err(CapabilityError::new)?;
    Ok(finished)
}
#[async_trait]
impl AudioArchiver for FfmpegArchiver {
    async fn encode_verified(&self, source: &Path) -> Result<PathBuf, CapabilityError> {
        let source = source.to_owned();
        tokio::task::spawn_blocking(move || encode(&source))
            .await
            .map_err(CapabilityError::new)?
    }
    async fn remove(&self, path: &Path) -> Result<(), CapabilityError> {
        std::fs::remove_file(path).map_err(CapabilityError::new)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn synthetic_caf(dir: &Path) -> PathBuf {
        let path = dir.join("microphone-raw.caf");
        let status = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2:sample_rate=48000",
                "-af",
                "volume=0.4",
                "-c:a",
                "pcm_f32le",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        path
    }
    #[tokio::test]
    async fn archive_is_verified_and_smaller() {
        let dir = tempfile::tempdir().unwrap();
        let raw = synthetic_caf(dir.path());
        let copy = FfmpegArchiver.encode_verified(&raw).await.unwrap();
        assert_eq!(copy.extension().unwrap(), "wv");
        same_samples(&raw, &copy).unwrap();
        assert!(std::fs::metadata(&copy).unwrap().len() < std::fs::metadata(&raw).unwrap().len());
        assert!(
            raw.exists(),
            "the original is only removed by the service after the swap"
        );
        assert!(!copy.with_extension("wv.partial").exists());
    }
    #[tokio::test]
    async fn preserves_existing_copies_and_rejects_different_audio() {
        let dir = tempfile::tempdir().unwrap();
        let raw = synthetic_caf(dir.path());
        std::fs::write(dir.path().join("microphone-raw.wv"), b"existing").unwrap();
        let fresh = FfmpegArchiver.encode_verified(&raw).await.unwrap();
        assert_ne!(fresh, dir.path().join("microphone-raw.wv"));
        assert_eq!(
            std::fs::read(dir.path().join("microphone-raw.wv")).unwrap(),
            b"existing"
        );
        let other = dir.path().join("other.caf");
        let status = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:duration=2:sample_rate=48000",
                "-c:a",
                "pcm_f32le",
            ])
            .arg(&other)
            .status()
            .unwrap();
        assert!(status.success());
        let flac = dir.path().join("other.flac");
        assert!(
            Command::new("ffmpeg")
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(&raw)
                .arg(&flac)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            same_samples(&other, &flac).is_err(),
            "different audio must fail verification"
        );
    }
}
