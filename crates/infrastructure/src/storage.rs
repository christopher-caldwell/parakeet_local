//! Metadata-only accounting. No source directory traversal, content reads, cleanup
//! or model initialization occurs here; symlinks inside the root are not followed.
use parakeet_application::{CapabilityError, storage::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
#[derive(Clone)]
pub struct LocalStorageInspection {
    pub root: PathBuf,
    pub database: PathBuf,
}
#[async_trait::async_trait]
impl StorageInspection for LocalStorageInspection {
    async fn observe(
        &self,
        audio: &[AudioStorageReference],
    ) -> Result<StorageObservation, CapabilityError> {
        let this = self.clone();
        let audio = audio.to_vec();
        tokio::task::spawn_blocking(move || this.scan(&audio))
            .await
            .map_err(CapabilityError::new)?
    }
}
fn identity(m: &std::fs::Metadata) -> (u64, u64) {
    (m.dev(), m.ino())
}
fn allocated(m: &std::fs::Metadata) -> u64 {
    m.blocks().saturating_mul(512)
}
impl LocalStorageInspection {
    fn scan(&self, audio: &[AudioStorageReference]) -> Result<StorageObservation, CapabilityError> {
        let root = self.root.canonicalize().map_err(CapabilityError::new)?;
        if !root.is_dir() {
            return Err(CapabilityError::message(
                "selected storage root is not a directory",
            ));
        }
        if audio.len() > 100000 {
            return Err(CapabilityError::message(
                "audio-reference inventory exceeds 100000 entries",
            ));
        }
        let database = self.database.canonicalize().map_err(CapabilityError::new)?;
        let database_paths = [
            database.clone(),
            append_suffix(&database, "-wal"),
            append_suffix(&database, "-shm"),
        ];
        let mut linked = BTreeMap::<(u64, u64), Vec<String>>::new();
        let mut external = BTreeMap::<String, ExternalAudioObservation>::new();
        let mut external_identity = BTreeMap::<(u64, u64), String>::new();
        let mut warnings = vec![];
        let mut complete = true;
        for reference in audio {
            match Path::new(&reference.path)
                .canonicalize()
                .and_then(|p| std::fs::metadata(&p).map(|m| (p, m)))
            {
                Ok((path, meta)) if meta.is_file() => {
                    let key = identity(&meta);
                    linked
                        .entry(key)
                        .or_default()
                        .push(reference.track_id.clone());
                    if !path.starts_with(&root) {
                        let display = external_identity
                            .entry(key)
                            .or_insert_with(|| path.to_string_lossy().into_owned())
                            .clone();
                        external
                            .entry(display.clone())
                            .or_insert_with(|| ExternalAudioObservation {
                                path: display,
                                track_ids: vec![],
                                missing_or_unresolved: false,
                                logical_bytes: meta.len(),
                                allocated_bytes: allocated(&meta),
                            })
                            .track_ids
                            .push(reference.track_id.clone());
                    }
                }
                result => {
                    complete = false;
                    let description = match result {
                        Ok(_) => "not a regular audio file".into(),
                        Err(e) => e.to_string(),
                    };
                    if warnings.len() < 200 {
                        warnings.push(format!(
                            "Referenced audio {}: {description}",
                            reference.path
                        ));
                    }
                    external
                        .entry(reference.path.clone())
                        .or_insert_with(|| ExternalAudioObservation {
                            path: reference.path.clone(),
                            track_ids: vec![],
                            missing_or_unresolved: true,
                            logical_bytes: 0,
                            allocated_bytes: 0,
                        })
                        .track_ids
                        .push(reference.track_id.clone());
                }
            }
        }
        for ids in linked.values_mut() {
            ids.sort();
            ids.dedup();
        }
        let mut files = vec![];
        let mut stack = vec![(root.clone(), 0usize)];
        let mut visited = 0usize;
        while let Some((directory, depth)) = stack.pop() {
            let entries = match std::fs::read_dir(&directory) {
                Ok(e) => e,
                Err(e) => {
                    complete = false;
                    if warnings.len() < 200 {
                        warnings.push(format!("Cannot inspect {}: {e}", directory.display()));
                    }
                    continue;
                }
            };
            for entry in entries {
                visited += 1;
                if visited > 100000 {
                    complete = false;
                    warnings.push(
                        "Storage scan stopped at 100000 entries; totals are incomplete".into(),
                    );
                    stack.clear();
                    break;
                }
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        complete = false;
                        if warnings.len() < 200 {
                            warnings.push(format!("Unreadable entry: {e}"));
                        }
                        continue;
                    }
                };
                let path = entry.path();
                let meta = match std::fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(e) => {
                        complete = false;
                        if warnings.len() < 200 {
                            warnings.push(format!("Cannot inspect {}: {e}", path.display()));
                        }
                        continue;
                    }
                };
                if meta.is_dir() {
                    if depth >= 32 {
                        complete = false;
                        if warnings.len() < 200 {
                            warnings.push(format!(
                                "Depth limit at {}; totals are incomplete",
                                path.display()
                            ));
                        }
                    } else {
                        stack.push((path, depth + 1));
                    }
                    continue;
                }
                if !meta.is_file() {
                    complete = false;
                    if warnings.len() < 200 {
                        warnings.push(format!(
                            "Link or special file not followed: {}",
                            path.display()
                        ));
                    }
                }
                let relative = path.strip_prefix(&root).map_err(CapabilityError::new)?;
                let (area, run_id, recognized) = classify(relative, &path, &database_paths);
                let key = identity(&meta);
                files.push(StorageFileObservation {
                    relative_path: relative.to_string_lossy().into_owned(),
                    area,
                    logical_bytes: meta.len(),
                    allocated_bytes: allocated(&meta),
                    device: key.0,
                    inode: key.1,
                    hard_links: meta.nlink(),
                    ordinary_file: meta.is_file(),
                    run_id,
                    recognized_completed_scratch: recognized,
                    linked_audio_tracks: linked.get(&key).cloned().unwrap_or_default(),
                });
            }
        }
        files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        let mut identities: BTreeSet<_> = files.iter().map(|f| (f.device, f.inode)).collect();
        let mut database_outside_root_bytes = 0u64;
        for path in &database_paths {
            if !path.starts_with(&root) {
                match std::fs::metadata(path) {
                    Ok(m) if identities.insert(identity(&m)) => {
                        database_outside_root_bytes =
                            database_outside_root_bytes.saturating_add(m.len())
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        complete = false;
                        if warnings.len() < 200 {
                            warnings.push(format!(
                                "Cannot inspect selected database {}: {e}",
                                path.display()
                            ));
                        }
                    }
                }
            }
        }
        Ok(StorageObservation {
            root: root.to_string_lossy().into_owned(),
            files,
            external_audio: external.into_values().collect(),
            database_outside_root_bytes,
            volume_available_bytes: fs2::available_space(&root).ok(),
            volume_total_bytes: fs2::total_space(&root).ok(),
            complete,
            warnings,
        })
    }
}
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    s.into()
}
fn classify(
    relative: &Path,
    absolute: &Path,
    database_paths: &[PathBuf; 3],
) -> (StorageArea, Option<String>, bool) {
    use StorageArea::*;
    if database_paths.iter().any(|p| p == absolute) {
        return (Database, None, false);
    }
    let Some(text) = relative.to_str() else {
        return (Other, None, false);
    };
    let parts: Vec<_> = text.split('/').collect();
    let area = match parts.first().copied() {
        Some("captures" | "media") => RecordedAudio,
        Some("models") if parts.get(1) == Some(&"work") => Processing,
        Some("models") => Models,
        Some("playback") => Playback,
        Some("clipboard-recovery") => Recovery,
        Some("evaluations") => Evaluations,
        _ => Other,
    };
    if area != Processing {
        return (area, None, false);
    }
    let run_id = parts
        .get(2)
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .map(|id| id.to_string())
        .filter(|id| parts.get(2) == Some(&id.as_str()));
    let recognized = run_id.is_some()
        && match parts.as_slice() {
            [_, _, _, "audio.f32" | "request.json"] => true,
            [_, _, _, "whisper-chunks", name] => name
                .strip_prefix("chunk-")
                .and_then(|s| s.strip_suffix(".json"))
                .is_some_and(|s| {
                    s.len() == 4
                        && s.bytes().all(|b| b.is_ascii_digit())
                        && s.parse::<usize>().is_ok_and(|n| n < 120)
                }),
            _ => false,
        };
    (area, run_id, recognized)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_known_complete_artifact_layouts_are_recognized() {
        let id = uuid::Uuid::new_v4().to_string();
        let db = [
            PathBuf::from("/db"),
            PathBuf::from("/db-wal"),
            PathBuf::from("/db-shm"),
        ];
        for name in [
            "audio.f32",
            "request.json",
            "whisper-chunks/chunk-0000.json",
        ] {
            let p = PathBuf::from(format!("models/work/{id}/{name}"));
            assert!(classify(&p, &p, &db).2);
        }
        for name in [
            "audio.f32.partial",
            "whisper-chunks/chunk-0000.json.partial",
            "whisper-chunks/chunk-evil.json",
            "original.m4a",
            "nested/audio.f32",
        ] {
            let p = PathBuf::from(format!("models/work/{id}/{name}"));
            assert!(!classify(&p, &p, &db).2);
        }
        let p = Path::new("models/work/not-a-run/audio.f32");
        assert!(!classify(p, p, &db).2);
    }
    #[tokio::test]
    async fn scanner_counts_files_links_and_external_references_without_following_root_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("owned");
        std::fs::create_dir(&root).unwrap();
        let database = root.join("parakeet.db");
        std::fs::write(&database, b"database").unwrap();
        let outside = tmp.path().join("original.wav");
        std::fs::write(&outside, b"preserved original").unwrap();
        let before = std::fs::metadata(&outside).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let work = root.join("models/work").join(&id);
        std::fs::create_dir_all(&work).unwrap();
        std::fs::hard_link(&outside, work.join("audio.f32")).unwrap();
        std::os::unix::fs::symlink(tmp.path(), root.join("loop")).unwrap();
        let inspection = LocalStorageInspection {
            root: root.clone(),
            database,
        };
        let refs = vec![
            AudioStorageReference {
                track_id: "track".into(),
                recording_id: "recording".into(),
                path: outside.to_str().unwrap().into(),
            },
            AudioStorageReference {
                track_id: "missing".into(),
                recording_id: "recording".into(),
                path: tmp.path().join("absent.wav").to_str().unwrap().into(),
            },
        ];
        let observation = inspection.observe(&refs).await.unwrap();
        assert!(!observation.complete);
        assert_eq!(observation.files.len(), 3);
        assert_eq!(observation.external_audio.len(), 2);
        let alias = observation
            .files
            .iter()
            .find(|f| f.run_id.is_some())
            .unwrap();
        assert_eq!(alias.linked_audio_tracks, vec!["track"]);
        assert_eq!(alias.hard_links, 2);
        assert_eq!(std::fs::read(&outside).unwrap(), b"preserved original");
        assert_eq!(
            std::fs::metadata(&outside).unwrap().mtime_nsec(),
            before.mtime_nsec()
        );
        assert!(root.join("loop").is_symlink());
    }
}
