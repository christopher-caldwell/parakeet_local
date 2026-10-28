use super::*;
use async_trait::async_trait;
use parakeet_application::speakers::{
    ClusterIdentification, SpeakerQueries, SpeakerSuggestion, SpeakerWrites, SuggestionResolution,
};
#[async_trait]
impl SpeakerQueries for SqliteStore {
    async fn notes(&self, profile_id: &str) -> Result<String, PersistenceError> {
        Ok(sqlx::query_file!("src/speakers/notes.sql", profile_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .map(|r| r.notes)
            .unwrap_or_default())
    }
    async fn identification(
        &self,
        id: &str,
    ) -> Result<Option<ClusterIdentification>, PersistenceError> {
        sqlx::query_file!("src/speakers/identification.sql", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .map(|r| decode(&r.data))
            .transpose()
    }
    async fn identifications(
        &self,
        recording: Option<&str>,
    ) -> Result<Vec<ClusterIdentification>, PersistenceError> {
        sqlx::query_file!("src/speakers/identifications.sql", recording)
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .iter()
            .map(|r| decode(&r.data))
            .collect()
    }
    async fn suggestion(&self, id: &str) -> Result<Option<SpeakerSuggestion>, PersistenceError> {
        let row = sqlx::query_file!("src/speakers/suggestion.sql", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?;
        if let Some(row) = row {
            let mut suggestion: SpeakerSuggestion = decode(&row.data)?;
            suggestion.resolutions = self.suggestion_resolutions(id).await?;
            Ok(Some(suggestion))
        } else {
            Ok(None)
        }
    }
    async fn suggestions(
        &self,
        recording: Option<&str>,
    ) -> Result<Vec<SpeakerSuggestion>, PersistenceError> {
        let rows = sqlx::query_file!("src/speakers/suggestions.sql", recording)
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?;
        let mut output = vec![];
        for row in rows {
            let mut suggestion: SpeakerSuggestion = decode(&row.data)?;
            suggestion.resolutions = self.suggestion_resolutions(&suggestion.id).await?;
            output.push(suggestion);
        }
        Ok(output)
    }
    async fn profiles(&self) -> Result<Vec<SpeakerProfile>, PersistenceError> {
        Ok(sqlx::query_file!("src/speakers/list.sql")
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .into_iter()
            .map(|r| SpeakerProfile {
                id: r.id,
                name: r.name,
                created_at: r.created_at,
            })
            .collect())
    }
    async fn examples(&self, profile: Option<&str>) -> Result<Vec<VoiceExample>, PersistenceError> {
        sqlx::query_file!("src/speakers/examples.sql", profile)
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .iter()
            .map(|r| decode(&r.data))
            .collect()
    }
    async fn segment(&self, id: &str) -> Result<Option<TranscriptSegment>, PersistenceError> {
        sqlx::query_file!("src/speakers/segment.sql", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .map(|r| decode(&r.data))
            .transpose()
    }
    async fn corrections(&self, id: &str) -> Result<Vec<Correction>, PersistenceError> {
        sqlx::query_file!("src/speakers/history.sql", id)
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .iter()
            .map(|r| {
                let mut c: Correction = decode(&r.data)?;
                c.profile_id = r.profile_id.clone();
                Ok(c)
            })
            .collect()
    }
}
#[async_trait]
impl SpeakerWrites for SqliteStore {
    async fn save_suggestion(&self, s: &SpeakerSuggestion) -> Result<(), PersistenceError> {
        let data = encode(s)?;
        sqlx::query_file!(
            "src/speakers/save_suggestion.sql",
            s.id,
            s.recording_id,
            data
        )
        .execute(&self.pool)
        .await
        .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn resolve_suggestion(
        &self,
        r: &SuggestionResolution,
        expected_revision: usize,
        correction: Option<&Correction>,
        expected_correction_revision: usize,
    ) -> Result<bool, PersistenceError> {
        let mut tx = self.pool.begin().await.map_err(PersistenceError::new)?;
        let data = encode(r)?;
        let expected_revision = expected_revision as i64;
        let result = sqlx::query_file!(
            "src/speakers/resolve_suggestion.sql",
            r.id,
            r.suggestion_id,
            r.profile_id,
            data,
            expected_revision
        )
        .execute(&mut *tx)
        .await
        .map_err(PersistenceError::new)?;
        if result.rows_affected() != 1 {
            return Ok(false);
        }
        if let Some(c) = correction {
            let data = encode(c)?;
            let expected = expected_correction_revision as i64;
            let result = sqlx::query_file!(
                "src/speakers/correct.sql",
                c.id,
                c.segment_id,
                c.profile_id,
                data,
                expected
            )
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
            if result.rows_affected() != 1 {
                return Ok(false);
            }
        }
        tx.commit().await.map_err(PersistenceError::new)?;
        Ok(true)
    }
    async fn save_identification(
        &self,
        identification: &ClusterIdentification,
        corrections: &[(Correction, usize)],
        examples: &[VoiceExample],
    ) -> Result<bool, PersistenceError> {
        let mut tx = self.pool.begin().await.map_err(PersistenceError::new)?;
        let data = encode(identification)?;
        sqlx::query_file!(
            "src/speakers/save_identification.sql",
            identification.id,
            identification.recording_id,
            identification.profile_id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(PersistenceError::new)?;
        for (c, expected) in corrections {
            if !append_correction(&mut tx, c, *expected).await? {
                return Ok(false);
            }
        }
        for e in examples {
            let data = encode(e)?;
            sqlx::query_file!("src/speakers/enroll.sql", e.id, e.profile_id, data)
                .execute(&mut *tx)
                .await
                .map_err(PersistenceError::new)?;
        }
        tx.commit().await.map_err(PersistenceError::new)?;
        Ok(true)
    }
    async fn undo_identification(
        &self,
        identification: &ClusterIdentification,
        corrections: &[(Correction, usize)],
        example_ids: &[String],
    ) -> Result<bool, PersistenceError> {
        let mut tx = self.pool.begin().await.map_err(PersistenceError::new)?;
        let data = encode(identification)?;
        let updated = sqlx::query_file!(
            "src/speakers/undo_identification.sql",
            identification.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(PersistenceError::new)?;
        if updated.rows_affected() != 1 {
            return Ok(false);
        }
        for (c, expected) in corrections {
            if !append_correction(&mut tx, c, *expected).await? {
                return Ok(false);
            }
        }
        for id in example_ids {
            sqlx::query_file!("src/speakers/remove_example.sql", id)
                .execute(&mut *tx)
                .await
                .map_err(PersistenceError::new)?;
        }
        tx.commit().await.map_err(PersistenceError::new)?;
        Ok(true)
    }
    async fn save_notes(&self, profile_id: &str, notes: &str) -> Result<bool, PersistenceError> {
        Ok(
            sqlx::query_file!("src/speakers/save_notes.sql", profile_id, notes)
                .execute(&self.pool)
                .await
                .map_err(PersistenceError::new)?
                .rows_affected()
                > 0,
        )
    }
    async fn create_profile(&self, p: &SpeakerProfile) -> Result<(), PersistenceError> {
        sqlx::query_file!("src/speakers/create.sql", p.id, p.name, p.created_at)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn rename_profile(&self, id: &str, name: &str) -> Result<bool, PersistenceError> {
        Ok(sqlx::query_file!("src/speakers/rename.sql", id, name)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .rows_affected()
            > 0)
    }
    async fn delete_profile(&self, id: &str) -> Result<bool, PersistenceError> {
        Ok(sqlx::query_file!("src/speakers/delete.sql", id)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .rows_affected()
            > 0)
    }
    async fn enroll(&self, e: &VoiceExample) -> Result<(), PersistenceError> {
        let data = encode(e)?;
        sqlx::query_file!("src/speakers/enroll.sql", e.id, e.profile_id, data)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn remove_example(&self, id: &str) -> Result<bool, PersistenceError> {
        Ok(sqlx::query_file!("src/speakers/remove_example.sql", id)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .rows_affected()
            > 0)
    }
    async fn correct(
        &self,
        c: &Correction,
        expected_revision: usize,
    ) -> Result<bool, PersistenceError> {
        let expected_revision = expected_revision as i64;
        let data = encode(c)?;
        let result = sqlx::query_file!(
            "src/speakers/correct.sql",
            c.id,
            c.segment_id,
            c.profile_id,
            data,
            expected_revision
        )
        .execute(&self.pool)
        .await
        .map_err(PersistenceError::new)?;
        Ok(result.rows_affected() > 0)
    }
}

impl SqliteStore {
    async fn suggestion_resolutions(
        &self,
        id: &str,
    ) -> Result<Vec<SuggestionResolution>, PersistenceError> {
        sqlx::query_file!("src/speakers/resolutions.sql", id)
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .iter()
            .map(|r| {
                let mut result: SuggestionResolution = decode(&r.data)?;
                result.profile_id = r.profile_id.clone();
                Ok(result)
            })
            .collect()
    }
}

async fn append_correction(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    c: &Correction,
    expected: usize,
) -> Result<bool, PersistenceError> {
    let data = encode(c)?;
    let expected = expected as i64;
    let result = sqlx::query_file!(
        "src/speakers/correct.sql",
        c.id,
        c.segment_id,
        c.profile_id,
        data,
        expected
    )
    .execute(&mut **tx)
    .await
    .map_err(PersistenceError::new)?;
    Ok(result.rows_affected() == 1)
}
