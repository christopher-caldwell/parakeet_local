use super::*;
use parakeet_domain::gestures::Shortcut;
operation_error!(ListPromptsError,sources{Persistence:PersistenceError=>"persistence"},values{});
operation_error!(SavePromptError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence"},values{});
operation_error!(RevokePromptError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});
operation_error!(ListShortcutsError,sources{Persistence:PersistenceError=>"persistence"},values{});
operation_error!(SaveShortcutsError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence"},values{Conflict=>"conflict"});
operation_error!(TransformTextError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence"},values{});
operation_error!(ReadAudioPreferencesError,sources{Persistence:PersistenceError=>"persistence"},values{});
operation_error!(SaveAudioPreferencesError,sources{Invalid:ValidationError=>"invalid_input",Persistence:PersistenceError=>"persistence"},values{});
#[derive(Debug, Clone)]
pub struct TransformTextInput {
    pub bundle_id: String,
    pub raw: String,
    pub dry_run: bool,
}
#[async_trait]
pub trait SettingsQueries: Send + Sync {
    async fn prompts(&self) -> Result<Vec<PromptProfile>, PersistenceError>;
    async fn shortcuts(&self) -> Result<Vec<Shortcut>, PersistenceError>;
}
#[async_trait]
pub trait SettingsWrites: Send + Sync {
    async fn save_prompt(&self, profile: &PromptProfile) -> Result<(), PersistenceError>;
    async fn replace_shortcuts(&self, bindings: &[Shortcut]) -> Result<(), PersistenceError>;
}
#[async_trait]
pub trait TextTransformation: Send + Sync {
    async fn transform(
        &self,
        profile: &PromptProfile,
        raw: &str,
    ) -> Result<String, CapabilityError>;
}
#[derive(Debug, Serialize, Deserialize)]
pub struct TransformResult {
    pub raw: String,
    pub enhanced: Option<String>,
    pub route: Option<String>,
    pub profile_id: Option<String>,
    pub failure: Option<String>,
}
pub struct SettingsService {
    queries: Arc<dyn SettingsQueries>,
    writes: Arc<dyn SettingsWrites>,
    transformer: Arc<dyn TextTransformation>,
}
impl SettingsService {
    pub fn new(
        queries: Arc<dyn SettingsQueries>,
        writes: Arc<dyn SettingsWrites>,
        transformer: Arc<dyn TextTransformation>,
    ) -> Self {
        Self {
            queries,
            writes,
            transformer,
        }
    }
    pub async fn prompts(&self) -> Result<Vec<PromptProfile>, ListPromptsError> {
        Ok(self.queries.prompts().await?)
    }
    pub async fn save_prompt(&self, profile: &PromptProfile) -> Result<(), SavePromptError> {
        profile.validate()?;
        self.writes.save_prompt(profile).await?;
        Ok(())
    }
    pub async fn revoke(&self, id: &str) -> Result<PromptProfile, RevokePromptError> {
        let mut profile = self
            .queries
            .prompts()
            .await?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| RevokePromptError::NotFound(id.into()))?;
        profile.authorized = false;
        profile.enabled = false;
        self.writes.save_prompt(&profile).await?;
        Ok(profile)
    }
    pub async fn shortcuts(&self) -> Result<Vec<Shortcut>, ListShortcutsError> {
        Ok(self.queries.shortcuts().await?)
    }
    pub async fn save_shortcuts(&self, bindings: &[Shortcut]) -> Result<(), SaveShortcutsError> {
        let mut ids = std::collections::BTreeSet::new();
        for (i, a) in bindings.iter().enumerate() {
            a.validate()?;
            if a.id.is_empty() || a.key.is_empty() || !ids.insert(&a.id) {
                return Err(
                    ValidationError("shortcuts need unique nonempty IDs and keys".into()).into(),
                );
            }
            for b in bindings.iter().skip(i + 1) {
                if a.conflicts(b) {
                    return Err(SaveShortcutsError::Conflict(b.id.clone()));
                }
            }
        }
        self.writes.replace_shortcuts(bindings).await?;
        Ok(())
    }
    pub async fn transform(
        &self,
        input: TransformTextInput,
    ) -> Result<TransformResult, TransformTextError> {
        let bundle_id = input.bundle_id.as_str();
        let raw = input.raw.as_str();
        let dry_run = input.dry_run;
        let profiles = self.queries.prompts().await?;
        let matches: Vec<_> = profiles.iter().filter(|p| p.permits(bundle_id)).collect();
        if matches.len() > 1 {
            return Err(ValidationError(
                "multiple authorized profiles for the same application".into(),
            )
            .into());
        }
        let mut result = TransformResult {
            raw: raw.into(),
            enhanced: None,
            route: None,
            profile_id: None,
            failure: None,
        };
        if let Some(profile) = matches.first() {
            profile.validate()?;
            result.route = Some(profile.route.clone());
            result.profile_id = Some(profile.id.clone());
            if !dry_run {
                match self.transformer.transform(profile, raw).await {
                    Ok(s) if !s.trim().is_empty() => result.enhanced = Some(s),
                    Ok(_) => result.failure = Some("transformation returned empty text".into()),
                    Err(e) => result.failure = Some(e.to_string()),
                }
            }
        }
        Ok(result)
    }
}
#[async_trait]
pub trait AudioPreferencesQueries: Send + Sync {
    async fn audio_preferences(&self) -> Result<AudioPreferences, PersistenceError>;
}
#[async_trait]
pub trait AudioPreferencesWrites: Send + Sync {
    async fn save_audio_preferences(
        &self,
        preferences: &AudioPreferences,
    ) -> Result<(), PersistenceError>;
}
pub struct AudioSettingsService {
    queries: Arc<dyn AudioPreferencesQueries>,
    writes: Arc<dyn AudioPreferencesWrites>,
}
impl AudioSettingsService {
    pub fn new(
        queries: Arc<dyn AudioPreferencesQueries>,
        writes: Arc<dyn AudioPreferencesWrites>,
    ) -> Self {
        Self { queries, writes }
    }
    pub async fn get(&self) -> Result<AudioPreferences, ReadAudioPreferencesError> {
        Ok(self.queries.audio_preferences().await?)
    }
    pub async fn save(&self, p: &AudioPreferences) -> Result<(), SaveAudioPreferencesError> {
        p.validate()?;
        self.writes.save_audio_preferences(p).await?;
        Ok(())
    }
}
