//! Scoped C callbacks. The context outlives the synchronous whisper full call.
use parakeet_application::{CapabilityError, processing::ProcessRequest, processing_control::*};
use parakeet_domain::{AudioTrack, ProcessingRun, TranscriptSegment};
use serde_json::{Value, json};
use std::{ffi::c_void, io::Write, path::Path};
pub struct Callbacks {
    pub control: ProcessingControl,
    pub consumed: i64,
    pub duration: i64,
    pub chunks: usize,
}
unsafe extern "C" fn abort(data: *mut c_void) -> bool {
    // SAFETY: install points at the caller-owned box, held through full().
    unsafe { (&*data.cast::<Callbacks>()).control.is_cancelled() }
}
unsafe extern "C" fn progress(
    _: *mut whisper_rs::whisper_rs_sys::whisper_context,
    _: *mut whisper_rs::whisper_rs_sys::whisper_state,
    percent: i32,
    data: *mut c_void,
) {
    // SAFETY: identical lifetime to abort; state access is synchronized.
    let c = unsafe { &*data.cast::<Callbacks>() };
    c.control.publish(
        ProcessingStage::Transcribing,
        c.consumed,
        Some(c.duration),
        c.chunks,
        Some(percent.clamp(0, 100) as u8),
    );
}
impl Callbacks {
    pub fn run(
        &self,
        state: &mut whisper_rs::WhisperState,
        mut params: whisper_rs::FullParams<'_, '_>,
        samples: &[f32],
    ) -> Result<(), whisper_rs::WhisperError> {
        let data = (self as *const Self).cast_mut().cast::<c_void>();
        // SAFETY: SDK callbacks borrow this box only during the synchronous full
        // invocation. No closure is transferred to or allocated by the SDK.
        unsafe {
            params.set_abort_callback(Some(abort));
            params.set_abort_callback_user_data(data);
            params.set_progress_callback(Some(progress));
            params.set_progress_callback_user_data(data);
        }
        state.full(params, samples)
    }
}
pub struct ChunkEvidence<'a> {
    pub request: &'a ProcessRequest,
    pub track: &'a AudioTrack,
    pub run: &'a ProcessingRun,
    pub index: usize,
    pub start_ms: i64,
    pub completed_ms: i64,
    pub segments: &'a [TranscriptSegment],
    pub raw: &'a [Value],
}
pub fn retain_chunk(root: &Path, e: ChunkEvidence<'_>) -> Result<(), CapabilityError> {
    let id = uuid::Uuid::parse_str(&e.run.id)
        .map_err(CapabilityError::new)?
        .to_string();
    let directory = root.join("work").join(id).join("whisper-chunks");
    std::fs::create_dir_all(&directory).map_err(CapabilityError::new)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .map_err(CapabilityError::new)?;
    let name = format!("chunk-{:04}.json", e.index);
    let path = directory.join(&name);
    let partial = directory.join(format!("{name}.partial"));
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&partial)
        .map_err(CapabilityError::new)?;
    let mut file = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut file,&json!({"version":1,"runtime":"whisper-rs-0.16.0:no-context-60s-v1","request":e.request,"track":e.track,"run_id":e.run.id,"model_revision":e.run.model_revision,"chunk_index":e.index,"local_start_ms":e.start_ms,"audio_completed_ms":e.completed_ms,"segments":e.segments,"raw":e.raw})).map_err(CapabilityError::new)?;
    file.flush().map_err(CapabilityError::new)?;
    file.get_ref().sync_all().map_err(CapabilityError::new)?;
    std::fs::hard_link(&partial, &path).map_err(CapabilityError::new)?;
    std::fs::remove_file(&partial).map_err(CapabilityError::new)?;
    std::fs::File::open(&directory)
        .and_then(|f| f.sync_all())
        .map_err(CapabilityError::new)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_chunk_is_private_durable_evidence_and_never_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let request = ProcessRequest {
            recording_id: "recording".into(),
            track_id: "track".into(),
            model: "fixture".into(),
            start_ms: 0,
            duration_ms: Some(60000),
            word_timestamps: false,
        };
        let track: AudioTrack = serde_json::from_value(json!({"id":"track","recording_id":"recording","kind":"file","path":"absent-test.wav","available":true,"provenance":{}})).unwrap();
        let mut run = ProcessingRun {
            id: uuid::Uuid::new_v4().to_string(),
            recording_id: "recording".into(),
            kind: "transcription".into(),
            model: "fixture".into(),
            model_revision: "revision".into(),
            status: "running".into(),
            started_at: "test".into(),
            completed_at: None,
            configuration: json!({}),
            result: Value::Null,
            error: None,
        };
        let save = |run: &ProcessingRun| {
            retain_chunk(
                tmp.path(),
                ChunkEvidence {
                    request: &request,
                    track: &track,
                    run,
                    index: 0,
                    start_ms: 0,
                    completed_ms: 60000,
                    segments: &[],
                    raw: &[],
                },
            )
        };
        save(&run).unwrap();
        let directory = tmp.path().join("work").join(&run.id).join("whisper-chunks");
        let path = directory.join("chunk-0000.json");
        let original = std::fs::read(&path).unwrap();
        let evidence: Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(evidence["model_revision"], "revision");
        assert_eq!(evidence["audio_completed_ms"], 60000);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(!directory.join("chunk-0000.json.partial").exists());
        assert!(save(&run).is_err());
        assert_eq!(std::fs::read(path).unwrap(), original);
        run.id = "../outside".into();
        assert!(save(&run).is_err());
        assert!(!tmp.path().join("outside").exists());
    }
    #[test]
    fn scoped_callbacks_report_progress_and_read_cancellation() {
        let c = Box::new(Callbacks {
            control: ProcessingControl::default(),
            consumed: 60000,
            duration: 120000,
            chunks: 1,
        });
        let p = (&*c as *const Callbacks).cast_mut().cast();
        unsafe {
            progress(std::ptr::null_mut(), std::ptr::null_mut(), 42, p);
            assert!(!abort(p));
        }
        assert_eq!(c.control.snapshot().current_chunk_percent, Some(42));
        c.control.request_cancellation();
        unsafe {
            assert!(abort(p));
        }
    }
}
