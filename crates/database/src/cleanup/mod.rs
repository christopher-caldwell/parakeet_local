use super::*;
use parakeet_application::cleanup::*;
pub struct SqliteCleanupUnit {
    transaction: sqlx::Transaction<'static, sqlx::Sqlite>,
}
#[async_trait::async_trait]
impl CleanupUnit for SqliteCleanupUnit {
    async fn commit(self) -> Result<(), PersistenceError> {
        self.transaction
            .commit()
            .await
            .map_err(PersistenceError::new)
    }
}
#[async_trait::async_trait]
impl CleanupTransactions for SqliteStore {
    type Unit = SqliteCleanupUnit;
    async fn begin(&self) -> Result<Self::Unit, PersistenceError> {
        Ok(SqliteCleanupUnit {
            transaction: self
                .pool
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(PersistenceError::new)?,
        })
    }
}
#[async_trait::async_trait]
impl CleanupQueries for SqliteStore {
    async fn cleanup_review(&self, id: &str) -> Result<Option<CleanupReview>, PersistenceError> {
        sqlx::query_file!("src/cleanup/show.sql", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .map(|r| decode(&r.data))
            .transpose()
    }
    async fn cleanup_reviews(&self) -> Result<Vec<CleanupReview>, PersistenceError> {
        sqlx::query_file!("src/cleanup/list.sql")
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .into_iter()
            .map(|r| decode(&r.data))
            .collect()
    }
}
#[async_trait::async_trait]
impl CleanupStore<SqliteCleanupUnit> for SqliteStore {
    async fn context(
        &self,
        unit: &mut SqliteCleanupUnit,
    ) -> Result<CleanupContext, PersistenceError> {
        let conn = &mut *unit.transaction;
        Ok(CleanupContext {
            policy: storage::policy(conn).await?,
            audio: storage::audio(conn).await?,
            runs: storage::runs(conn).await?,
        })
    }
    async fn load(
        &self,
        unit: &mut SqliteCleanupUnit,
        id: &str,
    ) -> Result<Option<CleanupReview>, PersistenceError> {
        sqlx::query_file!("src/cleanup/show.sql", id)
            .fetch_optional(&mut *unit.transaction)
            .await
            .map_err(PersistenceError::new)?
            .map(|r| decode(&r.data))
            .transpose()
    }
    async fn save(
        &self,
        unit: &mut SqliteCleanupUnit,
        review: &CleanupReview,
    ) -> Result<(), PersistenceError> {
        let data = encode(review)?;
        sqlx::query_file!("src/cleanup/save.sql", review.id, data, review.updated_at)
            .execute(&mut *unit.transaction)
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
}
