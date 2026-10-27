use parakeet_application::{CapabilityError, RecordingSource};
use parakeet_domain::ImportBundle;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;

#[derive(Clone)]
pub struct YouTubeMedia {
    pub executable: PathBuf,
    pub root: PathBuf,
}
pub fn validate_youtube_url(value: &str) -> Result<(), CapabilityError> {
    let url = url::Url::parse(value).map_err(CapabilityError::new)?;
    if url.scheme() != "https"
        || url.username() != ""
        || url.password().is_some()
        || url.port().is_some()
        || !matches!(
            url.host_str(),
            Some("www.youtube.com" | "youtube.com" | "m.youtube.com" | "youtu.be")
        )
    {
        return Err(CapabilityError::message(
            "expected an HTTPS YouTube video URL",
        ));
    }
    if url.host_str() == Some("youtu.be") && url.path().trim_matches('/').is_empty() {
        return Err(CapabilityError::message("YouTube video ID missing"));
    }
    if url.host_str() != Some("youtu.be")
        && !(url.path() == "/watch" && url.query_pairs().any(|(k, v)| k == "v" && !v.is_empty())
            || url.path().starts_with("/shorts/")
            || url.path().starts_with("/live/"))
    {
        return Err(CapabilityError::message(
            "expected one video, not a playlist/channel URL",
        ));
    }
    Ok(())
}
impl YouTubeMedia {
    pub async fn inspect(&self, url: &str) -> Result<Value, CapabilityError> {
        validate_youtube_url(url)?;
        let out = tokio::time::timeout(
            Duration::from_secs(120),
            Command::new(&self.executable)
                .args([
                    "--ignore-config",
                    "--no-playlist",
                    "--no-warnings",
                    "--dump-single-json",
                    "--skip-download",
                    "--",
                    url,
                ])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(CapabilityError::new)?
        .map_err(CapabilityError::new)?;
        if !out.status.success() {
            return Err(CapabilityError::message(format!(
                "yt-dlp metadata failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        let v: Value = serde_json::from_slice(&out.stdout).map_err(CapabilityError::new)?;
        Ok(
            json!({"id":v["id"],"title":v["title"],"channel":v["channel"],"duration":v["duration"],"upload_date":v["upload_date"],"original_url":url,"webpage_url":v["webpage_url"]}),
        )
    }
    pub async fn download(&self, url: &str) -> Result<ImportBundle, CapabilityError> {
        let metadata = self.inspect(url).await?;
        let id = metadata["id"]
            .as_str()
            .ok_or_else(|| CapabilityError::message("video has no stable ID"))?;
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(CapabilityError::message("invalid video ID in metadata"));
        }
        let dir = self.root.join(id);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(CapabilityError::new)?;
        let lease = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("import.lock"))
            .map_err(CapabilityError::new)?;
        fs2::FileExt::try_lock_exclusive(&lease).map_err(|_| {
            CapabilityError::message(
                "another import owns this video's download; retry after it finishes",
            )
        })?;
        let output = dir.join("source.%(ext)s");
        let status = tokio::time::timeout(
            Duration::from_secs(3600),
            Command::new(&self.executable)
                .args([
                    "--ignore-config",
                    "--no-playlist",
                    "--no-overwrites",
                    "--no-progress",
                    "-f",
                    "bestaudio/best",
                    "-x",
                    "--audio-format",
                    "m4a",
                    "-o",
                ])
                .arg(output)
                .args(["--", url])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .kill_on_drop(true)
                .status(),
        )
        .await
        .map_err(CapabilityError::new)?
        .map_err(CapabilityError::new)?;
        if !status.success() {
            return Err(CapabilityError::message(format!(
                "yt-dlp download failed: {status}"
            )));
        }
        let path = dir.join("source.m4a");
        let mut bundle = crate::artifacts::MacParakeetArtifacts
            .import_file(Path::new(&path))
            .await?;
        let source_key = format!("youtube:{id}");
        let recording_id = parakeet_domain::stable_id("recording", &source_key);
        bundle.recording.id = recording_id.clone();
        bundle.recording.source_key = source_key;
        bundle.recording.source_kind = "youtube".into();
        bundle.recording.source_title = metadata["title"].as_str().unwrap_or(id).into();
        bundle.recording.metadata = metadata.clone();
        bundle.original = metadata.clone();
        for t in &mut bundle.tracks {
            t.recording_id = recording_id.clone();
            t.id = parakeet_domain::stable_id("track", &format!("{recording_id}:file"));
            t.provenance["ownership"] = json!("application_owned");
        }
        tokio::fs::write(
            dir.join("metadata.json"),
            serde_json::to_vec_pretty(&metadata).map_err(CapabilityError::new)?,
        )
        .await
        .map_err(CapabilityError::new)?;
        Ok(bundle)
    }
}

#[async_trait::async_trait]
impl parakeet_application::MediaImport for YouTubeMedia {
    async fn inspect(&self, url: &str) -> Result<Value, CapabilityError> {
        YouTubeMedia::inspect(self, url).await
    }
    async fn download(&self, url: &str) -> Result<ImportBundle, CapabilityError> {
        YouTubeMedia::download(self, url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restricts_video_urls() {
        assert!(validate_youtube_url("https://youtu.be/abc").is_ok());
        assert!(validate_youtube_url("https://www.youtube.com/watch?v=abc").is_ok());
        for s in [
            "file:///etc/passwd",
            "https://youtube.com.evil.test/watch?v=x",
            "https://youtube.com/playlist?list=x",
            "https://user:pass@youtube.com/watch?v=x",
        ] {
            assert!(validate_youtube_url(s).is_err());
        }
    }
}
