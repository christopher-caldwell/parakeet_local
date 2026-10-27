use crate::{audio, models::ModelFiles};
use parakeet_application::{processing::*, *};
use parakeet_domain::*;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub struct NemotronJob<'a> {
    pub runtime: &'a tokio::runtime::Handle,
    pub models: &'a ModelFiles,
    pub request: &'a ProcessRequest,
    pub track: &'a AudioTrack,
    pub run: &'a ProcessingRun,
    pub duration: i64,
    pub started: Instant,
    pub executable: &'a std::path::Path,
    pub control: &'a parakeet_application::processing_control::ProcessingControl,
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

    let scratch = models.root.join("work").join(&run.id);
    std::fs::create_dir_all(&scratch).map_err(CapabilityError::new)?;
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
    let output = runtime.block_on(supervised_runtime(executable, &options, duration, control))?;
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
    // Only this job's derived PCM scratch is removed. Original sources and raw model evidence remain intact.
    let _ = std::fs::remove_file(&pcm);
    let _ = std::fs::remove_file(&options);
    let _ = std::fs::remove_dir(&scratch);
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
) -> Result<Vec<u8>, CapabilityError> {
    use parakeet_application::processing_control::ProcessingStage;
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
                                        ProcessingStage::Diarizing,
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
