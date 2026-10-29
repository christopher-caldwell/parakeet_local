use async_trait::async_trait;
use parakeet_application::{audio_locations::AudioLocationWrites, *};
use parakeet_domain::{AudioTrack, ImportBundle};
use std::sync::Arc;

/// Entry-point decorator: every available source is checked as decodable audio before it is
/// published to the library or a track is relinked to it.
pub struct ValidatedAudioWrites {
    pub library: Arc<dyn LibraryWrites>,
    pub locations: Arc<dyn AudioLocationWrites>,
}
#[async_trait]
impl LibraryWrites for ValidatedAudioWrites {
    async fn import(&self, bundle: &ImportBundle) -> Result<bool, PersistenceError> {
        for track in &bundle.tracks {
            if track.available {
                crate::audio::validate_source(track).map_err(PersistenceError::new)?;
            }
        }
        self.library.import(bundle).await
    }
    async fn rename(&self, id: &str, title: &str) -> Result<bool, PersistenceError> {
        self.library.rename(id, title).await
    }
}
#[async_trait]
impl AudioLocationWrites for ValidatedAudioWrites {
    async fn replace_locator(
        &self,
        expected: &AudioTrack,
        replacement: &AudioTrack,
    ) -> Result<bool, PersistenceError> {
        crate::audio::validate_source(replacement).map_err(PersistenceError::new)?;
        self.locations.replace_locator(expected, replacement).await
    }
}
