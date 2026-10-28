use super::*;
use parakeet_application::analysis::{MeetingAnalysis, MeetingAnalysisWrites};
#[async_trait::async_trait]
impl MeetingAnalysisWrites for SqliteStore {
    async fn replace_analysis(
        &self,
        expected: Option<&MeetingAnalysis>,
        next: &MeetingAnalysis,
    ) -> Result<bool, PersistenceError> {
        next.validate().map_err(PersistenceError::new)?;
        if expected.is_some_and(|p| p.recording_id != next.recording_id) {
            return Err(PersistenceError::new(std::io::Error::other(
                "analysis cannot change recording identity",
            )));
        }
        let prior = expected
            .map(serde_json::to_value)
            .transpose()
            .map_err(PersistenceError::new)?
            .as_ref()
            .map(encode)
            .transpose()?;
        let data = encode(&serde_json::to_value(next).map_err(PersistenceError::new)?)?;
        Ok(
            sqlx::query_file!("src/analysis/save.sql", next.recording_id, prior, data)
                .execute(&self.pool)
                .await
                .map_err(PersistenceError::new)?
                .rows_affected()
                == 1,
        )
    }
}
