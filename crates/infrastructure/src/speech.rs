use crate::speech_cache::{Cached, SpeechCache};
use crate::{
    audio,
    models::{ModelFiles, SPEAKRS_ID, SPEAKRS_REV},
};
use async_trait::async_trait;
use parakeet_application::processing_control::*;
use parakeet_application::{processing::*, *};
use parakeet_domain::*;
use serde_json::json;
use std::{sync::Arc, time::Instant};

#[derive(Clone)]
pub struct LocalSpeech {
    models: Arc<ModelFiles>,
    cache: Arc<SpeechCache>,
    model_runtime: std::path::PathBuf,
}
impl LocalSpeech {
    pub fn new(models: Arc<ModelFiles>, model_runtime: std::path::PathBuf) -> Self {
        Self {
            models,
            model_runtime,
            cache: Arc::new(SpeechCache::default()),
        }
    }
}
pub(crate) fn peak_rss_bytes() -> Option<u64> {
    // macOS reports ru_maxrss in bytes; process lifetime peak, not a per-model allocation estimate.
    #[cfg(target_os = "macos")]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        unsafe {
            if libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) == 0 {
                return Some(usage.assume_init().ru_maxrss as u64);
            }
        }
    }
    None
}
#[async_trait]
impl SpeechProcessing for LocalSpeech {
    fn cancellation_mode(&self, model: &str) -> &'static str {
        if model.starts_with("whisper-") {
            "cooperative_callback"
        } else if model.starts_with("nemotron3-") {
            "terminate_owned_child"
        } else {
            "at_engine_boundary"
        }
    }
    async fn process(
        &self,
        request: &RunProcessingInput,
        track: &AudioTrack,
        run: &ProcessingRun,
    ) -> Result<EngineOutput, CapabilityError> {
        self.process_controlled(request, track, run, ProcessingControl::unobserved())
            .await
    }
    async fn process_controlled(
        &self,
        request: &RunProcessingInput,
        track: &AudioTrack,
        run: &ProcessingRun,
        control: ProcessingControl,
    ) -> Result<EngineOutput, CapabilityError> {
        SpeechCache::start_cleanup(&self.cache);
        let cache = self.cache.clone();
        let models = self.models.clone();
        let model_runtime = self.model_runtime.clone();
        let req = request.clone();
        let track = track.clone();
        let run = run.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move ||{
   control.check()?;control.publish(ProcessingStage::LoadingModel,0,None,0,None);
   let _lease=models.acquire()?;audio::validate_source(&track)?;let directory=models.directory(&req.model)?;
   let duration=req.duration_ms.or_else(||track.duration_ms.map(|d|d-req.start_ms)).ok_or_else(||CapabilityError::message("unknown source duration; specify an explicit bounded interval"))?;
   if duration<=0 || duration>7_200_000{return Err(CapabilityError::message("processing is bounded to two hours per run; specify a shorter interval"));}
   let started=Instant::now();let loaded=Instant::now();
   if req.model.starts_with("nemotron3-"){return crate::native_models::nemotron(crate::native_models::NemotronJob{runtime:&runtime,models:&models,request:&req,track:&track,run:&run,duration,started,executable:&model_runtime,control:&control});}
   if req.model.starts_with("whisper-"){
    control.publish(ProcessingStage::LoadingModel,0,Some(duration),0,None);
    crate::whisper::transcribe(crate::whisper::WhisperJob{runtime:&runtime,models:&models,cache:&cache,request:&req,track:&track,run:&run,control:&control,duration,started,directory:&directory})
   }else if req.model==SPEAKRS_ID || req.model=="speakrs-community-1-cpu"{
    control.publish(ProcessingStage::LoadingModel,0,Some(duration),0,None);
    let mut pipeline=speakrs::OwnedDiarizationPipeline::from_dir(&directory,if req.model==SPEAKRS_ID{speakrs::ExecutionMode::CoreMl}else{speakrs::ExecutionMode::Cpu}).map_err(CapabilityError::new)?;let load_seconds=loaded.elapsed().as_secs_f64();let decoding=Instant::now();let samples=runtime.block_on(async {tokio::select!{r=audio::decode(&track,req.start_ms,duration)=>r,_=control.cancelled()=>Err(CapabilityError::message("decoding cancelled"))}})?;let decode_seconds=decoding.elapsed().as_secs_f64();let audio_seconds=samples.len() as f64/16000.;
    control.check()?;control.publish(ProcessingStage::Diarizing,0,Some(duration),0,None);
    let output=pipeline.run(&samples).map_err(CapabilityError::new)?;let mut intervals=vec![];
    for s in &output.segments{if let Some(offset)=track.meeting_offset_ms(){let start_ms=offset+req.start_ms+(s.start*1000.).round() as i64;let end_ms=offset+req.start_ms+(s.end*1000.).round() as i64;if end_ms>start_ms{intervals.push(SpeakerInterval{start_ms,end_ms,speaker:format!("{}:{}",track.id,s.speaker),track_id:Some(track.id.clone())});}}}
    intervals.sort_by(|a,b|a.start_ms.cmp(&b.start_ms).then(a.speaker.cmp(&b.speaker)));let wall=started.elapsed().as_secs_f64();
    Ok(EngineOutput{segments:vec![],intervals,raw:json!({"rttm_track_local":output.rttm(&track.id),"track_start_ms":req.start_ms,"embedding_shape":output.embeddings.0.shape(),"cluster_labels_scoped_to_run":true}),metrics:json!({"wall_seconds":wall,"model_load_seconds":load_seconds,"decode_seconds":decode_seconds,"audio_seconds":audio_seconds,"real_time_factor":wall/audio_seconds,"process_lifetime_peak_rss_bytes":peak_rss_bytes(),"overlap_preserved":true,"timing_space":if track.meeting_offset_ms().is_some(){"meeting"}else{"track_local_only_in_raw_rttm"},"configuration":format!("speakrs 0.7.0 {} 1s step",req.model)})})
   }else{Err(CapabilityError::message("unsupported speech engine"))}
  }).await.map_err(CapabilityError::new)?
    }
}
#[async_trait]
impl VoiceEmbeddings for LocalSpeech {
    async fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingOutput, CapabilityError> {
        let track = &request.track;
        let start_ms = request.start_ms;
        let end_ms = request.end_ms;
        let model_id = request.model.clone();
        if model_id != SPEAKRS_ID && model_id != "speakrs-community-1-cpu" {
            return Err(CapabilityError::message(
                "selected bundle does not provide WeSpeaker embeddings",
            ));
        }
        let verified = self.models.readiness(&model_id).await?;
        if !verified.verified {
            return Err(CapabilityError::message(
                "install and verify the speakrs model before extracting embeddings",
            ));
        }
        SpeechCache::start_cleanup(&self.cache);
        let cache = self.cache.clone();
        let models = self.models.clone();
        let track = track.clone();
        let more_ranges = request.more_ranges.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move ||{let _lease=models.acquire()?;let started=Instant::now();let mut samples=runtime.block_on(audio::decode(&track,start_ms,end_ms-start_ms))?;for (a,b) in &more_ranges{samples.extend(runtime.block_on(audio::decode(&track,*a,*b-*a))?);}
   if samples.len()<48000{return Err(CapabilityError::message("decoded enrollment sample contains less than three seconds"));}
   let rms=(samples.iter().map(|s|(*s as f64).powi(2)).sum::<f64>()/samples.len() as f64).sqrt();if rms<0.0003{return Err(CapabilityError::message("sample has insufficient acoustic energy for enrollment"));}
   let key=models.cache_identity(&model_id)?;let mut cached=cache.embedding.lock().map_err(|_|CapabilityError::message("embedding cache lock failed"))?;
   let reused=cached.as_ref().is_some_and(|c|c.key==key);
   if !reused { *cached=None;let model=speakrs::inference::EmbeddingModel::with_mode(models.directory(&model_id)?.join("wespeaker-voxceleb-resnet34.onnx"),if model_id.ends_with("-cpu"){speakrs::ExecutionMode::Cpu}else{speakrs::ExecutionMode::CoreMl}).map_err(CapabilityError::new)?;*cached=Some(Cached{key,model,last_used:Instant::now()}); }
   let model=&mut cached.as_mut().ok_or_else(||CapabilityError::message("embedding cache missing after initialization"))?.model;
   // The upstream single-window embed method pads to ten seconds but weights
   // the whole padded window. Mask padding explicitly; pool normalized windows
   // so a 20-second enrollment does not silently use only the first ten seconds.
   let mut vectors = Vec::new();
   for chunk in samples.chunks(160_000) {
       if chunk.len() < 48_000 { continue; }
       let mask=valid_embedding_mask(chunk.len());
       let vector=model.embed_masked(chunk,&mask,None).map_err(CapabilityError::new)?.to_vec();
       vectors.push(normalize_vector(vector)?);
   }
   let mut vector=vec![0f32;256];
   for window in &vectors { for (sum,value) in vector.iter_mut().zip(window) { *sum+=value; } }
   let vector=normalize_vector(vector)?;
   if let Some(c)=cached.as_mut(){c.last_used=Instant::now();}
   Ok(EmbeddingOutput{model:"wespeaker-voxceleb-resnet34".into(),revision:format!("{SPEAKRS_REV}:speakrs-0.7.0:masked-window-v1:{}",if model_id.ends_with("-cpu"){"cpu"}else{"coreml"}),vector,metrics:json!({"wall_seconds":started.elapsed().as_secs_f64(),"rms":rms,"requires_clean_nonoverlapping_sample":true,"embedding_windows":vectors.len(),"model_context_reused":reused,"cache_idle_seconds":60,"padding_masked":true})})
  }).await.map_err(CapabilityError::new)?
    }
}

