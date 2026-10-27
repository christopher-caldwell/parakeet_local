use super::*;
use async_trait::async_trait;
use parakeet_application::processing::RunWrites;
#[async_trait]
impl RunWrites for SqliteStore {
    async fn start(&self, run: &ProcessingRun) -> Result<(), PersistenceError> {
        let data = encode(run)?;
        sqlx::query_file!(
            "src/runs/start.sql",
            run.id,
            run.recording_id,
            run.kind,
            run.status,
            data
        )
        .execute(&self.pool)
        .await
        .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn finish(
        &self,
        run: &ProcessingRun,
        segments: &[TranscriptSegment],
    ) -> Result<(), PersistenceError> {
        let mut tx = self.pool.begin().await.map_err(PersistenceError::new)?;
        let data = encode(run)?;
        let result = sqlx::query_file!("src/runs/finish.sql", run.id, run.status, data)
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        if result.rows_affected() != 1 {
            return Err(PersistenceError::new(std::io::Error::other(
                "run has already settled or is missing",
            )));
        }
        for s in segments {
            let data = encode(s)?;
            let ordinal = s.ordinal as i64;
            sqlx::query_file!(
                "src/library/segment.sql",
                s.id,
                s.recording_id,
                s.run_id,
                s.start_ms,
                ordinal,
                data
            )
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        }
        tx.commit().await.map_err(PersistenceError::new)?;
        Ok(())
    }
}
