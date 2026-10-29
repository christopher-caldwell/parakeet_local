use crate::{audio, models::ModelFiles};
use parakeet_application::{processing::*, *};
use parakeet_domain::*;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub struct NemotronJob<'a> {
    pub runtime: &'a tokio::runtime::Handle,
    pub models: &'a ModelFiles,
    pub request: &'a RunProcessingInput,
    pub track: &'a AudioTrack,
    pub run: &'a ProcessingRun,
    pub duration: i64,
    pub started: Instant,
    pub executable: &'a std::path::Path,
    pub control: &'a parakeet_application::processing_control::ProcessingControl,
}
/// This job's derived PCM and request files, removed however the job ends (success, error or
/// cancellation). Original sources and raw model evidence are never inside it.
struct JobScratch(std::path::PathBuf);
impl JobScratch {
    fn create(path: std::path::PathBuf) -> Result<Self, CapabilityError> {
        std::fs::create_dir_all(&path).map_err(CapabilityError::new)?;
        Ok(Self(path))
    }
}
impl Drop for JobScratch {
    fn drop(&mut self) {
        for name in ["audio.f32", "speech.f32", "request.json"] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}
pub fn nemotron(job: NemotronJob<'_>) -> Result<EngineOutput, CapabilityError> {
    let NemotronJob {
        runtime,
        models,
        request,
        track,
        run,
        duration,
        started,
        executable,
        control,
    } = job;

    let job_scratch = JobScratch::create(models.root.join("work").join(&run.id))?;
    let scratch = job_scratch.0.clone();
    let pcm = scratch.join("audio.f32");
    use parakeet_application::processing_control::ProcessingStage;
    control.check()?;
    control.publish(ProcessingStage::Decoding, 0, Some(duration), 0, None);
    let decoded = Instant::now();
    let samples = audio::decode_to_file(track, request.start_ms, duration, &pcm);
    let samples=runtime.block_on(async {tokio::select!{r=samples=>r,_=control.cancelled()=>Err(CapabilityError::message("PCM preparation cancelled"))}})?;
    let decode_seconds = decoded.elapsed().as_secs_f64();
    let audio_seconds = samples as f64 / 16000.;
    let directory =
        std::fs::canonicalize(models.directory(&request.model)?).map_err(CapabilityError::new)?;
    let options = scratch.join("request.json");
    std::fs::write(&options,serde_json::to_vec(&json!({"directory":directory,"audio_f32":std::fs::canonicalize(&pcm).map_err(CapabilityError::new)?,"preset":if request.model=="nemotron3-offline"{"offline"}else{"fast128"}})).map_err(CapabilityError::new)?).map_err(CapabilityError::new)?;
    control.check()?;
    control.publish(ProcessingStage::LoadingModel, 0, Some(duration), 0, None);
    let output = runtime.block_on(supervised_runtime(
        executable,
        &options,
        duration,
        control,
        ProcessingStage::Diarizing,
    ))?;
    let raw: Value = serde_json::from_slice(&output).map_err(CapabilityError::new)?;
    let mut intervals = vec![];
    if let Some(offset) = track.meeting_offset_ms() {
        for item in raw["intervals"]
            .as_array()
            .ok_or_else(|| CapabilityError::message("Nemotron runtime omitted intervals"))?
        {
            let local_start = item["start_ms"]
                .as_i64()
                .ok_or_else(|| CapabilityError::message("invalid local start"))?;
            let local_end = item["end_ms"]
                .as_i64()
                .ok_or_else(|| CapabilityError::message("invalid local end"))?;
            if local_start < 0
                || local_end <= local_start
                || local_start as f64 > audio_seconds * 1000.
            {
                continue;
            }
            intervals.push(SpeakerInterval {
                start_ms: offset + request.start_ms + local_start,
                end_ms: offset
                    + request.start_ms
                    + local_end.min((audio_seconds * 1000.).ceil() as i64),
                speaker: format!(
                    "{}:{}",
                    track.id,
                    item["speaker"].as_str().unwrap_or("unknown")
                ),
                track_id: Some(track.id.clone()),
            });
        }
    }
    intervals.sort_by(|a, b| a.start_ms.cmp(&b.start_ms).then(a.speaker.cmp(&b.speaker)));
    control.publish(
        ProcessingStage::Finishing,
        (samples as i64 / 16).min(duration),
        Some(duration),
        0,
        None,
    );
    let wall = started.elapsed().as_secs_f64();
    let mut helper_metrics = raw.clone();
    if let Some(object) = helper_metrics.as_object_mut() {
        object.remove("intervals");
    }
    let metrics = json!({"wall_seconds":wall,"model_load_seconds":raw["model_load_seconds"],"decode_seconds":decode_seconds,"decode_storage":"streamed_f32_disk_v1","pcm_bytes":samples*4,"audio_seconds":audio_seconds,"real_time_factor":wall/audio_seconds,"helper_metrics":helper_metrics,"overlap_preserved":true,"timing_space":if track.meeting_offset_ms().is_some(){"meeting"}else{"track_local_only_in_raw"},"embedding_capability":false});
    drop(job_scratch);
    Ok(EngineOutput {
        segments: vec![],
        intervals,
        raw,
        metrics,
    })
}

async fn supervised_runtime(
    executable: &std::path::Path,
    options: &std::path::Path,
    duration: i64,
    control: &parakeet_application::processing_control::ProcessingControl,
    stage: parakeet_application::processing_control::ProcessingStage,
) -> Result<Vec<u8>, CapabilityError> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    let mut child = tokio::process::Command::new(executable)
        .arg(options)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(CapabilityError::new)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CapabilityError::message("runtime stdout missing"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| CapabilityError::message("runtime stderr missing"))?;
    let read_output = tokio::spawn(async move {
        let mut bytes = vec![];
        stdout
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await?;
        Ok::<_, std::io::Error>(bytes)
    });
    let observer = control.clone();
    let read_errors = tokio::spawn(async move {
        let mut errors = vec![];
        let mut line = vec![];
        let mut block = [0u8; 4096];
        let mut oversized = false;
        loop {
            let n = stderr.read(&mut block).await?;
            if n == 0 {
                break;
            }
            for byte in &block[..n] {
                if *byte == b'\n' {
                    if !oversized {
                        if let Ok(v) = serde_json::from_slice::<Value>(&line) {
                            if v["event"] == "inference_progress" {
                                if let (Some(done), Some(total)) =
                                    (v["completed_ms"].as_i64(), v["total_ms"].as_i64())
                                {
                                    observer.publish(
                                        stage,
                                        done.min(duration),
                                        Some(total.min(duration)),
                                        0,
                                        None,
                                    );
                                }
                            } else if errors.len() + line.len() < 16384 {
                                errors.extend_from_slice(&line);
                            }
                        } else if errors.len() + line.len() < 16384 {
                            errors.extend_from_slice(&line);
                        }
                    }
                    line.clear();
                    oversized = false;
                } else if line.len() < 8192 {
                    line.push(*byte);
                } else {
                    oversized = true;
                }
            }
        }
        Ok::<_, std::io::Error>(errors)
    });
    let mut cancelled = false;
    let mut timed_out = false;
    let status = tokio::select! {
        r=child.wait()=>r.map_err(CapabilityError::new)?,
        _=control.cancelled()=>{cancelled=true;child.kill().await.map_err(CapabilityError::new)?;child.wait().await.map_err(CapabilityError::new)?},
        _=tokio::time::sleep(Duration::from_secs(600+duration as u64/500))=>{timed_out=true;child.kill().await.map_err(CapabilityError::new)?;child.wait().await.map_err(CapabilityError::new)?},
    };
    let bytes = read_output
        .await
        .map_err(CapabilityError::new)?
        .map_err(CapabilityError::new)?;
    let errors = read_errors
        .await
        .map_err(CapabilityError::new)?
        .map_err(CapabilityError::new)?;
    if cancelled {
        return Err(CapabilityError::message(
            "owned diarization runtime stopped after cancellation",
        ));
    }
    if timed_out {
        return Err(CapabilityError::message(
            "owned diarization runtime timed out and stopped",
        ));
    }
    if !status.success() || bytes.len() > 32 * 1024 * 1024 {
        return Err(CapabilityError::message(format!(
            "local Nemotron runtime failed: {} {}",
            String::from_utf8_lossy(&bytes[..bytes.len().min(16384)]),
            String::from_utf8_lossy(&errors)
        )));
    }
    Ok(bytes)
}

