use crate::{
    jobs::FileProcessingOwnership,
    models::{ModelFiles, ModelLease},
    storage::LocalStorageInspection,
};
use async_trait::async_trait;
use parakeet_application::{
    audio_locations::AudioLocationWrites,
    cleanup::*,
    jobs::{ProcessingLease, ProcessingOwnership},
    storage::*,
    *,
};
use parakeet_domain::{AudioTrack, ImportBundle};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
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
/// Entry-point decorator: validation and publication share the cleanup gate.
/// Revalidation here closes the prepared-import/relink versus move race.
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
#[derive(Clone)]
pub struct LocalCleanupFiles {
    root: PathBuf,
    database: PathBuf,
    gate: StorageGate,
    models: Arc<ModelFiles>,
    ownership: Arc<FileProcessingOwnership>,
}
impl LocalCleanupFiles {
    pub fn new(root: PathBuf, database: PathBuf, models: Arc<ModelFiles>) -> io::Result<Self> {
        Ok(Self {
            gate: StorageGate::for_database(&database)?,
            ownership: Arc::new(FileProcessingOwnership::for_database(&database)?),
            root,
            database,
            models,
        })
    }
}
struct Access {
    held: Arc<Held>,
    scope: CleanupScope,
}
struct Held {
    root: Arc<File>,
    _gate: StorageAccess,
    _model: ModelLease,
    _runs: Vec<Box<dyn ProcessingLease>>,
}
#[async_trait]
impl CleanupFiles for LocalCleanupFiles {
    async fn acquire(&self, run_ids: &[String]) -> Result<Box<dyn CleanupAccess>, CapabilityError> {
        let gate = self.gate.acquire(true).await?;
        let models = self.models.clone();
        let selected_root = self.root.clone();
        let model = tokio::task::spawn_blocking(move || {
            let root = selected_root.canonicalize().map_err(CapabilityError::new)?;
            if models.root.file_name() != Some(std::ffi::OsStr::new("models"))
                || models
                    .root
                    .parent()
                    .and_then(|p| p.canonicalize().ok())
                    .as_deref()
                    != Some(root.as_path())
            {
                return Err(CapabilityError::message(
                    "cleanup requires the selected root's own models directory",
                ));
            }
            match std::fs::symlink_metadata(&models.root) {
                Ok(meta) if !meta.is_dir() => {
                    return Err(CapabilityError::message(
                        "models directory is linked or not a directory",
                    ));
                }
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    return Err(CapabilityError::new(e));
                }
                _ => {}
            }
            models.acquire()
        })
        .await
        .map_err(CapabilityError::new)??;
        let mut runs = Vec::new();
        for id in run_ids {
            runs.push(self.ownership.try_acquire(id).await?.ok_or_else(|| {
                CapabilityError::message("processing run still has a live owner")
            })?);
        }
        let root = self.root.clone();
        let database = self.database.clone();
        let (root, scope) = tokio::task::spawn_blocking(move || -> io::Result<_> {
            let path = root.canonicalize()?;
            let database = database.canonicalize()?;
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)?;
            let meta = file.metadata()?;
            Ok((
                file,
                CleanupScope {
                    root: path.to_string_lossy().into_owned(),
                    database: database.to_string_lossy().into_owned(),
                    device: meta.dev(),
                    inode: meta.ino(),
                },
            ))
        })
        .await
        .map_err(CapabilityError::new)?
        .map_err(CapabilityError::new)?;
        Ok(Box::new(Access {
            scope,
            held: Arc::new(Held {
                root: Arc::new(root),
                _gate: gate,
                _model: model,
                _runs: runs,
            }),
        }))
    }
}
#[async_trait]
impl CleanupAccess for Access {
    fn scope(&self) -> CleanupScope {
        self.scope.clone()
    }
    async fn observe(
        &self,
        audio: &[AudioStorageReference],
    ) -> Result<StorageObservation, CapabilityError> {
        LocalStorageInspection {
            root: PathBuf::from(&self.scope.root),
            database: PathBuf::from(&self.scope.database),
        }
        .observe(audio)
        .await
    }
    async fn identify(
        &self,
        candidate: &RetentionCandidate,
    ) -> Result<CleanupFile, CapabilityError> {
        let root = self.held.root.clone();
        let candidate = candidate.clone();
        tokio::task::spawn_blocking(move || {
            identify(&root, &candidate.relative_path, &candidate.run_id)
        })
        .await
        .map_err(CapabilityError::new)?
        .map_err(CapabilityError::new)
    }
    async fn transfer(
        &self,
        review: &CleanupReview,
        restore: bool,
    ) -> Result<Vec<String>, CapabilityError> {
        let held = self.held.clone();
        let review = review.clone();
        tokio::task::spawn_blocking(move || {
            let root = &held.root;
            let mut errors = Vec::new();
            // Stage checks the whole reviewed set before moving the first file.
            if !restore {
                for file in &review.files {
                    match identify(root, &file.relative_path, &file.run_id) {
                        Ok(actual) if actual == *file => {}
                        _ => errors
                            .push(format!("reviewed identity changed: {}", file.relative_path)),
                    }
                }
            }
            if errors.is_empty() {
                for (index, file) in review.files.iter().enumerate() {
                    if let Err(error) = transfer(root, &review.id, index, file, restore) {
                        errors.push(format!("{}: {error}", file.relative_path));
                    }
                }
            }
            errors
        })
        .await
        .map_err(CapabilityError::new)
    }
}
fn cstr(name: &str) -> io::Result<CString> {
    CString::new(name).map_err(io::Error::other)
}
fn components(relative: &str) -> io::Result<Vec<&str>> {
    let parts: Vec<_> = relative.split('/').collect();
    if parts.is_empty()
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains('\0'))
    {
        return Err(io::Error::other("unsafe relative path"));
    }
    Ok(parts)
}
fn directory(root: &File, parts: &[&str], create: bool) -> io::Result<File> {
    let mut current = root.try_clone()?;
    for part in parts {
        let name = cstr(part)?;
        if create {
            let result = unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o700) };
            if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error());
            }
            if result == 0 {
                current.sync_all()?;
            }
        }
        let fd = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        current = unsafe { File::from_raw_fd(fd) };
    }
    Ok(current)
}
fn open_file(parent: &File, name: &str) -> io::Result<File> {
    let name = cstr(name)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn known_path<'a>(relative: &'a str, run_id: &str) -> io::Result<Vec<&'a str>> {
    let p = components(relative)?;
    if p.len() < 4
        || p[0] != "models"
        || p[1] != "work"
        || p[2] != run_id
        || uuid::Uuid::parse_str(run_id)
            .map(|u| u.to_string())
            .ok()
            .as_deref()
            != Some(run_id)
    {
        return Err(io::Error::other("not a canonical processing path"));
    }
    let valid = match p.as_slice() {
        [_, _, _, "audio.f32" | "request.json"] => true,
        [_, _, _, "whisper-chunks", name] => name
            .strip_prefix("chunk-")
            .and_then(|v| v.strip_suffix(".json"))
            .is_some_and(|v| {
                v.len() == 4
                    && v.bytes().all(|b| b.is_ascii_digit())
                    && v.parse::<usize>().is_ok_and(|v| v < 120)
            }),
        _ => false,
    };
    if !valid {
        return Err(io::Error::other("unrecognized completed scratch file"));
    }
    Ok(p)
}
fn signature(meta: &std::fs::Metadata) -> (u64, u64, u64, i128, i128, u64) {
    (
        meta.dev(),
        meta.ino(),
        meta.len(),
        i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec()),
        i128::from(meta.ctime()) * 1_000_000_000 + i128::from(meta.ctime_nsec()),
        meta.nlink(),
    )
}
fn fingerprint(parent: &File, name: &str, relative: &str, run_id: &str) -> io::Result<CleanupFile> {
    let mut file = open_file(parent, name)?;
    let before = file.metadata()?;
    if !before.is_file() || before.nlink() != 1 || before.len() > MAX_BYTES {
        return Err(io::Error::other(
            "file is not a bounded single-link ordinary file",
        ));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    let mut read = 0u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        read = read.saturating_add(n as u64);
        if read > MAX_BYTES {
            return Err(io::Error::other("file grew beyond review limit"));
        }
        hash.update(&buffer[..n]);
    }
    let after = file.metadata()?;
    let current = open_file(parent, name)?.metadata()?;
    if signature(&before) != signature(&after)
        || signature(&after) != signature(&current)
        || read != after.len()
    {
        return Err(io::Error::other("file changed while hashing"));
    }
    let (device, inode, bytes, modified_ns, changed_ns, _) = signature(&after);
    Ok(CleanupFile {
        relative_path: relative.into(),
        run_id: run_id.into(),
        bytes,
        device,
        inode,
        modified_ns,
        changed_ns,
        sha256: format!("{:x}", hash.finalize()),
    })
}
fn identify(root: &File, relative: &str, run_id: &str) -> io::Result<CleanupFile> {
    let p = known_path(relative, run_id)?;
    let parent = directory(root, &p[..p.len() - 1], false)?;
    fingerprint(&parent, p[p.len() - 1], relative, run_id)
}
fn matches(actual: &CleanupFile, expected: &CleanupFile) -> bool {
    actual.device == expected.device
        && actual.inode == expected.inode
        && actual.bytes == expected.bytes
        && actual.modified_ns == expected.modified_ns
        && actual.sha256 == expected.sha256
}
fn transfer(
    root: &File,
    review_id: &str,
    index: usize,
    expected: &CleanupFile,
    restore: bool,
) -> io::Result<()> {
    if uuid::Uuid::parse_str(review_id)
        .map(|id| id.to_string())
        .ok()
        .as_deref()
        != Some(review_id)
    {
        return Err(io::Error::other("invalid review identity"));
    }
    let p = known_path(&expected.relative_path, &expected.run_id)?;
    let original = directory(root, &p[..p.len() - 1], restore)?;
    let recovery = directory(root, &[".cleanup", review_id, "files"], true)?;
    let original_name = p[p.len() - 1];
    let recovery_name = format!("{index:04}.bin");
    let (source, source_name, destination, destination_name) = if restore {
        (&recovery, recovery_name.as_str(), &original, original_name)
    } else {
        (&original, original_name, &recovery, recovery_name.as_str())
    };
    match open_file(destination, destination_name) {
        Ok(_) => {
            // A prior successful move is idempotent only when the source is absent.
            let actual = fingerprint(
                destination,
                destination_name,
                &expected.relative_path,
                &expected.run_id,
            )?;
            if matches(&actual, expected)
                && open_file(source, source_name)
                    .is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
            {
                return Ok(());
            }
            return Err(io::Error::other("destination exists; nothing overwritten"));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let actual = fingerprint(
        source,
        source_name,
        &expected.relative_path,
        &expected.run_id,
    )?;
    if !matches(&actual, expected) || (!restore && actual.changed_ns != expected.changed_ns) {
        return Err(io::Error::other(
            "source identity changed; retained in place",
        ));
    }
    open_file(source, source_name)?.sync_all()?;
    let old = cstr(source_name)?;
    let new = cstr(destination_name)?;
    let result = unsafe {
        libc::renameatx_np(
            source.as_raw_fd(),
            old.as_ptr(),
            destination.as_raw_fd(),
            new.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    source.sync_all()?;
    destination.sync_all()?;
    root.sync_all()?;
    let moved = fingerprint(
        destination,
        destination_name,
        &expected.relative_path,
        &expected.run_id,
    )?;
    if !matches(&moved, expected) {
        return Err(io::Error::other(
            "post-move identity changed; payload retained for inspection",
        ));
    }
    Ok(())
}
