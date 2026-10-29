use async_trait::async_trait;
use parakeet_application::{
    CapabilityError,
    jobs::{ProcessingLease, ProcessingOwnership},
};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct FileProcessingOwnership {
    directory: PathBuf,
}
impl FileProcessingOwnership {
    /// Canonical database location, not data-root or PID, defines the ownership
    /// scope. CLI database overrides and symlink aliases must share locks.
    pub fn for_database(database: &Path) -> Result<Self, std::io::Error> {
        let database = database.canonicalize()?;
        let mut name = database.file_name().expect("database file").to_os_string();
        name.push(".processing-leases");
        Ok(Self {
            directory: database.with_file_name(name),
        })
    }
}
struct FileLease(File, PathBuf);
impl ProcessingLease for FileLease {}
impl Drop for FileLease {
    fn drop(&mut self) {
        // Unlink while still holding the lock; acquirers re-check the path's identity after
        // locking, so a racing opener of the old inode retries instead of sharing ownership.
        let _ = std::fs::remove_file(&self.1);
        let _ = fs2::FileExt::unlock(&self.0);
    }
}
#[async_trait]
impl ProcessingOwnership for FileProcessingOwnership {
    async fn try_acquire(
        &self,
        run_id: &str,
    ) -> Result<Option<Box<dyn ProcessingLease>>, CapabilityError> {
        let id = uuid::Uuid::parse_str(run_id)
            .map_err(CapabilityError::new)?
            .to_string();
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            use std::os::unix::fs::MetadataExt;
            std::fs::create_dir_all(&directory).map_err(CapabilityError::new)?;
            let path = directory.join(format!("{id}.lock"));
            loop {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&path)
                    .map_err(CapabilityError::new)?;
                match fs2::FileExt::try_lock_exclusive(&file) {
                    Ok(()) => {
                        let same = match (file.metadata(), std::fs::metadata(&path)) {
                            (Ok(held), Ok(current)) => {
                                held.ino() == current.ino() && held.dev() == current.dev()
                            }
                            _ => false,
                        };
                        if same {
                            return Ok(Some(
                                Box::new(FileLease(file, path)) as Box<dyn ProcessingLease>
                            ));
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                    Err(e) => return Err(CapabilityError::new(e)),
                }
            }
        })
        .await
        .map_err(CapabilityError::new)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn released_leases_leave_no_lock_files_and_live_owner_still_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        let database = tmp.path().join("test.db");
        std::fs::write(&database, b"fixture").unwrap();
        let ownership = FileProcessingOwnership::for_database(&database).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let held = ownership.try_acquire(&id).await.unwrap().unwrap();
        assert!(ownership.try_acquire(&id).await.unwrap().is_none());
        drop(held);
        for _ in 0..5 {
            let lease = ownership.try_acquire(&id).await.unwrap().unwrap();
            drop(lease);
        }
        assert_eq!(std::fs::read_dir(&ownership.directory).unwrap().count(), 0);
    }
    #[tokio::test]
    async fn lease_holder_child() {
        let Some(database) = std::env::var_os("PARAKEET_TEST_LEASE_DATABASE") else {
            return;
        };
        let ownership = FileProcessingOwnership::for_database(Path::new(&database)).unwrap();
        let id = std::env::var("PARAKEET_TEST_LEASE_ID").unwrap();
        let _lease = ownership.try_acquire(&id).await.unwrap().unwrap();
        println!("LEASE_READY");
        use std::io::Write;
        std::io::stdout().flush().unwrap();
        std::future::pending::<()>().await;
    }
    #[tokio::test]
    async fn process_death_releases_lease_and_aliases_share_scope() {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let tmp = tempfile::tempdir().unwrap();
        let database = tmp.path().join("test.db");
        std::fs::write(&database, b"fixture").unwrap();
        let ownership = FileProcessingOwnership::for_database(&database).unwrap();
        assert!(!ownership.directory.exists());
        assert!(ownership.try_acquire("../invalid").await.is_err());
        let id = uuid::Uuid::new_v4().to_string();
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "jobs::tests::lease_holder_child", "--nocapture"])
            .env("PARAKEET_TEST_LEASE_DATABASE", &database)
            .env("PARAKEET_TEST_LEASE_ID", &id)
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(line) = lines.next_line().await.unwrap() {
                if line == "LEASE_READY" {
                    return;
                }
            }
            panic!("child exited before acquiring lease");
        })
        .await
        .unwrap();
        let alias = tmp.path().join("alias.db");
        std::os::unix::fs::symlink(&database, &alias).unwrap();
        let alias_owner = FileProcessingOwnership::for_database(&alias).unwrap();
        assert!(alias_owner.try_acquire(&id).await.unwrap().is_none());
        child.kill().await.unwrap();
        child.wait().await.unwrap();
        assert!(ownership.try_acquire(&id).await.unwrap().is_some());
    }
}
