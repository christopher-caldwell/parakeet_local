//! Private, versioned checkpoints. Incompatible evidence causes a recorded full
//! restart; it can never be transplanted into a different source or model.
use parakeet_application::{
    CapabilityError, processing::RunProcessingInput, processing_control::*,
};
use parakeet_domain::{AudioTrack, ProcessingRun, TranscriptSegment, stable_id};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

// Bump this identity when decoding, inference parameters, or timing derivation
// changes; a compatible weight revision alone is not enough for chunk reuse.
const RUNTIME: &str =
    "whisper-rs-0.16.0:en:beam5:threads4:temp0:no-context:60000ms:no-overlap:token-whitespace-v1";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceIdentity {
    pub sha256: String,
    pub bytes: u64,
}
pub fn identify_source(
    track: &AudioTrack,
    control: &ProcessingControl,
) -> Result<SourceIdentity, CapabilityError> {
    crate::audio::validate_source(track)?;
    let mut file = std::fs::File::open(&track.path).map_err(CapabilityError::new)?;
    let before = file.metadata().map_err(CapabilityError::new)?;
    if !before.is_file() {
        return Err(CapabilityError::message(
            "resume source must be a regular file",
        ));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 131072];
    let mut bytes = 0u64;
    loop {
        control.check()?;
        let n = file.read(&mut buffer).map_err(CapabilityError::new)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        if bytes > before.len() {
            return Err(CapabilityError::message("source grew while hashing"));
        }
        hash.update(&buffer[..n]);
    }
    let after = file.metadata().map_err(CapabilityError::new)?;
    use std::os::unix::fs::MetadataExt;
    if bytes != before.len()
        || before.len() != after.len()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.mtime() != after.mtime()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err(CapabilityError::message(
            "source changed while checking resume identity",
        ));
    }
    crate::audio::validate_source(track)?;
    let identity = SourceIdentity {
        sha256: hex::encode(hash.finalize()),
        bytes,
    };
    if track.provenance["content_sha256"]
        .as_str()
        .is_some_and(|hash| hash != identity.sha256)
    {
        return Err(CapabilityError::message(
            "source content no longer matches its saved SHA-256",
        ));
    }
    Ok(identity)
}
#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u8,
    sha256: String,
    payload: Box<serde_json::value::RawValue>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Chunk {
    runtime: String,
    request: RunProcessingInput,
    track: AudioTrack,
    run_id: String,
    model_revision: String,
    model_key: String,
    source: SourceIdentity,
    chunk_index: usize,
    local_start_ms: i64,
    audio_completed_ms: i64,
    decoded_ms: i64,
    segments: Vec<TranscriptSegment>,
    raw: Vec<Value>,
}
pub struct ChunkEvidence<'a> {
    pub request: &'a RunProcessingInput,
    pub track: &'a AudioTrack,
    pub run: &'a ProcessingRun,
    pub source: &'a SourceIdentity,
    pub model_key: &'a str,
    pub index: usize,
    pub start_ms: i64,
    pub completed_ms: i64,
    pub decoded_ms: i64,
    pub segments: &'a [TranscriptSegment],
    pub raw: &'a [Value],
}
fn directory(root: &Path, run_id: &str) -> Result<PathBuf, CapabilityError> {
    let id = uuid::Uuid::parse_str(run_id)
        .map_err(CapabilityError::new)?
        .to_string();
    Ok(root.join("work").join(id).join("whisper-chunks"))
}
pub fn retain_chunk(root: &Path, e: ChunkEvidence<'_>) -> Result<(), CapabilityError> {
    let chunk = Chunk {
        runtime: RUNTIME.into(),
        request: e.request.clone(),
        track: e.track.clone(),
        run_id: e.run.id.clone(),
        model_revision: e.run.model_revision.clone(),
        model_key: e.model_key.into(),
        source: e.source.clone(),
        chunk_index: e.index,
        local_start_ms: e.start_ms,
        audio_completed_ms: e.completed_ms,
        decoded_ms: e.decoded_ms,
        segments: e.segments.into(),
        raw: e.raw.into(),
    };
    let payload = serde_json::value::to_raw_value(&chunk).map_err(CapabilityError::new)?;
    let envelope = Envelope {
        version: 2,
        sha256: hex::encode(Sha256::digest(payload.get().as_bytes())),
        payload,
    };
    let directory = directory(root, &e.run.id)?;
    std::fs::create_dir_all(&directory).map_err(CapabilityError::new)?;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .map_err(CapabilityError::new)?;
    let name = format!("chunk-{:04}.json", e.index);
    let path = directory.join(&name);
    let partial = directory.join(format!("{name}.partial"));
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&partial)
        .map_err(CapabilityError::new)?;
    let mut file = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut file, &envelope).map_err(CapabilityError::new)?;
    file.flush().map_err(CapabilityError::new)?;
    file.get_ref().sync_all().map_err(CapabilityError::new)?;
    std::fs::hard_link(&partial, &path).map_err(CapabilityError::new)?;
    std::fs::remove_file(&partial).map_err(CapabilityError::new)?;
    std::fs::File::open(&directory)
        .and_then(|f| f.sync_all())
        .map_err(CapabilityError::new)?;
    Ok(())
}
pub struct ResumeInput<'a> {
    pub root: &'a Path,
    pub request: &'a RunProcessingInput,
    pub track: &'a AudioTrack,
    pub run: &'a ProcessingRun,
    pub source: &'a SourceIdentity,
    pub model_key: &'a str,
    pub duration_ms: i64,
    pub control: &'a ProcessingControl,
}
pub struct Prefix {
    pub consumed_ms: i64,
    pub decoded_ms: i64,
    pub chunks: usize,
    pub segments: Vec<TranscriptSegment>,
    pub raw: Vec<Value>,
    pub report: Value,
}
impl Prefix {
    fn empty(status: &str, reason: Option<String>) -> Self {
        Self {
            consumed_ms: 0,
            decoded_ms: 0,
            chunks: 0,
            segments: vec![],
            raw: vec![],
            report: json!({"status":status,"reason":reason,"reused_chunks":0,"reused_audio_ms":0}),
        }
    }
}
pub fn restore(input: ResumeInput<'_>) -> Result<Prefix, CapabilityError> {
    if !input.control.retains_work() {
        return Ok(Prefix::empty("unobserved", None));
    }
    let Some(prior) = input.run.configuration["retry_of_run_id"].as_str() else {
        return Ok(Prefix::empty("new_run", None));
    };
    let chunks = match read_compatible(&input, prior) {
        Ok(chunks) if !chunks.is_empty() => chunks,
        Ok(_) => {
            return Ok(Prefix::empty(
                "unavailable",
                Some("No completed compatible chunks".into()),
            ));
        }
        Err(error) => {
            input.control.check()?;
            return Ok(Prefix::empty(
                "restart_required",
                Some(error.to_string().chars().take(1024).collect()),
            ));
        }
    };
    let mut prefix = Prefix::empty("reused", None);
    for mut chunk in chunks {
        input.control.check()?;
        for segment in &mut chunk.segments {
            segment.id = stable_id("segment", &format!("{}:{}", input.run.id, segment.ordinal));
            segment.run_id = input.run.id.clone();
        }
        // Rehome validated evidence so a second interruption never needs an
        // unbounded chain of ancestors. Publish no DB transcript until success.
        retain_chunk(
            input.root,
            ChunkEvidence {
                request: input.request,
                track: input.track,
                run: input.run,
                source: input.source,
                model_key: input.model_key,
                index: chunk.chunk_index,
                start_ms: chunk.local_start_ms,
                completed_ms: chunk.audio_completed_ms,
                decoded_ms: chunk.decoded_ms,
                segments: &chunk.segments,
                raw: &chunk.raw,
            },
        )?;
        // FFmpeg can trim a few codec-padding milliseconds without changing
        // the next requested window origin. Preserve both clocks exactly.
        prefix.consumed_ms += (input.duration_ms - prefix.consumed_ms).min(60000);
        prefix.decoded_ms += chunk.decoded_ms;
        prefix.chunks += 1;
        prefix.segments.extend(chunk.segments);
        prefix.raw.extend(chunk.raw);
        input.control.publish(
            ProcessingStage::RestoringChunks,
            chunk.audio_completed_ms,
            Some(input.duration_ms),
            prefix.chunks,
            None,
        );
    }
    prefix.report = json!({"status":"reused","source_run_id":prior,"reused_chunks":prefix.chunks,"reused_audio_ms":prefix.decoded_ms,"reused_timeline_ms":prefix.consumed_ms,"source_sha256":input.source.sha256,"protocol":2,"runtime":RUNTIME});
    Ok(prefix)
}
fn read_compatible(input: &ResumeInput<'_>, prior: &str) -> Result<Vec<Chunk>, CapabilityError> {
    let directory = directory(input.root, prior)?;
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(CapabilityError::new(e)),
    };
    let mut files = vec![];
    for entry in entries {
        let entry = entry.map_err(CapabilityError::new)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".partial") {
            continue;
        }
        if !entry.file_type().map_err(CapabilityError::new)?.is_file()
            || !name.starts_with("chunk-")
            || !name.ends_with(".json")
        {
            return Err(CapabilityError::message(
                "Unexpected checkpoint entry; refusing reuse",
            ));
        }
        files.push(entry.path());
        if files.len() > 120 {
            return Err(CapabilityError::message(
                "Checkpoint count exceeds two-hour bound",
            ));
        }
    }
    files.sort();
    let mut chunks = vec![];
    let mut consumed = 0;
    let mut ordinal = 0;
    let mut bytes = 0u64;
    for (index, path) in files.into_iter().enumerate() {
        input.control.check()?;
        if path.file_name().and_then(|n| n.to_str())
            != Some(format!("chunk-{index:04}.json").as_str())
        {
            return Err(CapabilityError::message(
                "Completed checkpoints are not a contiguous prefix",
            ));
        }
        let length = std::fs::metadata(&path)
            .map_err(CapabilityError::new)?
            .len();
        bytes += length;
        if length > 8 * 1024 * 1024 || bytes > 64 * 1024 * 1024 {
            return Err(CapabilityError::message(
                "Checkpoint data exceeds resume bound",
            ));
        }
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(CapabilityError::new)?;
        let mut content = Vec::new();
        file.take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut content)
            .map_err(CapabilityError::new)?;
        if content.len() > 8 * 1024 * 1024 || content.len() as u64 != length {
            return Err(CapabilityError::message(
                "Checkpoint size changed while reading",
            ));
        }
        let envelope: Envelope = serde_json::from_slice(&content).map_err(CapabilityError::new)?;
        if envelope.version != 2
            || envelope.sha256 != hex::encode(Sha256::digest(envelope.payload.get().as_bytes()))
        {
            return Err(CapabilityError::message(
                "Checkpoint version or checksum is incompatible",
            ));
        }
        let chunk: Chunk =
            serde_json::from_str(envelope.payload.get()).map_err(CapabilityError::new)?;
        let length = (input.duration_ms - consumed).min(60000);
        if length <= 0
            || chunk.runtime != RUNTIME
            || chunk.run_id != prior
            || chunk.chunk_index != index
            || chunk.model_revision != input.run.model_revision
            || chunk.model_key != input.model_key
            || chunk.source != *input.source
            || chunk.track != *input.track
            || serde_json::to_value(&chunk.request).map_err(CapabilityError::new)?
                != serde_json::to_value(input.request).map_err(CapabilityError::new)?
            || chunk.local_start_ms != input.request.start_ms + consumed
            || chunk.decoded_ms < (length - 100).max(1)
            || chunk.decoded_ms > length + 100
            || chunk.audio_completed_ms != (consumed + chunk.decoded_ms).min(input.duration_ms)
            || chunk.segments.len() != chunk.raw.len()
        {
            return Err(CapabilityError::message(
                "Checkpoint source, model, settings or timing changed; refusing reuse",
            ));
        }
        for (segment, raw) in chunk.segments.iter().zip(&chunk.raw) {
            segment.validate().map_err(CapabilityError::new)?;
            let local_start = segment.evidence["local_start_ms"].as_i64();
            let local_end = segment.evidence["local_end_ms"].as_i64();
            if ordinal >= 120000
                || segment.id != stable_id("segment", &format!("{prior}:{ordinal}"))
                || segment.ordinal != ordinal
                || segment.run_id != prior
                || segment.recording_id != input.run.recording_id
                || segment.track_id.as_deref() != Some(input.track.id.as_str())
                || segment.speaker_claim.is_some()
                || raw["text"] != segment.text
                || raw["evidence"] != segment.evidence
                || !matches!((local_start,local_end),(Some(a),Some(b)) if a>=chunk.local_start_ms && b>=a && b<=chunk.local_start_ms+chunk.decoded_ms)
                || segment.start_ms
                    != input
                        .track
                        .meeting_offset_ms()
                        .zip(local_start)
                        .map(|(a, b)| a + b)
                || segment.end_ms
                    != input
                        .track
                        .meeting_offset_ms()
                        .zip(local_end)
                        .map(|(a, b)| a + b)
            {
                return Err(CapabilityError::message(
                    "Checkpoint segment ownership or audio-time evidence is invalid",
                ));
            }
            ordinal += 1;
        }
        consumed += length;
        chunks.push(chunk);
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        tmp: tempfile::TempDir,
        request: RunProcessingInput,
        track: AudioTrack,
        old: ProcessingRun,
        source: SourceIdentity,
    }
    impl Fixture {
        fn new(offset: Option<i64>) -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("source.wav");
            std::fs::write(&path, b"deterministic local source identity").unwrap();
            let track = AudioTrack {
                id: "track".into(),
                recording_id: "recording".into(),
                kind: parakeet_domain::TrackKind::File,
                path: path.to_str().unwrap().into(),
                start_offset_ms: offset,
                sample_rate: Some(16000),
                duration_ms: Some(125000),
                available: true,
                size_bytes: None,
                modified_unix_ms: None,
                provenance: json!({}),
            };
            let source = identify_source(&track, &ProcessingControl::default()).unwrap();
            Self {
                tmp,
                request: RunProcessingInput {
                    recording_id: "recording".into(),
                    track_id: "track".into(),
                    model: "fixture".into(),
                    start_ms: 1000,
                    duration_ms: Some(120000),
                    word_timestamps: false,
                },
                track,
                old: ProcessingRun {
                    id: uuid::Uuid::new_v4().to_string(),
                    recording_id: "recording".into(),
                    kind: "transcription".into(),
                    model: "fixture".into(),
                    model_revision: "revision".into(),
                    status: "interrupted".into(),
                    started_at: "test".into(),
                    completed_at: Some("test".into()),
                    configuration: json!({}),
                    result: Value::Null,
                    error: None,
                },
                source,
            }
        }
        fn save(&self, index: usize) -> Result<(), CapabilityError> {
            let start = self.request.start_ms + index as i64 * 60000;
            let evidence = json!({"local_start_ms":start,"local_end_ms":start+1000,"attribution":"unknown","no_speech_probability":1.8694359837923002e-8});
            let text = format!("Chunk {index} café");
            let segment = TranscriptSegment {
                id: stable_id("segment", &format!("{}:{index}", self.old.id)),
                recording_id: self.old.recording_id.clone(),
                run_id: self.old.id.clone(),
                track_id: Some(self.track.id.clone()),
                ordinal: index,
                start_ms: self.track.meeting_offset_ms().map(|o| o + start),
                end_ms: self.track.meeting_offset_ms().map(|o| o + start + 1000),
                text: text.clone(),
                speaker_claim: None,
                evidence: evidence.clone(),
            };
            retain_chunk(
                self.tmp.path(),
                ChunkEvidence {
                    request: &self.request,
                    track: &self.track,
                    run: &self.old,
                    source: &self.source,
                    model_key: "key",
                    index,
                    start_ms: start,
                    completed_ms: (index as i64 + 1) * 60000,
                    decoded_ms: 60000,
                    segments: &[segment],
                    raw: &[json!({"text":text,"evidence":evidence})],
                },
            )
        }
        fn next(&self) -> ProcessingRun {
            let mut run = self.old.clone();
            run.id = uuid::Uuid::new_v4().to_string();
            run.status = "running".into();
            run.completed_at = None;
            run.configuration = json!({"retry_of_run_id":self.old.id});
            run
        }
        fn restore(&self, run: &ProcessingRun) -> Prefix {
            restore(ResumeInput {
                root: self.tmp.path(),
                request: &self.request,
                track: &self.track,
                run,
                source: &self.source,
                model_key: "key",
                duration_ms: 120000,
                control: &ProcessingControl::default(),
            })
            .unwrap()
        }
        fn path(&self, index: usize) -> PathBuf {
            directory(self.tmp.path(), &self.old.id)
                .unwrap()
                .join(format!("chunk-{index:04}.json"))
        }
    }
    fn modify(path: &Path, change: impl FnOnce(&mut Value)) {
        let mut envelope: Envelope = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let mut payload: Value = serde_json::from_str(envelope.payload.get()).unwrap();
        change(&mut payload);
        envelope.payload = serde_json::value::to_raw_value(&payload).unwrap();
        envelope.sha256 = hex::encode(Sha256::digest(envelope.payload.get().as_bytes()));
        std::fs::write(path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    }
    #[test]
    fn private_durable_chunks_resume_with_new_ids_and_survive_another_interruption() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new(Some(2800));
        f.save(0).unwrap();
        f.save(1).unwrap();
        let original = std::fs::read(f.path(0)).unwrap();
        assert_eq!(
            std::fs::metadata(f.path(0)).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(f.path(0).parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(!f.path(0).with_file_name("chunk-0000.json.partial").exists());
        let run = f.next();
        let prefix = f.restore(&run);
        assert_eq!(prefix.report["status"], "reused");
        assert_eq!(prefix.chunks, 2);
        assert_eq!(prefix.consumed_ms, 120000);
        assert_eq!(prefix.segments.len(), 2);
        assert_eq!(prefix.segments[0].start_ms, Some(3800));
        assert_eq!(
            prefix.segments[0].id,
            stable_id("segment", &format!("{}:0", run.id))
        );
        assert_eq!(prefix.segments[0].run_id, run.id);
        assert_eq!(prefix.raw[0]["text"], "Chunk 0 café");
        assert_eq!(std::fs::read(f.path(0)).unwrap(), original);
        let mut second = f.next();
        second.configuration["retry_of_run_id"] = json!(run.id);
        assert_eq!(f.restore(&second).chunks, 2);
        assert!(f.save(0).is_err());
        assert_eq!(std::fs::read(f.path(0)).unwrap(), original);
        assert!(directory(f.tmp.path(), "../outside").is_err());
    }
    #[test]
    fn checksums_model_source_settings_timing_and_ownership_are_required() {
        let f = Fixture::new(Some(2800));
        f.save(0).unwrap();
        let path = f.path(0);
        let original = std::fs::read(&path).unwrap();
        let mutations: [fn(&mut Value); 11] = [
            |p| p["model_key"] = json!("changed"),
            |p| p["model_revision"] = json!("changed"),
            |p| p["runtime"] = json!("changed"),
            |p| p["source"]["sha256"] = json!("changed"),
            |p| p["request"]["word_timestamps"] = json!(true),
            |p| p["track"]["start_offset_ms"] = json!(999),
            |p| p["decoded_ms"] = json!(59000),
            |p| p["segments"][0]["id"] = json!("foreign"),
            |p| p["segments"][0]["speaker_claim"] = json!("Me"),
            |p| p["raw"][0]["text"] = json!("changed"),
            |p| p["segments"][0]["start_ms"] = json!(999),
        ];
        for change in mutations {
            std::fs::write(&path, &original).unwrap();
            modify(&path, change);
            let p = f.restore(&f.next());
            assert_eq!(p.report["status"], "restart_required");
            assert_eq!(p.chunks, 0);
            assert!(p.segments.is_empty());
        }
        let mut envelope: Envelope = serde_json::from_slice(&original).unwrap();
        envelope.sha256 = "corrupt".into();
        std::fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert_eq!(f.restore(&f.next()).chunks, 0);
        std::fs::write(path, b"{\"version\":1}").unwrap();
        assert_eq!(f.restore(&f.next()).report["status"], "restart_required");
    }
    #[test]
    fn partial_files_are_ignored_but_gaps_symlinks_and_excess_data_restart() {
        let f = Fixture::new(Some(0));
        f.save(0).unwrap();
        f.save(1).unwrap();
        let original = std::fs::read(f.path(0)).unwrap();
        std::fs::write(
            f.path(0).with_file_name("chunk-0002.json.partial"),
            b"unfinished",
        )
        .unwrap();
        assert_eq!(f.restore(&f.next()).chunks, 2);
        std::fs::remove_file(f.path(0)).unwrap();
        assert_eq!(f.restore(&f.next()).report["status"], "restart_required");
        let other = f.tmp.path().join("other.json");
        std::fs::write(&other, &original).unwrap();
        std::os::unix::fs::symlink(&other, f.path(0)).unwrap();
        assert_eq!(f.restore(&f.next()).chunks, 0);
        std::fs::remove_file(f.path(0)).unwrap();
        std::fs::write(f.path(0), vec![b' '; 8 * 1024 * 1024 + 1]).unwrap();
        assert_eq!(f.restore(&f.next()).chunks, 0);
        let mut first = f.next();
        first.configuration = json!({});
        assert_eq!(f.restore(&first).report["status"], "new_run");
    }
    #[test]
    fn source_content_is_rechecked_and_cancellation_stops_hashing() {
        let mut f = Fixture::new(Some(0));
        let old = f.source.clone();
        std::fs::write(&f.track.path, b"changed local source").unwrap();
        assert_ne!(
            identify_source(&f.track, &ProcessingControl::default()).unwrap(),
            old
        );
        f.track.provenance["content_sha256"] = json!(old.sha256);
        assert!(identify_source(&f.track, &ProcessingControl::default()).is_err());
        let control = ProcessingControl::default();
        control.request_cancellation();
        assert!(identify_source(&f.track, &control).is_err());
    }
    #[test]
    fn codec_padding_preserves_the_requested_next_window_and_actual_audio_length() {
        let f = Fixture::new(Some(2800));
        f.save(0).unwrap();
        modify(&f.path(0), |p| {
            p["decoded_ms"] = json!(59988);
            p["audio_completed_ms"] = json!(59988);
        });
        let prefix = f.restore(&f.next());
        assert_eq!(prefix.chunks, 1);
        assert_eq!(prefix.consumed_ms, 60000);
        assert_eq!(prefix.decoded_ms, 59988);
        assert_eq!(prefix.report["reused_audio_ms"], 59988);
        assert_eq!(prefix.report["reused_timeline_ms"], 60000);
        assert_eq!(prefix.segments[0].start_ms, Some(3800));
    }
    #[test]
    fn unknown_meeting_origin_remains_unknown_in_reused_segments() {
        let f = Fixture::new(None);
        f.save(0).unwrap();
        let prefix = f.restore(&f.next());
        assert_eq!(prefix.chunks, 1);
        assert_eq!(prefix.segments[0].start_ms, None);
        assert_eq!(prefix.segments[0].end_ms, None);
        assert_eq!(prefix.segments[0].evidence["local_start_ms"], 1000);
    }
}
