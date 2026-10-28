use super::*;
use parakeet_application::storage::*;
use parakeet_domain::storage::RetentionPolicy;
#[async_trait::async_trait]
impl StorageQueries for SqliteStore {
    async fn storage_runs(&self) -> Result<Vec<StorageRunFact>, PersistenceError> {
        Ok(sqlx::query_file!("src/storage/runs.sql")
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .into_iter()
            .map(|r| StorageRunFact {
                id: r.id,
                recording_id: r.recording_id,
                status: r.status,
                ownership_verified: r.ownership_verified,
                completed_at: r.completed_at,
                pending_meeting: r.pending_meeting,
                speaker_reference: r.speaker_reference,
                manual_correction: r.manual_correction,
            })
            .collect())
    }
    async fn storage_audio(&self) -> Result<Vec<AudioStorageReference>, PersistenceError> {
        Ok(sqlx::query_file!("src/storage/audio.sql")
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .into_iter()
            .map(|r| AudioStorageReference {
                track_id: r.id,
                recording_id: r.recording_id,
                path: r.path,
            })
            .collect())
    }
    async fn retention_policy(&self) -> Result<RetentionPolicySnapshot, PersistenceError> {
        let row = sqlx::query_file!("src/storage/policy.sql")
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .ok_or_else(|| {
                PersistenceError::new(std::io::Error::other(
                    "storage policy is missing; run explicit application database migrations",
                ))
            })?;
        Ok(RetentionPolicySnapshot {
            policy: decode(&row.data)?,
            revision: row.revision as u64,
            updated_at: row.updated_at,
        })
    }
}
#[async_trait::async_trait]
impl StoragePolicyWrites for SqliteStore {
    async fn save_retention_policy(
        &self,
        policy: &RetentionPolicy,
        expected_revision: u64,
        at: &str,
    ) -> Result<Option<RetentionPolicySnapshot>, PersistenceError> {
        let data = encode(policy)?;
        let expected = i64::try_from(expected_revision).map_err(PersistenceError::new)?;
        sqlx::query_file!("src/storage/save_policy.sql", data, at, expected)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .map(|row| {
                Ok(RetentionPolicySnapshot {
                    policy: decode(&row.data)?,
                    revision: row.revision as u64,
                    updated_at: row.updated_at,
                })
            })
            .transpose()
    }
}