fn valid_embedding_mask(samples: usize) -> Vec<f32> {
    (0..589)
        .map(|i| {
            if (i as f64 + 0.5) / 589.0 < samples.min(160_000) as f64 / 160_000.0 {
                1.0
            } else {
                0.0
            }
        })
        .collect()
}
fn normalize_vector(mut vector: Vec<f32>) -> Result<Vec<f32>, CapabilityError> {
    let norm = vector
        .iter()
        .map(|v| (*v as f64).powi(2))
        .sum::<f64>()
        .sqrt();
    if vector.len() != 256 || !norm.is_finite() || norm <= 0.0 {
        return Err(CapabilityError::message("invalid speaker embedding"));
    }
    for v in &mut vector {
        *v = (*v as f64 / norm) as f32;
    }
    Ok(vector)
}
#[cfg(test)]
mod embedding_tests {
    use super::*;
    #[test]
    fn short_audio_does_not_weight_padded_silence() {
        let short = valid_embedding_mask(48_000);
        assert_eq!(short.len(), 589);
        assert!((short.iter().sum::<f32>() - 176.7).abs() < 1.0);
        assert!(short[177..].iter().all(|v| *v == 0.0));
        assert!(valid_embedding_mask(160_000).iter().all(|v| *v == 1.0));
    }
}
