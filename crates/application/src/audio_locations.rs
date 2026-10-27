use super::*;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFileIdentity {
    pub path: String,
    pub size_bytes: u64,
    pub modified_unix_ms: u64,
    pub sha256: String,
}
#[async_trait]
pub trait AudioFileIdentityReader: Send + Sync {
    async fn identify(&self, path: &Path) -> Result<AudioFileIdentity, CapabilityError>;
}
#[async_trait]
pub trait AudioLocationWrites: Send + Sync {
    async fn replace_locator(
        &self,
        expected: &AudioTrack,
        replacement: &AudioTrack,
    ) -> Result<bool, PersistenceError>;
}
#[derive(Debug, Clone)]
pub struct FingerprintAudioInput {
    pub recording_id: String,
    pub track_id: String,
}
#[derive(Debug, Clone)]
pub struct RelinkAudioInput {
    pub recording_id: String,
    pub track_id: String,
    pub path: PathBuf,
    pub dry_run: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioRelinkReport {
    pub track: AudioTrack,
    pub dry_run: bool,
    pub changed: bool,
    pub verification: String,
}
operation_error!(FingerprintAudioError,sources{Persistence:PersistenceError=>"persistence",Source:CapabilityError=>"source"},values{NotFound=>"not_found",Conflict=>"conflict"});
operation_error!(RelinkAudioError,sources{Persistence:PersistenceError=>"persistence",Source:CapabilityError=>"source"},values{NotFound=>"not_found",Conflict=>"conflict",Unverifiable=>"source"});
pub struct AudioLocationService {
    queries: Arc<dyn LibraryQueries>,
    writes: Arc<dyn AudioLocationWrites>,
    reader: Arc<dyn AudioFileIdentityReader>,
}
impl AudioLocationService {
    pub fn new(
        queries: Arc<dyn LibraryQueries>,
        writes: Arc<dyn AudioLocationWrites>,
        reader: Arc<dyn AudioFileIdentityReader>,
    ) -> Self {
        Self {
            queries,
            writes,
            reader,
        }
    }
    async fn track(
        &self,
        recording: &str,
        id: &str,
    ) -> Result<Option<AudioTrack>, PersistenceError> {
        Ok(self
            .queries
            .detail(recording)
            .await?
            .and_then(|d| d.tracks.into_iter().find(|t| t.id == id)))
    }
    pub async fn fingerprint(
        &self,
        input: FingerprintAudioInput,
    ) -> Result<AudioTrack, FingerprintAudioError> {
        let old = self
            .track(&input.recording_id, &input.track_id)
            .await?
            .ok_or_else(|| FingerprintAudioError::NotFound(input.track_id))?;
        let identity = self.reader.identify(Path::new(&old.path)).await?;
        validate_legacy_fingerprint(&old, &identity).map_err(FingerprintAudioError::Conflict)?;
        if let Some(hash) = old.provenance["content_sha256"].as_str() {
            if hash != identity.sha256 {
                return Err(FingerprintAudioError::Conflict(
                    "source content no longer matches its saved SHA-256".into(),
                ));
            }
            return Ok(old);
        }
        let mut next = old.clone();
        next.provenance["content_sha256"] = json!(identity.sha256);
        next.provenance["identity_established_at"] = json!(now());
        if !self.writes.replace_locator(&old, &next).await? {
            return Err(FingerprintAudioError::Conflict(
                "source locator changed during fingerprinting; inspect and retry".into(),
            ));
        }
        Ok(next)
    }
    pub async fn relink(
        &self,
        input: RelinkAudioInput,
    ) -> Result<AudioRelinkReport, RelinkAudioError> {
        let old = self
            .track(&input.recording_id, &input.track_id)
            .await?
            .ok_or_else(|| RelinkAudioError::NotFound(input.track_id))?;
        let expected = if let Some(hash) = old.provenance["content_sha256"].as_str() {
            hash.to_owned()
        } else {
            let original=self.reader.identify(Path::new(&old.path)).await.map_err(|e|RelinkAudioError::Unverifiable(format!("no saved content hash and original cannot be fingerprinted; refusing to transplant transcripts or voice references: {e}")))?;
            validate_legacy_fingerprint(&old, &original).map_err(RelinkAudioError::Unverifiable)?;
            original.sha256
        };
        let replacement = self.reader.identify(&input.path).await?;
        if replacement.sha256 != expected {
            return Err(RelinkAudioError::Conflict(
                "replacement audio is not byte-identical to the original content hash".into(),
            ));
        }
        let mut next = old.clone();
        next.path = replacement.path;
        next.size_bytes = Some(replacement.size_bytes);
        next.modified_unix_ms = Some(replacement.modified_unix_ms);
        next.provenance["content_sha256"] = json!(expected);
        let changed = next != old;
        if changed {
            let history = next.provenance["locator_history"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut history = history;
            history.push(json!({"path":old.path,"size_bytes":old.size_bytes,"modified_unix_ms":old.modified_unix_ms,"relinked_at":now(),"verification":"sha256_identical"}));
            next.provenance["locator_history"] = json!(history);
            if !input.dry_run && !self.writes.replace_locator(&old, &next).await? {
                return Err(RelinkAudioError::Conflict(
                    "source locator changed while hashing; inspect and retry".into(),
                ));
            }
        }
        Ok(AudioRelinkReport {
            track: next,
            dry_run: input.dry_run,
            changed,
            verification: "sha256_identical".into(),
        })
    }
}
fn validate_legacy_fingerprint(
    track: &AudioTrack,
    identity: &AudioFileIdentity,
) -> Result<(), String> {
    if track.size_bytes != Some(identity.size_bytes)
        || track.modified_unix_ms != Some(identity.modified_unix_ms)
    {
        return Err("original no longer matches imported size/mtime; refusing to establish a new content identity".into());
    }
    Ok(())
}
