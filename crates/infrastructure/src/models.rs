use async_trait::async_trait;
use parakeet_application::{
    CapabilityError, LocalModels, ModelDownloadProgress, ModelInfo, ModelSelection,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::process::Command;

pub const WHISPER_REV: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
pub const SPEAKRS_REV: &str = "67f963b6a1d94fcd4678948237b3cc8bd69bf1d8";
pub const NEMOTRON_REV: &str = "1b0b133f6f8820292010afd776d8f9fbc9fca17e";
pub const SPEAKRS_ID: &str = "speakrs-community-1-coreml";
#[derive(Clone)]
pub struct ModelFiles {
    pub root: PathBuf,
    progress: std::sync::Arc<std::sync::Mutex<Option<ModelDownloadProgress>>>,
}
struct ClearProgress(std::sync::Arc<std::sync::Mutex<Option<ModelDownloadProgress>>>);
pub struct ModelLease(File);
impl Drop for ModelLease {
    fn drop(&mut self) {
        // Explicit unlock also releases a descriptor briefly inherited by a concurrently
        // spawning child before exec closes it. Closing only our copy can retain flock.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}
impl Drop for ClearProgress {
    fn drop(&mut self) {
        if let Ok(mut progress) = self.0.lock() {
            *progress = None;
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct InstalledManifest {
    pub id: String,
    pub revision: String,
    pub files: BTreeMap<String, String>,
    pub bytes: u64,
    pub installed_at: String,
    pub upstream: Value,
}
impl ModelFiles {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            progress: Default::default(),
        }
    }
    pub fn directory(&self, id: &str) -> Result<PathBuf, CapabilityError> {
        catalog(id)?;
        Ok(self.root.join(id))
    }
    /// Called while holding the model lease. Cached runtimes are reusable only
    /// for the same verified revision and exact current weight fingerprints.
    pub fn cache_identity(&self, id: &str) -> Result<String, CapabilityError> {
        let inspected = self.inspect(id, false)?;
        if !inspected.verified {
            return Err(CapabilityError::message(
                "model verification is unavailable or stale; refusing cached inference",
            ));
        }
        let (_, _, _, _, files) = catalog(id)?;
        Ok(format!(
            "{id}:{}:{}",
            inspected.revision,
            file_fingerprints(&self.directory(id)?, &files)
        ))
    }
    pub fn acquire(&self) -> Result<ModelLease, CapabilityError> {
        std::fs::create_dir_all(&self.root).map_err(CapabilityError::new)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(self.root.join("model-access.lock"))
            .map_err(CapabilityError::new)?;
        let metadata = file.metadata().map_err(CapabilityError::new)?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(CapabilityError::message(
                "model lock must be a single-link ordinary file",
            ));
        }
        fs2::FileExt::try_lock_exclusive(&file).map_err(|e| {
            CapabilityError::message(format!(
                "another model install/inference/removal owns the model lease: {e}"
            ))
        })?;
        Ok(ModelLease(file))
    }
    fn inspect(&self, id: &str, full: bool) -> Result<ModelInfo, CapabilityError> {
        let (category, revision, license, _, files) = catalog(id)?;
        let directory = self.directory(id)?;
        let manifest_path = directory.join("installed.json");
        let manifest = std::fs::read(&manifest_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<InstalledManifest>(&b).ok());
        let expected: Value = serde_json::from_str(include_str!("model-checksums.json"))
            .map_err(CapabilityError::new)?;
        let cache = std::fs::read(directory.join("verified.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        let fingerprints = file_fingerprints(&directory, &files);
        let catalog_matches = manifest.as_ref().is_some_and(|m| {
            m.id == id
                && m.revision == revision
                && serde_json::to_value(&m.files).ok().as_ref() == Some(&expected[id]["sha256"])
        });
        let mut verified = catalog_matches
            && cache
                .as_ref()
                .is_some_and(|c| c["revision"] == revision && c["files"] == fingerprints);
        let mut detail = json!({"runtime":if category=="asr"{"whisper-rs 0.16.0 / whisper.cpp"}else if id.starts_with("nemotron3-"){"FluidAudio Nemotron subset 1b0ed50d / CoreML CPU+GPU"}else if id.ends_with("-cpu"){"speakrs 0.7.0 / native CPU"}else{"speakrs 0.7.0 / CoreML"},"offline_processing":true,"required_files":files.len(),"verification_basis":if full{"sha256_against_pinned_catalog"}else{"cached_sha256_plus_file_size_and_mtime"},"provides_voice_embeddings":id==SPEAKRS_ID||id=="speakrs-community-1-cpu","speaker_capacity":if id.starts_with("nemotron3-"){Some(8)}else{None}});
        if full {
            // A failed explicit check must invalidate an older verification receipt.
            match std::fs::remove_file(directory.join("verified.json")) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(CapabilityError::new(error)),
            }
        }
        if full && catalog_matches {
            verified = true;
            for relative in &files {
                let expected_hash = expected[id]["sha256"][relative].as_str().ok_or_else(|| {
                    CapabilityError::message("model checksum catalog missing file")
                })?;
                if hash_file(&directory.join(relative))? != expected_hash {
                    verified = false;
                    detail["verification_error"] = json!(format!("checksum mismatch: {relative}"));
                    break;
                }
            }
            if verified {
                let cache = json!({"revision":revision,"files":fingerprints,"verified_at":parakeet_application::now()});
                let temporary =
                    directory.join(format!(".verified-{}", parakeet_application::new_id()));
                std::fs::write(
                    &temporary,
                    serde_json::to_vec(&cache).map_err(CapabilityError::new)?,
                )
                .map_err(CapabilityError::new)?;
                std::fs::rename(temporary, directory.join("verified.json"))
                    .map_err(CapabilityError::new)?;
            }
        }
        Ok(ModelInfo {
            experimental_word_timestamps: id.starts_with("whisper-"),
            id: id.into(),
            category: category.into(),
            revision: revision.into(),
            installed: manifest.is_some(),
            verified,
            bytes: manifest
                .as_ref()
                .map(|m| m.bytes)
                .or_else(|| expected[id]["bytes"].as_u64()),
            license: license.into(),
            path: manifest.map(|_| directory.display().to_string()),
            detail,
        })
    }
}
fn catalog(
    id: &str,
) -> Result<
    (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        Vec<String>,
    ),
    CapabilityError,
> {
    match id {
        "whisper-base.en" => Ok((
            "asr",
            WHISPER_REV,
            "MIT",
            "ggerganov/whisper.cpp",
            vec!["ggml-base.en.bin".into()],
        )),
        "whisper-small.en" => Ok((
            "asr",
            WHISPER_REV,
            "MIT",
            "ggerganov/whisper.cpp",
            vec!["ggml-small.en.bin".into()],
        )),
        "speakrs-community-1-cpu" => Ok((
            "diarization",
            SPEAKRS_REV,
            "CC-BY-4.0 upstream Community-1/WeSpeaker; see model attribution",
            "avencera/speakrs-models",
            speakrs::models::required_model_files(speakrs::ExecutionMode::Cpu),
        )),
        "nemotron3-fast128" | "nemotron3-offline" => {
            let preset = if id == "nemotron3-fast128" {
                "fast128"
            } else {
                "offline"
            };
            let stem = format!("monolithic/v2/Nemotron3Diarizer_{preset}.mlmodelc");
            Ok((
                "diarization",
                NEMOTRON_REV,
                "OpenMDW-1.1 (NVIDIA Nemotron 3 weights; FluidInference conversion)",
                "FluidInference/nemotron-3-diarization-coreml",
                vec![
                    "learnable_sil_emb.bin".into(),
                    format!("{stem}/analytics/coremldata.bin"),
                    format!("{stem}/coremldata.bin"),
                    format!("{stem}/model.mil"),
                    format!("{stem}/weights/weight.bin"),
                ],
            ))
        }
        SPEAKRS_ID => Ok((
            "diarization",
            SPEAKRS_REV,
            "CC-BY-4.0 upstream Community-1/WeSpeaker; see model attribution",
            "avencera/speakrs-models",
            speakrs::models::required_model_files(speakrs::ExecutionMode::CoreMl),
        )),
        _ => Err(CapabilityError::message(format!(
            "unsupported local model {id}"
        ))),
    }
}
pub fn hash_file(path: &Path) -> Result<String, CapabilityError> {
    let mut file = File::open(path).map_err(CapabilityError::new)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 131072];
    loop {
        let n = file.read(&mut buffer).map_err(CapabilityError::new)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}
async fn metadata(repo: &str, revision: &str) -> Result<Value, CapabilityError> {
    let url = format!(
        "https://huggingface.co/api/models/{repo}/tree/{revision}?recursive=true&limit=1000"
    );
    let out = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--max-time",
            "60",
            &url,
        ])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(CapabilityError::new)?;
    if !out.status.success() {
        return Err(CapabilityError::message(format!(
            "model metadata request failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    serde_json::from_slice(&out.stdout).map_err(CapabilityError::new)
}
#[async_trait]
impl LocalModels for ModelFiles {
    fn download_progress(&self) -> Option<ModelDownloadProgress> {
        self.progress.lock().ok().and_then(|p| p.clone())
    }
    async fn selection(&self) -> Result<ModelSelection, CapabilityError> {
        let path = self.root.join("selection.json");
        if !path.exists() {
            return Ok(ModelSelection::default());
        }
        serde_json::from_slice(&tokio::fs::read(path).await.map_err(CapabilityError::new)?)
            .map_err(CapabilityError::new)
    }
    async fn select(&self, id: &str) -> Result<(), CapabilityError> {
        let _lease = self.acquire()?;
        let (category, _, _, _, _) = catalog(id)?;
        let mut selected = self.selection().await?;
        if category == "asr" {
            selected.asr = id.into();
        } else {
            selected.diarization = id.into();
        }
        let path = self.root.join("selection.json");
        let temporary = self.root.join("selection.json.partial");
        tokio::fs::write(
            &temporary,
            serde_json::to_vec_pretty(&selected).map_err(CapabilityError::new)?,
        )
        .await
        .map_err(CapabilityError::new)?;
        tokio::fs::rename(&temporary, &path)
            .await
            .map_err(CapabilityError::new)?;
        Ok(())
    }

    async fn select_embedding(&self, id: &str) -> Result<(), CapabilityError> {
        catalog(id)?;
        let _lease = self.acquire()?;
        let mut selection = self.selection().await?;
        selection.embedding = id.into();
        let temporary = self.root.join("selection.json.partial");
        tokio::fs::write(
            &temporary,
            serde_json::to_vec_pretty(&selection).map_err(CapabilityError::new)?,
        )
        .await
        .map_err(CapabilityError::new)?;
        tokio::fs::rename(temporary, self.root.join("selection.json"))
            .await
            .map_err(CapabilityError::new)?;
        Ok(())
    }
    async fn list(&self) -> Result<Vec<ModelInfo>, CapabilityError> {
        let own = self.clone();
        tokio::task::spawn_blocking(move || {
            [
                "whisper-base.en",
                "whisper-small.en",
                SPEAKRS_ID,
                "speakrs-community-1-cpu",
                "nemotron3-fast128",
                "nemotron3-offline",
            ]
            .iter()
            .map(|id| own.inspect(id, false))
            .collect()
        })
        .await
        .map_err(CapabilityError::new)?
    }
    async fn readiness(&self, id: &str) -> Result<ModelInfo, CapabilityError> {
        let own = self.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || {
            let info = own.inspect(&id, false)?;
            if info.installed && !info.verified {
                own.inspect(&id, true)
            } else {
                Ok(info)
            }
        })
        .await
        .map_err(CapabilityError::new)?
    }
    async fn verify(&self, id: &str) -> Result<ModelInfo, CapabilityError> {
        let own = self.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || own.inspect(&id, true))
            .await
            .map_err(CapabilityError::new)?
    }
    async fn install(&self, id: &str) -> Result<ModelInfo, CapabilityError> {
        let _lease = self.acquire()?;
        let _progress_guard = ClearProgress(self.progress.clone());
        let (category, revision, _, repo, files) = catalog(id)?;
        let existing = self.verify(id).await?;
        if existing.verified {
            return Ok(existing);
        }
        let upstream = metadata(repo, revision).await?;
        let directory = self.directory(id)?;
        let stage = self
            .root
            .join(format!(".install-{}", parakeet_application::new_id()));
        std::fs::create_dir(&stage).map_err(CapabilityError::new)?;
        let mut manifest = InstalledManifest {
            id: id.into(),
            revision: revision.into(),
            files: BTreeMap::new(),
            bytes: 0,
            installed_at: parakeet_application::now(),
            upstream: json!({"repository":repo,"revision":revision,"category":category}),
        };
        for (index, relative) in files.iter().enumerate() {
            eprintln!("model {id}: file {}/{} {relative}", index + 1, files.len());
            let target = stage.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(CapabilityError::new)?;
            }
            let partial = target.with_extension("download");
            let url = format!("https://huggingface.co/{repo}/resolve/{revision}/{relative}");
            let mut downloader = Command::new("curl")
                .args([
                    "--fail",
                    "--silent",
                    "--show-error",
                    "--location",
                    "--proto",
                    "=https",
                    "--retry",
                    "2",
                    "--max-time",
                    "1800",
                    "--output",
                ])
                .arg(&partial)
                .arg(&url)
                .stderr(Stdio::inherit())
                .kill_on_drop(true)
                .spawn()
                .map_err(CapabilityError::new)?;
            let expected: Value = serde_json::from_str(include_str!("model-checksums.json"))
                .map_err(CapabilityError::new)?;
            let status = loop {
                if let Some(status) = downloader.try_wait().map_err(CapabilityError::new)? {
                    break status;
                }
                let bytes = tokio::fs::metadata(&partial)
                    .await
                    .ok()
                    .map(|m| m.len())
                    .unwrap_or(0);
                if let Ok(mut progress) = self.progress.lock() {
                    *progress = Some(ModelDownloadProgress {
                        model: id.into(),
                        file: relative.clone(),
                        file_index: index + 1,
                        total_files: files.len(),
                        downloaded_bytes: manifest.bytes + bytes,
                        total_bytes: expected[id]["bytes"].as_u64(),
                    });
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            };
            if !status.success() {
                return Err(CapabilityError::message(format!(
                    "download failed for {relative}; partial install retained in {}",
                    stage.display()
                )));
            }
            let hash = hash_file(&partial)?;
            let catalog: Value = serde_json::from_str(include_str!("model-checksums.json"))
                .map_err(CapabilityError::new)?;
            if catalog[id]["sha256"][relative].as_str() != Some(&hash) {
                return Err(CapabilityError::message(format!(
                    "pinned catalog checksum mismatch: {relative}"
                )));
            }
            if let Some(expected) = upstream
                .as_array()
                .and_then(|a| a.iter().find(|f| f["path"].as_str() == Some(relative)))
                .and_then(|f| f["lfs"]["oid"].as_str())
                && hash != expected
            {
                return Err(CapabilityError::message(format!(
                    "upstream checksum mismatch for {relative}"
                )));
            }
            manifest.bytes += std::fs::metadata(&partial)
                .map_err(CapabilityError::new)?
                .len();
            manifest.files.insert(relative.clone(), hash);
            std::fs::rename(&partial, &target).map_err(CapabilityError::new)?;
        }
        std::fs::write(
            stage.join("installed.json"),
            serde_json::to_vec_pretty(&manifest).map_err(CapabilityError::new)?,
        )
        .map_err(CapabilityError::new)?;
        if directory.exists() {
            return Err(CapabilityError::message(format!(
                "existing invalid model directory {}; remove deliberately before reinstalling; validated staging retained",
                directory.display()
            )));
        }
        std::fs::rename(&stage, &directory).map_err(CapabilityError::new)?;
        self.verify(id).await
    }
    async fn remove(&self, id: &str) -> Result<(), CapabilityError> {
        let _lease = self.acquire()?;
        let directory = self.directory(id)?;
        if directory.exists() {
            std::fs::remove_dir_all(directory).map_err(CapabilityError::new)?;
        }
        Ok(())
    }
}

fn file_fingerprints(directory: &Path, files: &[String]) -> Value {
    let mut out = serde_json::Map::new();
    for name in files {
        if let Ok(meta) = std::fs::metadata(directory.join(name)) {
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos().to_string());
            out.insert(
                name.clone(),
                json!({"bytes":meta.len(),"modified_unix_ns":modified}),
            );
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn failed_hash_check_invalidates_old_verification_receipt() {
        let temp = tempfile::tempdir().unwrap();
        let models = ModelFiles::new(temp.path().join("models"));
        let directory = models.directory("whisper-base.en").unwrap();
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("ggml-base.en.bin"), b"bad model").unwrap();
        let expected: Value = serde_json::from_str(include_str!("model-checksums.json")).unwrap();
        let files: BTreeMap<String, String> =
            serde_json::from_value(expected["whisper-base.en"]["sha256"].clone()).unwrap();
        let manifest = InstalledManifest {
            id: "whisper-base.en".into(),
            revision: WHISPER_REV.into(),
            files,
            bytes: 9,
            installed_at: "fixture".into(),
            upstream: Value::Null,
        };
        std::fs::write(
            directory.join("installed.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let (_, _, _, _, required) = catalog("whisper-base.en").unwrap();
        let receipt =
            json!({"revision":WHISPER_REV,"files":file_fingerprints(&directory,&required)});
        std::fs::write(
            directory.join("verified.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        assert!(!models.verify("whisper-base.en").await.unwrap().verified);
        assert!(!directory.join("verified.json").exists());
        assert!(models.cache_identity("whisper-base.en").is_err());
        assert!(!models.inspect("whisper-base.en", false).unwrap().verified);
    }
    #[test]
    fn model_lease_and_model_identity_prevent_unsafe_overlap_or_paths() {
        let temp = tempfile::tempdir().unwrap();
        let models = ModelFiles::new(temp.path().join("models"));
        assert!(models.directory("../source").is_err());
        let lease = models.acquire().unwrap();
        assert!(models.acquire().is_err());
        drop(lease);
        assert!(models.acquire().is_ok());
    }
}
