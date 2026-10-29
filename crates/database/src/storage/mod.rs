use super::*;
use parakeet_application::storage::*;
#[async_trait::async_trait]
impl StorageQueries for SqliteStore {
    async fn storage_audio(&self) -> Result<Vec<AudioStorageReference>, PersistenceError> {
        audio(&mut *self.pool.acquire().await.map_err(PersistenceError::new)?).await
    }
}
pub(crate) async fn audio(
    conn: &mut sqlx::SqliteConnection,
) -> Result<Vec<AudioStorageReference>, PersistenceError> {
    Ok(sqlx::query_file!("src/storage/audio.sql")
        .fetch_all(conn)
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
