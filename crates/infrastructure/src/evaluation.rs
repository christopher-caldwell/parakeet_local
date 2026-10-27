use async_trait::async_trait;
use parakeet_application::{CapabilityError, evaluation::*};
use parakeet_domain::SpeakerInterval;
use serde_json::Value;
use std::path::{Path, PathBuf};
#[derive(Clone)]
pub struct EvaluationFiles {
    pub root: PathBuf,
}
async fn bounded_input(path: &Path) -> Result<Vec<u8>, CapabilityError> {
    use tokio::io::AsyncReadExt;
    const LIMIT: usize = 8 * 1024 * 1024;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(CapabilityError::new)?;
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(CapabilityError::new)?;
    if bytes.len() > LIMIT {
        return Err(CapabilityError::message("scoring input exceeds 8 MiB"));
    }
    Ok(bytes)
}
#[async_trait]
impl parakeet_application::scoring::ScoringInputReader for EvaluationFiles {
    async fn speaker_transcript(
        &self,
        path: &Path,
    ) -> Result<parakeet_application::scoring::SpeakerTranscriptEvidence, CapabilityError> {
        use sha2::{Digest, Sha256};
        let bytes = bounded_input(path).await?;
        Ok(parakeet_application::scoring::SpeakerTranscriptEvidence {
            transcript: serde_json::from_slice(&bytes).map_err(CapabilityError::new)?,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        })
    }
    async fn intervals(&self, path: &Path) -> Result<Vec<SpeakerInterval>, CapabilityError> {
        serde_json::from_slice(&bounded_input(path).await?).map_err(CapabilityError::new)
    }
}
#[async_trait]
impl EvaluationArtifacts for EvaluationFiles {
    async fn load_suite(&self, path: &Path) -> Result<EvaluationSuite, CapabilityError> {
        serde_json::from_slice(&tokio::fs::read(path).await.map_err(CapabilityError::new)?)
            .map_err(CapabilityError::new)
    }
    async fn reference(&self, path: &Path) -> Result<Vec<SpeakerInterval>, CapabilityError> {
        serde_json::from_slice(&tokio::fs::read(path).await.map_err(CapabilityError::new)?)
            .map_err(CapabilityError::new)
    }
    async fn fingerprint(&self, path: &Path) -> Result<String, CapabilityError> {
        let path = path.to_owned();
        tokio::task::spawn_blocking(move || crate::models::hash_file(&path))
            .await
            .map_err(CapabilityError::new)?
    }
    async fn save_report(&self, id: &str, report: &Value) -> Result<String, CapabilityError> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(CapabilityError::new)?;
        let path = self.root.join(format!("{id}.json"));
        let temp = path.with_extension("json.partial");
        tokio::fs::write(
            &temp,
            serde_json::to_vec_pretty(report).map_err(CapabilityError::new)?,
        )
        .await
        .map_err(CapabilityError::new)?;
        tokio::fs::rename(&temp, &path)
            .await
            .map_err(CapabilityError::new)?;
        Ok(path.display().to_string())
    }
    async fn report(&self, id: &str) -> Result<Value, CapabilityError> {
        uuid::Uuid::parse_str(id).map_err(CapabilityError::new)?;
        serde_json::from_slice(
            &tokio::fs::read(self.root.join(format!("{id}.json")))
                .await
                .map_err(CapabilityError::new)?,
        )
        .map_err(CapabilityError::new)
    }
}
