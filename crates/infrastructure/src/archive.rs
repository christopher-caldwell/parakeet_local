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
    remove_abandoned_partials(source);
    let partial = Partial(finished.with_extension("wv.partial"));
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
        .arg(&partial.0)
        .stdin(Stdio::null())
        .status()
        .map_err(CapabilityError::new)?;
    if !status.success() {
        return Err(CapabilityError::message(
            "FFmpeg could not encode the lossless copy",
        ));
    }
    same_samples(source, &partial.0)?;
    std::fs::File::open(&partial.0)
        .and_then(|f| f.sync_all())
        .map_err(CapabilityError::new)?;
    std::fs::rename(&partial.0, &finished).map_err(CapabilityError::new)?;
    Ok(finished)
}
/// This attempt's encoder output, removed on every early return; renaming it away keeps it.
struct Partial(PathBuf);
impl Drop for Partial {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
/// No encode runs for a day, so an older partial of this source was left by a process that was
/// killed or crashed. A partial still being written by another process is always newer.
const ABANDONED_PARTIAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
fn remove_abandoned_partials(source: &Path) {
    let (Some(dir), Some(stem)) = (source.parent(), source.file_stem().and_then(|s| s.to_str()))
    else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let abandoned = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > ABANDONED_PARTIAL);
        if abandoned && name.starts_with(&format!("{stem}.")) && name.ends_with(".wv.partial") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
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
    #[test]
    fn only_abandoned_partials_of_this_source_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("microphone-raw.caf");
        let old = dir.path().join("microphone-raw.a.wv.partial");
        let fresh = dir.path().join("microphone-raw.b.wv.partial");
        let other = dir.path().join("system-raw.c.wv.partial");
        for path in [&old, &fresh, &other] {
            std::fs::write(path, b"partial").unwrap();
        }
        let two_days_ago = std::time::SystemTime::now() - 2 * ABANDONED_PARTIAL;
        for path in [&old, &other] {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(two_days_ago)
                .unwrap();
        }
        remove_abandoned_partials(&source);
        assert!(!old.exists());
        assert!(fresh.exists(), "another process may still be writing it");
        assert!(
            other.exists(),
            "another source's partial is not this attempt's to remove"
        );
    }
}
