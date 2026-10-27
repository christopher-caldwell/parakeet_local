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
    } = job;

    let scratch = models.root.join("work").join(&run.id);
    std::fs::create_dir_all(&scratch).map_err(CapabilityError::new)?;
    let pcm = scratch.join("audio.f32");
    let decoded = Instant::now();
    let samples = runtime.block_on(audio::decode_to_file(
        track,
        request.start_ms,
        duration,
        &pcm,
    ))?;
    let decode_seconds = decoded.elapsed().as_secs_f64();
    let audio_seconds = samples as f64 / 16000.;
    let directory =
        std::fs::canonicalize(models.directory(&request.model)?).map_err(CapabilityError::new)?;
    let options = scratch.join("request.json");
    std::fs::write(&options,serde_json::to_vec(&json!({"directory":directory,"audio_f32":std::fs::canonicalize(&pcm).map_err(CapabilityError::new)?,"preset":if request.model=="nemotron3-offline"{"offline"}else{"fast128"}})).map_err(CapabilityError::new)?).map_err(CapabilityError::new)?;
    let output = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(600 + duration as u64 / 500),
            tokio::process::Command::new(executable)
                .arg(&options)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(CapabilityError::new)?
        .map_err(CapabilityError::new)
    })?;
    if !output.status.success() {
        return Err(CapabilityError::message(format!(
            "local Nemotron runtime failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let raw: Value = serde_json::from_slice(&output.stdout).map_err(CapabilityError::new)?;
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
