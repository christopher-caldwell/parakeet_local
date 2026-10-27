use async_trait::async_trait;
use parakeet_application::{capture::*, *};
use parakeet_domain::*;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

#[derive(Clone)]
pub struct NativeBridge {
    state: Arc<std::sync::Mutex<Value>>,
    pub executable: PathBuf,
    pub root: PathBuf,
}
impl NativeBridge {
    pub fn new(executable: PathBuf, root: PathBuf) -> Self {
        Self {
            executable,
            root,
            state: Arc::new(std::sync::Mutex::new(
                json!({"event":"idle","state":"idle"}),
            )),
        }
    }
    pub async fn query(&self, mode: &str) -> Result<Value, CapabilityError> {
        let output = tokio::time::timeout(
            Duration::from_secs(10),
            Command::new(&self.executable)
                .arg(mode)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(CapabilityError::new)?
        .map_err(CapabilityError::new)?;
        let v: Value = serde_json::from_slice(&output.stdout).map_err(CapabilityError::new)?;
        if !output.status.success() {
            return Err(CapabilityError::message(v.to_string()));
        }
        Ok(v)
    }
    pub async fn insert(
        &self,
        target: &gestures::InsertionTarget,
        text: &str,
    ) -> Result<parakeet_application::dictation::InsertionReport, CapabilityError> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(CapabilityError::new)?;
        let path = self.root.join(format!("insert-{}.json", new_id()));
        let mut v = serde_json::to_value(target).map_err(CapabilityError::new)?;
        v["text"] = json!(text);
        v["clipboard_recovery_directory"] = json!(self.root.join("clipboard-recovery"));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .await
            .map_err(CapabilityError::new)?;
        file.write_all(&serde_json::to_vec(&v).map_err(CapabilityError::new)?)
            .await
            .map_err(CapabilityError::new)?;
        drop(file);
        let output = tokio::time::timeout(
            Duration::from_secs(10),
            Command::new(&self.executable)
                .arg("insert")
                .arg(&path)
                .kill_on_drop(true)
                .output(),
        )
        .await;
        let _ = tokio::fs::remove_file(&path).await;
        let output = output
            .map_err(CapabilityError::new)?
            .map_err(CapabilityError::new)?;
        if !output.status.success() {
            return Err(CapabilityError::message(
                String::from_utf8_lossy(&output.stdout).to_string(),
            ));
        }
        let result: Value = serde_json::from_slice(&output.stdout).map_err(CapabilityError::new)?;
        if result["inserted"] != true {
            return Err(CapabilityError::message(
                "insertion did not report success; inspect original target before retrying",
            ));
        }
        serde_json::from_value(result).map_err(CapabilityError::new)
    }
}
#[async_trait]
impl parakeet_application::dictation::TextInsertionRecovery for NativeBridge {
    async fn recover(
        &self,
    ) -> Result<parakeet_application::dictation::InsertionRecoveryReport, CapabilityError> {
        let directory = self.root.join("clipboard-recovery");
        if !directory.exists() {
            return Ok(Default::default());
        }
        let output = tokio::time::timeout(
            Duration::from_secs(10),
            Command::new(&self.executable)
                .arg("clipboard-recover")
                .arg(directory)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(CapabilityError::new)?
        .map_err(CapabilityError::new)?;
        if !output.status.success() {
            return Err(CapabilityError::message(String::from_utf8_lossy(
                &output.stdout,
            )));
        }
        serde_json::from_slice(&output.stdout).map_err(CapabilityError::new)
    }
}
#[async_trait]
impl AudioCapture for NativeBridge {
    fn current_status(&self) -> Value {
        self.state
            .lock()
            .map(|v| v.clone())
            .unwrap_or_else(|_| json!({"event":"state_error"}))
    }
    async fn devices(&self) -> Result<Value, CapabilityError> {
        self.query("devices").await
    }
    async fn permissions(&self) -> Result<Value, CapabilityError> {
        self.query("permissions").await
    }
    async fn recoverable(&self) -> Result<Value, CapabilityError> {
        let mut sessions = vec![];
        let root = self.root.join("captures");
        if !root.exists() {
            return Ok(json!({"sessions":sessions}));
        }
        let mut directories = tokio::fs::read_dir(root)
            .await
            .map_err(CapabilityError::new)?;
        while let Some(entry) = directories
            .next_entry()
            .await
            .map_err(CapabilityError::new)?
        {
            if !entry
                .file_type()
                .await
                .map_err(CapabilityError::new)?
                .is_dir()
            {
                continue;
            }
            let directory = entry.path();
            let manifest = directory.join("manifest.json").exists();
            let metadata = directory.join("meeting-recording-metadata.json").exists();
            let has_audio = directory.join("microphone-raw.caf").exists()
                || directory.join("system-raw.caf").exists();
            sessions.push(json!({"directory":directory,"manifest":manifest,"metadata":metadata,"has_audio":has_audio,"state":if manifest&&metadata{"finalized_or_partial_inspect_capture_report"}else{"interrupted_recoverable_do_not_delete"}}));
        }
        Ok(json!({"sessions":sessions}))
    }
    async fn recover(&self, directory: &std::path::Path) -> Result<ImportBundle, CapabilityError> {
        let base =
            std::fs::canonicalize(self.root.join("captures")).map_err(CapabilityError::new)?;
        let directory = std::fs::canonicalize(directory).map_err(CapabilityError::new)?;
        if !directory.starts_with(&base) || directory == base {
            return Err(CapabilityError::message(
                "recovery is restricted to this application's capture directories; external recordings are never repaired",
            ));
        }
        let lease = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("capture.lock"))
            .map_err(CapabilityError::new)?;
        fs2::FileExt::try_lock_exclusive(&lease).map_err(|_| {
            CapabilityError::message("capture is active; recovery cannot touch a live session")
        })?;
        let options: Value = serde_json::from_slice(
            &tokio::fs::read(directory.join("session.json"))
                .await
                .map_err(CapabilityError::new)?,
        )
        .map_err(CapabilityError::new)?;
        let events = tokio::fs::read_to_string(directory.join("events.jsonl"))
            .await
            .unwrap_or_default();
        let events: Vec<Value> = events
            .lines()
            .filter_map(|s| serde_json::from_str(s).ok())
            .collect();
        let mut alignment = serde_json::Map::new();
        let mut maximum = None::<i64>;
        let mut unknown = false;
        let mut usable = 0;
        for source in ["microphone", "system"] {
            let path = directory.join(format!("{source}-raw.caf"));
            if !path.exists() {
                continue;
            }
            let probe = crate::audio::probe(&path).await?;
            let duration = probe["duration_ms"]
                .as_i64()
                .ok_or_else(|| CapabilityError::message("recovered audio has unknown duration"))?;
            if duration <= 0 {
                continue;
            }
            let first = events
                .iter()
                .find(|e| e["event"] == "first_buffer" && e["source"] == source);
            let offset = first.and_then(|e| e["start_offset_ms"].as_i64());
            if let Some(o) = offset {
                maximum = Some(maximum.unwrap_or(0).max(o + duration));
            } else {
                unknown = true;
            }
            let rate = probe["sample_rate"].as_u64().unwrap_or(48000);
            alignment.insert(source.into(),json!({"startOffsetMs":offset,"sampleRate":rate,"writtenFrameCount":duration as u64*rate/1000,"recoveredMediaDurationMs":duration,"gaps":"unknown_after_interruption"}));
            usable += 1;
        }
        if usable == 0 {
            return Err(CapabilityError::message(
                "no decodable retained audio; all files preserved",
            ));
        }
        let metadata = json!({"sourceAlignment":alignment,"requestedMicrophoneUID":options["microphone_uid"],"requestedSystemAudio":options["system_audio"],"recovered":true,"errors":["Interrupted capture: coverage and stream gaps require inspection"]});
        let manifest = json!({"schema":"com.parakeet.local.capture","schemaVersion":1,"meeting":{"id":options["id"],"title":options["title"],"createdAt":options["created_at"],"durationMs":if unknown{None}else{maximum}}});
        for (name, value) in [
            ("meeting-recording-metadata.json", metadata),
            ("manifest.json", manifest),
        ] {
            let path = directory.join(name);
            if path.exists() {
                let preserved = directory.join(format!("pre-recovery-{}-{name}", new_id()));
                tokio::fs::copy(&path, preserved)
                    .await
                    .map_err(CapabilityError::new)?;
            }
            let tmp = directory.join(format!("{name}.partial"));
            tokio::fs::write(
                &tmp,
                serde_json::to_vec_pretty(&value).map_err(CapabilityError::new)?,
            )
            .await
            .map_err(CapabilityError::new)?;
            tokio::fs::rename(tmp, path)
                .await
                .map_err(CapabilityError::new)?;
        }
        let mut discovery = crate::artifacts::MacParakeetArtifacts
            .discover(&directory)
            .await?;
        let mut bundle = discovery
            .recordings
            .pop()
            .ok_or_else(|| CapabilityError::message("recovery artifacts cannot be normalized"))?;
        bundle
            .warnings
            .push("Recovered partial capture; timeline gaps may be unknown".into());
        bundle.recording.metadata["state"] = json!("recovered_partial");
        Ok(bundle)
    }
    async fn record(
        &self,
        request: &CaptureRequest,
        stop: Arc<AtomicBool>,
    ) -> Result<ImportBundle, CapabilityError> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(CapabilityError::new)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("capture.lock"))
            .map_err(CapabilityError::new)?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|e| CapabilityError::message(format!("another process owns capture: {e}")))?;
        if let Ok(mut state) = self.state.lock() {
            *state = json!({"event":"starting","state":"starting","first_buffer_ready":false,"requested_microphone_uid":request.microphone_uid,"requested_system_audio":request.system_audio});
        }
        let id = new_id();
        let directory = self.root.join("captures").join(&id);
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(CapabilityError::new)?;
        let options = directory.join("session.json");
        let created_at = now();
        let value = json!({"id":id,"created_at":created_at,"directory":directory,"title":request.title,"microphone_uid":request.microphone_uid,"system_audio":request.system_audio,"maximum_seconds":request.maximum_seconds,"state":"starting"});
        tokio::fs::write(
            &options,
            serde_json::to_vec_pretty(&value).map_err(CapabilityError::new)?,
        )
        .await
        .map_err(CapabilityError::new)?;
        let mut child = Command::new(&self.executable)
            .arg("record")
            .arg(&options)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(CapabilityError::new)?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| CapabilityError::message("capture control pipe unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| CapabilityError::message("capture event pipe unavailable"))?;
        let event_path = directory.join("events.jsonl");
        let state = self.state.clone();
        let events = tokio::spawn(async move {
            let mut lines = BufReader::new(output).lines();
            let mut file = tokio::fs::File::create(event_path).await?;
            while let Some(line) = lines.next_line().await? {
                file.write_all(line.as_bytes()).await?;
                file.write_all(b"\n").await?;
                if let Ok(event) = serde_json::from_str::<Value>(&line)
                    && let Ok(mut latest) = state.lock()
                {
                    let name = event["event"].as_str().unwrap_or("unknown").to_owned();
                    let mut merged = latest.as_object().cloned().unwrap_or_default();
                    if let Some(fields) = event.as_object() {
                        for (key, value) in fields {
                            merged.insert(key.clone(), value.clone());
                        }
                    }
                    if name == "completed" || name == "failed" {
                        for source in ["microphone", "system"] {
                            if let Some(health) =
                                merged.get_mut(source).and_then(Value::as_object_mut)
                            {
                                health.insert("is_receiving".into(), json!(false));
                            }
                        }
                    }
                    let ready = ["microphone", "system"].iter().any(|source| {
                        merged
                            .get(*source)
                            .is_some_and(|health| health["is_receiving"] == true)
                    });
                    let state = match name.as_str() {
                        "first_buffer" => "recording",
                        "health" if ready => "recording",
                        "health" => "waiting_or_stalled",
                        "capture_started" => "starting",
                        "completed" => "completed",
                        "failed" => "failed",
                        "source_failed" => "source_failed",
                        _ => "starting",
                    };
                    merged.insert("state".into(), json!(state));
                    *latest = Value::Object(merged);
                }
                eprintln!("capture: {line}");
            }
            Ok::<_, std::io::Error>(())
        });
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(request.maximum_seconds + 20);
        let mut stopping = None;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(CapabilityError::new)? {
                break status;
            }
            if stop.load(Ordering::SeqCst) && stopping.is_none() {
                let _ = stdin.write_all(b"stop\n").await;
                stopping = Some(tokio::time::Instant::now());
            }
            if tokio::time::Instant::now() >= deadline
                || stopping.is_some_and(|time| time.elapsed() > Duration::from_secs(10))
            {
                let _ = child.kill().await;
                let _ = events.await;
                return Err(CapabilityError::message(format!(
                    "native capture did not settle; retained recovery audio in {}",
                    directory.display()
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        events
            .await
            .map_err(CapabilityError::new)?
            .map_err(CapabilityError::new)?;
        if !status.success() {
            let explanation = self
                .state
                .lock()
                .ok()
                .and_then(|s| s["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "native capture failed".into());
            return Err(CapabilityError::message(format!(
                "{explanation} ({status}); retained session in {}",
                directory.display()
            )));
        }
        let discovery = crate::artifacts::MacParakeetArtifacts
            .discover(&directory)
            .await?;
        let mut bundle = discovery.recordings.into_iter().next().ok_or_else(|| {
            CapabilityError::message("native capture did not produce readable artifacts")
        })?;
        let report = &bundle.recording.metadata["capture"]["sourceAlignment"];
        let has_real_frames = ["microphone", "system"].iter().any(|source| {
            report[source]["firstBufferReceived"] == true
                && report[source]["writtenFrameCount"]
                    .as_u64()
                    .is_some_and(|n| n > 0)
        });
        if !has_real_frames
            || !bundle
                .tracks
                .iter()
                .any(|t| t.available && t.duration_ms.is_some_and(|d| d > 0))
        {
            return Err(CapabilityError::message(
                "capture has no usable audio; retained session for recovery",
            ));
        }
        bundle.recording.metadata["state"] = json!(if bundle.warnings.is_empty() {
            "ready_for_transcription"
        } else {
            "partial_needs_inspection"
        });
        Ok(bundle)
    }
}
#[async_trait]
impl parakeet_application::dictation::TextInsertion for NativeBridge {
    async fn insert(
        &self,
        target: &gestures::InsertionTarget,
        text: &str,
    ) -> Result<parakeet_application::dictation::InsertionReport, CapabilityError> {
        NativeBridge::insert(self, target, text).await
    }
}

struct NativeShortcutStream {
    _lease: std::fs::File,
    child: tokio::process::Child,
    lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}
#[async_trait]
impl parakeet_application::dictation::ShortcutStream for NativeShortcutStream {
    async fn next_event(
        &mut self,
    ) -> Result<Option<parakeet_application::dictation::ShortcutEvent>, CapabilityError> {
        loop {
            let Some(line) = self.lines.next_line().await.map_err(CapabilityError::new)? else {
                let status = self.child.wait().await.map_err(CapabilityError::new)?;
                if !status.success() {
                    return Err(CapabilityError::message(format!(
                        "native global shortcuts failed: {status}"
                    )));
                }
                return Ok(None);
            };
            let v: Value = serde_json::from_str(&line).map_err(CapabilityError::new)?;
            if v["event"] == "failed" {
                return Err(CapabilityError::message(
                    v["message"].as_str().unwrap_or("native shortcut failure"),
                ));
            }
            if v["event"] == "hotkeys_ready" {
                continue;
            }
            return serde_json::from_value(v)
                .map(Some)
                .map_err(CapabilityError::new);
        }
    }
}
#[async_trait]
impl parakeet_application::dictation::GlobalShortcuts for NativeBridge {
    async fn listen(
        &self,
        bindings: &[gestures::Shortcut],
    ) -> Result<Box<dyn parakeet_application::dictation::ShortcutStream>, CapabilityError> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(CapabilityError::new)?;
        let lease = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("shortcuts.lock"))
            .map_err(CapabilityError::new)?;
        fs2::FileExt::try_lock_exclusive(&lease).map_err(|e| {
            CapabilityError::message(format!(
                "another process owns global dictation shortcuts: {e}"
            ))
        })?;
        let file = self.root.join(format!("hotkeys-{}.json", new_id()));
        tokio::fs::write(
            &file,
            serde_json::to_vec(bindings).map_err(CapabilityError::new)?,
        )
        .await
        .map_err(CapabilityError::new)?;
        let mut child = Command::new(&self.executable)
            .arg("hotkeys")
            .arg(&file)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(CapabilityError::new)?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| CapabilityError::message("native shortcut event stream unavailable"))?;
        Ok(Box::new(NativeShortcutStream {
            _lease: lease,
            child,
            lines: BufReader::new(output).lines(),
        }))
    }
}
#[derive(Default)]
pub struct StderrDictationObserver;
impl parakeet_application::dictation::DictationObserver for StderrDictationObserver {
    fn event(&self, event: Value) {
        eprintln!("dictation: {event}");
    }
}
