use super::*;
use async_trait::async_trait;
use parakeet_application::settings::{SettingsQueries, SettingsWrites};
use parakeet_domain::gestures::Shortcut;
#[async_trait]
impl SettingsQueries for SqliteStore {
    async fn prompts(&self) -> Result<Vec<PromptProfile>, PersistenceError> {
        sqlx::query_file!("src/settings/prompts.sql")
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .iter()
            .map(|r| decode(&r.data))
            .collect()
    }
    async fn shortcuts(&self) -> Result<Vec<Shortcut>, PersistenceError> {
        sqlx::query_file!("src/settings/shortcuts.sql")
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .iter()
            .map(|r| decode(&r.data))
            .collect()
    }
}
#[async_trait]
impl SettingsWrites for SqliteStore {
    async fn save_prompt(&self, p: &PromptProfile) -> Result<(), PersistenceError> {
        let data = encode(p)?;
        sqlx::query_file!("src/settings/prompt.sql", p.id, p.bundle_id, data)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn replace_shortcuts(&self, bindings: &[Shortcut]) -> Result<(), PersistenceError> {
        let mut tx = self.pool.begin().await.map_err(PersistenceError::new)?;
        sqlx::query_file!("src/settings/clear_shortcuts.sql")
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        for b in bindings {
            let data = encode(b)?;
            sqlx::query_file!("src/settings/shortcut.sql", b.id, data)
                .execute(&mut *tx)
                .await
                .map_err(PersistenceError::new)?;
        }
        tx.commit().await.map_err(PersistenceError::new)?;
        Ok(())
    }
}
#[async_trait]
impl parakeet_application::settings::AudioPreferencesQueries for SqliteStore {
    async fn audio_preferences(&self) -> Result<AudioPreferences, PersistenceError> {
        let row = sqlx::query_file!("src/settings/audio.sql")
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?;
        row.map(|r| decode(&r.data))
            .unwrap_or_else(|| Ok(AudioPreferences::default()))
    }
}
#[async_trait]
impl parakeet_application::settings::AudioPreferencesWrites for SqliteStore {
    async fn save_audio_preferences(&self, p: &AudioPreferences) -> Result<(), PersistenceError> {
        let data = encode(p)?;
        sqlx::query_file!("src/settings/save_audio.sql", data)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
}
#[async_trait]
impl parakeet_application::dictation::DictationWrites for SqliteStore {
    async fn save_dictation(
        &self,
        r: &parakeet_application::dictation::DictationReceipt,
    ) -> Result<(), PersistenceError> {
        let data = encode(r)?;
        sqlx::query_file!("src/settings/dictation.sql", r.id, r.recording_id, data)
            .execute(&self.pool)
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
}

#[async_trait]
impl parakeet_application::dictation::DictationQueries for SqliteStore {
    async fn dictations(
        &self,
    ) -> Result<Vec<parakeet_application::dictation::DictationReceipt>, PersistenceError> {
        sqlx::query_file!("src/settings/dictations.sql")
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .iter()
            .map(|r| decode(&r.data))
            .collect()
    }
    async fn dictation(
        &self,
        id: &str,
    ) -> Result<Option<parakeet_application::dictation::DictationReceipt>, PersistenceError> {
        sqlx::query_file!("src/settings/show_dictation.sql", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?
            .map(|r| decode(&r.data))
            .transpose()
    }
}