pub struct ParakeetJob<'a> {
    pub runtime: &'a tokio::runtime::Handle,
    pub models: &'a ModelFiles,
    pub request: &'a RunProcessingInput,
    pub track: &'a AudioTrack,
    pub run: &'a ProcessingRun,
    pub duration: i64,
    pub started: Instant,
    pub executable: &'a std::path::Path,
    pub control: &'a parakeet_application::processing_control::ProcessingControl,
}
/// Decoded in blocks so a long recording never sits in memory whole.
const BLOCK_MS: i64 = 600_000;

/// Parakeet TDT over the same speech-aligned chunks Whisper uses. Rust cuts and offsets; the
/// helper only turns each chunk's samples into sentences.
pub fn parakeet(job: ParakeetJob<'_>) -> Result<EngineOutput, CapabilityError> {
    use parakeet_application::processing_control::ProcessingStage;
    use std::io::Write;
    let ParakeetJob {
        runtime,
        models,
        request,
        track,
        run,
        duration,
        started,
        executable,
        control,
    } = job;
    let job_scratch = JobScratch::create(models.root.join("work").join(&run.id))?;
    let scratch = job_scratch.0.clone();
    let pcm = scratch.join("speech.f32");
    let mut writer =
        std::io::BufWriter::new(std::fs::File::create(&pcm).map_err(CapabilityError::new)?);
    let (mut chunks, mut origins, mut rms_values) = (vec![], vec![], vec![]);
    let (mut file_samples, mut audio_seconds, mut speech_seconds) = (0usize, 0., 0.);
    let decoding = Instant::now();
    let mut position = 0i64;
    while position < duration {
        control.check()?;
        control.publish(ProcessingStage::Decoding, position, Some(duration), 0, None);
        let length = (duration - position).min(BLOCK_MS);
        let samples = runtime.block_on(async {
            tokio::select! {
                r = audio::decode(track, request.start_ms + position, length) => r,
                _ = control.cancelled() => Err(CapabilityError::message("decoding cancelled")),
            }
        })?;
        if samples.is_empty() {
            break;
        }
        audio_seconds += samples.len() as f64 / 16000.;
        let last_block =
            position + length >= duration || (samples.len() as i64) < length * 16 - 1600;
        let (planned, consumed) = crate::vad::plan_chunks(&samples, last_block);
        for chunk in &planned {
            let piece = &samples[chunk.start..chunk.end];
            for sample in piece {
                writer
                    .write_all(&sample.to_le_bytes())
                    .map_err(CapabilityError::new)?;
            }
            chunks.push(json!({"file_start": file_samples, "samples": piece.len()}));
            origins.push(position + chunk.start as i64 / 16);
            rms_values.push(
                (piece.iter().map(|s| (*s as f64).powi(2)).sum::<f64>()
                    / piece.len().max(1) as f64)
                    .sqrt(),
            );
            file_samples += piece.len();
            speech_seconds += chunk.speech as f64 / 16000.;
        }
        if last_block {
            break;
        }
        position += (consumed as i64 / 16).max(1_000);
    }
    writer.flush().map_err(CapabilityError::new)?;
    drop(writer);
    let decode_seconds = decoding.elapsed().as_secs_f64();
    let offset = track.meeting_offset_ms();
    let mut segments: Vec<TranscriptSegment> = vec![];
    let mut raw: Vec<Value> = vec![];
    let mut helper_metrics = json!({});
    if !chunks.is_empty() {
        let directory = std::fs::canonicalize(models.directory(&request.model)?)
            .map_err(CapabilityError::new)?;
        let options = scratch.join("request.json");
        std::fs::write(
            &options,
            serde_json::to_vec(&json!({
                "task": "asr",
                "directory": directory,
                "audio_f32": std::fs::canonicalize(&pcm).map_err(CapabilityError::new)?,
                "chunks": chunks,
            }))
            .map_err(CapabilityError::new)?,
        )
        .map_err(CapabilityError::new)?;
        control.check()?;
        control.publish(ProcessingStage::LoadingModel, 0, Some(duration), 0, None);
        let output = runtime.block_on(supervised_runtime(
            executable,
            &options,
            duration,
            control,
            ProcessingStage::Transcribing,
        ))?;
        let reply: Value = serde_json::from_slice(&output).map_err(CapabilityError::new)?;
        for item in reply["segments"]
            .as_array()
            .ok_or_else(|| CapabilityError::message("Parakeet runtime omitted segments"))?
        {
            let chunk = item["chunk"].as_u64().unwrap_or(u64::MAX) as usize;
            let (Some(origin), Some(text)) = (origins.get(chunk), item["text"].as_str()) else {
                return Err(CapabilityError::message(
                    "Parakeet runtime returned an invalid segment",
                ));
            };
            let local_start = request.start_ms + origin + item["start_ms"].as_i64().unwrap_or(0);
            let local_end = request.start_ms + origin + item["end_ms"].as_i64().unwrap_or(0);
            let rms = rms_values[chunk];
            let mut evidence = json!({"local_start_ms": local_start, "local_end_ms": local_end, "confidence": item["confidence"], "chunk": chunk, "chunk_rms": rms, "uncertain_speech": rms < 0.0003, "attribution": "unknown", "timing_quality": "model_token_derived"});
            if let Some(reason) = crate::whisper::non_speech_text(text) {
                evidence["suppressed"] = json!(reason);
                raw.push(json!({"text": "", "suppressed_text": text, "evidence": evidence}));
                continue;
            }
            raw.push(json!({"text": text, "evidence": evidence}));
            let ordinal = segments.len();
            segments.push(TranscriptSegment {
                id: stable_id("segment", &format!("{}:{ordinal}", run.id)),
                recording_id: run.recording_id.clone(),
                run_id: run.id.clone(),
                track_id: Some(track.id.clone()),
                ordinal,
                start_ms: offset.map(|o| o + local_start),
                end_ms: offset.map(|o| o + local_end.max(local_start)),
                text: text.to_string(),
                speaker_claim: None,
                evidence,
            });
        }
        helper_metrics = reply.clone();
        if let Some(object) = helper_metrics.as_object_mut() {
            object.remove("segments");
        }
    }
    drop(job_scratch);
    order_segments(&mut segments);
    control.publish(
        ProcessingStage::Finishing,
        duration,
        Some(duration),
        chunks.len(),
        None,
    );
    let wall = started.elapsed().as_secs_f64();
    Ok(EngineOutput {
        segments,
        intervals: vec![],
        raw: json!(raw),
        metrics: json!({
            "wall_seconds": wall,
            "model_load_seconds": helper_metrics["model_load_seconds"],
            "decode_seconds": decode_seconds,
            "audio_seconds": audio_seconds,
            "speech_seconds": speech_seconds,
            "skipped_non_speech_seconds": (audio_seconds - speech_seconds).max(0.),
            "real_time_factor": wall / audio_seconds.max(f64::EPSILON),
            "chunking": "energy-vad-speech-chunks-v1",
            "inference_chunks": chunks.len(),
            "helper_metrics": helper_metrics,
            "runtime_configuration": "fluidaudio-0.9.1-asr-subset:parakeet-tdt-0.6b-v2:speech-chunks",
            "experimental_word_timestamps": false,
            "timing_space": if offset.is_some() { "meeting" } else { "untimed_with_track_local_evidence" },
        }),
    })
}
