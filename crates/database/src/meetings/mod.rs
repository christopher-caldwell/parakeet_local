use super::*;
use async_trait::async_trait;
use parakeet_application::meeting::{MeetingFinalization, MeetingFinalizationWrites};
#[async_trait]
impl MeetingFinalizationWrites for SqliteStore {
    async fn replace_finalization(
        &self,
        id: &str,
        expected: Option<&MeetingFinalization>,
        next: &MeetingFinalization,
    ) -> Result<bool, PersistenceError> {
        if next.recording_id != id
            || expected.is_some_and(|p| p.id != next.id || p.recording_id != id)
        {
            return Err(PersistenceError::new(std::io::Error::other(
                "finalization cannot change identity",
            )));
        }
        next.validate().map_err(PersistenceError::new)?;
        // Value normalization matches the metadata representation used by atomic import.
        let prior = expected
            .map(serde_json::to_value)
            .transpose()
            .map_err(PersistenceError::new)?;
        let prior = prior.as_ref().map(encode).transpose()?;
        let data = encode(&serde_json::to_value(next).map_err(PersistenceError::new)?)?;
        Ok(
            sqlx::query_file!("src/meetings/finalization.sql", id, prior, data)
                .execute(&self.pool)
                .await
                .map_err(PersistenceError::new)?
                .rows_affected()
                == 1,
        )
    }
}
