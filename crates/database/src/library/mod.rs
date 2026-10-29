use super::*;
use async_trait::async_trait;
#[async_trait]
impl parakeet_application::audio_locations::AudioLocationWrites for SqliteStore {
    async fn replace_locator(
        &self,
        expected: &AudioTrack,
        replacement: &AudioTrack,
    ) -> Result<bool, PersistenceError> {
        if expected.id != replacement.id || expected.recording_id != replacement.recording_id {
            return Err(PersistenceError::new(std::io::Error::other(
                "relink cannot change source identity",
            )));
        }
        let old = encode(expected)?;
        let next = encode(replacement)?;
        Ok(
            sqlx::query_file!("src/library/relink.sql", expected.id, old, next)
                .execute(&self.pool)
                .await
                .map_err(PersistenceError::new)?
                .rows_affected()
                == 1,
        )
    }
}
struct RecordingRow {
    id: String,
    source_key: String,
    source_kind: String,
    source_title: String,
    title_override: Option<String>,
    created_at: String,
    duration_ms: Option<i64>,
    metadata: String,
}
impl RecordingRow {
    fn domain(self) -> Result<Recording, PersistenceError> {
        Ok(Recording {
            id: self.id,
            source_key: self.source_key,
            source_kind: self.source_kind,
            source_title: self.source_title,
            title_override: self.title_override,
            created_at: self.created_at,
            duration_ms: self.duration_ms,
            metadata: decode(&self.metadata)?,
        })
    }
}
#[async_trait]
impl LibraryQueries for SqliteStore {
    async fn list(&self, search: Option<&str>) -> Result<Vec<Recording>, PersistenceError> {
        sqlx::query_file_as!(RecordingRow, "src/library/list.sql", search)
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .into_iter()
            .map(RecordingRow::domain)
            .collect()
    }
    async fn detail(&self, id: &str) -> Result<Option<RecordingDetail>, PersistenceError> {
        let mut tx = self.pool.begin().await.map_err(PersistenceError::new)?;
        let Some(recording) = sqlx::query_file_as!(RecordingRow, "src/library/get.sql", id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(PersistenceError::new)?
        else {
            return Ok(None);
        };
        let tracks = sqlx::query_file!("src/library/tracks.sql", id)
            .fetch_all(&mut *tx)
            .await
            .map_err(PersistenceError::new)?
            .iter()
            .map(|r| decode(&r.data))
            .collect::<Result<_, _>>()?;
        let segments = sqlx::query_file!("src/library/segments.sql", id)
            .fetch_all(&mut *tx)
            .await
            .map_err(PersistenceError::new)?
            .iter()
            .map(|r| decode(&r.data))
            .collect::<Result<_, _>>()?;
        let runs = sqlx::query_file!("src/runs/list.sql", id)
            .fetch_all(&mut *tx)
            .await
            .map_err(PersistenceError::new)?
            .iter()
            .map(|r| decode(&r.data))
            .collect::<Result<_, _>>()?;
        let corrections = sqlx::query_file!("src/library/corrections.sql", id)
            .fetch_all(&mut *tx)
            .await
            .map_err(PersistenceError::new)?
            .iter()
            .map(|r| {
                let mut c: Correction = decode(&r.data)?;
                c.profile_id = r.profile_id.clone();
                Ok(c)
            })
            .collect::<Result<_, PersistenceError>>()?;
        let detail = RecordingDetail {
            recording: recording.domain()?,
            tracks,
            segments,
            runs,
            corrections,
            attributions: vec![],
            word_attributions: vec![],
            source_attributions: vec![],
            unmapped_corrected_segments: vec![],
        };
        tx.commit().await.map_err(PersistenceError::new)?;
        Ok(Some(detail))
    }
}
#[async_trait]
impl LibraryWrites for SqliteStore {
    async fn import(&self, b: &ImportBundle) -> Result<bool, PersistenceError> {
        let r = &b.recording;
        let metadata = encode(&r.metadata)?;
        let original = encode(&b.original)?;
        // One logical import publication: no partially imported recording is visible.
        let mut tx = self.pool.begin().await.map_err(PersistenceError::new)?;
        let result = sqlx::query_file!(
            "src/library/import.sql",
            r.id,
            r.source_key,
            r.source_kind,
            r.source_title,
            r.title_override,
            r.created_at,
            r.duration_ms,
            metadata,
            original
        )
        .execute(&mut *tx)
        .await
        .map_err(PersistenceError::new)?;
        if result.rows_affected() == 0 {
            tx.rollback().await.map_err(PersistenceError::new)?;
            return Ok(false);
        }
        for t in &b.tracks {
            let data = encode(t)?;
            sqlx::query_file!("src/library/track.sql", t.id, t.recording_id, data)
                .execute(&mut *tx)
                .await
                .map_err(PersistenceError::new)?;
        }
        for s in &b.segments {
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
        Ok(true)
    }
    async fn rename(&self, id: &str, title: &str) -> Result<bool, PersistenceError> {
        Ok(sqlx::query_file!("src/library/rename.sql", id, title)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .rows_affected()
            > 0)
    }
}
