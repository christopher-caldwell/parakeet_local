use super::*;
use async_trait::async_trait;
use parakeet_application::jobs::{ProcessingQueries, RecoveryWrites};
use parakeet_application::processing::RunWrites;
#[async_trait]
impl ProcessingQueries for SqliteStore {
    async fn control_state(
        &self,
        id: &str,
    ) -> Result<Option<parakeet_application::processing_control::RunControlState>, PersistenceError>
    {
        Ok(sqlx::query_file!("src/runs/control.sql", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .map(
                |r| parakeet_application::processing_control::RunControlState {
                    status: r.status,
                    cancellation_requested_at: r.cancellation_requested_at,
                },
            ))
    }
    async fn pending_runs(&self) -> Result<Vec<ProcessingRun>, PersistenceError> {
        sqlx::query_file!("src/runs/pending.sql")
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .into_iter()
            .map(|r| decode(&r.data))
            .collect()
    }
    async fn run(&self, id: &str) -> Result<Option<ProcessingRun>, PersistenceError> {
        sqlx::query_file!("src/runs/get.sql", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .map(|r| decode(&r.data))
            .transpose()
    }
}
#[async_trait]
impl RecoveryWrites for SqliteStore {
    async fn interrupt(&self, run: &ProcessingRun) -> Result<bool, PersistenceError> {
        if run.status != "interrupted" || run.completed_at.is_none() {
            return Err(PersistenceError::new(std::io::Error::other(
                "recovery requires a terminal interrupted result",
            )));
        }
        let data = encode(run)?;
        Ok(sqlx::query_file!("src/runs/interrupt.sql", run.id, data)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .rows_affected()
            == 1)
    }
}
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

#[async_trait]
impl parakeet_application::processing_control::RunControlWrites for SqliteStore {
    async fn save_progress(
        &self,
        id: &str,
        p: &parakeet_application::processing_control::RunProgress,
    ) -> Result<bool, PersistenceError> {
        let data = encode(p)?;
        Ok(sqlx::query_file!("src/runs/progress.sql", id, data)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .rows_affected()
            == 1)
    }
    async fn request_cancellation(&self, id: &str, at: &str) -> Result<bool, PersistenceError> {
        Ok(sqlx::query_file!("src/runs/cancel.sql", id, at)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .rows_affected()
            == 1)
    }
}
