use async_trait::async_trait;
use parakeet_application::{audio_locations::AudioLocationWrites, *};
use parakeet_domain::{AudioTrack, ImportBundle};
use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct StorageGate {
    path: PathBuf,
}
pub struct StorageAccess(File);
impl Drop for StorageAccess {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}
impl StorageGate {
    pub fn for_database(database: &Path) -> io::Result<Self> {
        let database = database.canonicalize()?;
        let mut name = database
            .file_name()
            .ok_or_else(|| io::Error::other("database filename missing"))?
            .to_os_string();
        name.push(".storage-access.lock");
        Ok(Self {
            path: database.with_file_name(name),
        })
    }
    pub async fn acquire(&self, exclusive: bool) -> Result<StorageAccess, CapabilityError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let f = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)?;
            if exclusive {
                fs2::FileExt::try_lock_exclusive(&f)?;
            } else {
                fs2::FileExt::try_lock_shared(&f)?;
            }
            Ok(StorageAccess(f))
        })
        .await
        .map_err(CapabilityError::new)?
        .map_err(|e: io::Error| {
            CapabilityError::message(format!("storage access is busy or unavailable: {e}"))
        })
    }
}
/// Entry-point decorator: validation and publication share the storage gate, so library
/// imports and audio relinks cannot interleave with a file being replaced on disk.
pub struct GuardedAudioWrites {
    pub gate: StorageGate,
    pub library: Arc<dyn LibraryWrites>,
    pub locations: Arc<dyn AudioLocationWrites>,
}
#[async_trait]
impl LibraryWrites for GuardedAudioWrites {
    async fn import(&self, bundle: &ImportBundle) -> Result<bool, PersistenceError> {
        let _guard = self
            .gate
            .acquire(false)
            .await
            .map_err(PersistenceError::new)?;
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
impl AudioLocationWrites for GuardedAudioWrites {
    async fn replace_locator(
        &self,
        expected: &AudioTrack,
        replacement: &AudioTrack,
    ) -> Result<bool, PersistenceError> {
        let _guard = self
            .gate
            .acquire(false)
            .await
            .map_err(PersistenceError::new)?;
        crate::audio::validate_source(replacement).map_err(PersistenceError::new)?;
        self.locations.replace_locator(expected, replacement).await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn exclusive_access_blocks_others_through_a_symlinked_database_path() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("test.db");
        std::fs::write(&database, b"").unwrap();
        let alias = dir.path().join("alias.db");
        std::os::unix::fs::symlink(&database, &alias).unwrap();
        let held = StorageGate::for_database(&database)
            .unwrap()
            .acquire(true)
            .await
            .unwrap();
        let other = StorageGate::for_database(&alias).unwrap();
        assert!(other.acquire(false).await.is_err());
        drop(held);
        assert!(other.acquire(false).await.is_ok());
    }
}
