//! Whisper transcription over speech-aligned chunks.
//!
//! Silence is never decoded, so it cannot become hallucinated text, and chunks are cut inside
//! silence, so words are not split at chunk boundaries. A failed or cancelled run simply
//! restarts: there is no partial-progress scratch state.
use crate::speech_cache::{Cached, SpeechCache};
use crate::{audio, models::ModelFiles, vad};
use parakeet_application::processing::*;
use parakeet_application::processing_control::*;
use parakeet_application::*;
use parakeet_domain::*;
use serde_json::{Value, json};
use std::time::Instant;

pub(crate) struct WhisperJob<'a> {
    pub runtime: &'a tokio::runtime::Handle,
    pub models: &'a ModelFiles,
    pub cache: &'a SpeechCache,
    pub request: &'a RunProcessingInput,
    pub track: &'a AudioTrack,
    pub run: &'a ProcessingRun,
    pub control: &'a ProcessingControl,
    pub duration: i64,
    pub started: Instant,
    pub directory: &'a std::path::Path,
}
/// Decoded in blocks so a long recording never sits in memory whole.
const BLOCK_MS: i64 = 600_000;
/// Conversational speech runs well above this; less in a long chunk means words were skipped.
const SPARSE_WORDS_PER_SECOND: f64 = 1.2;
const SPARSE_MIN_SPEECH_SECONDS: f64 = 6.;

pub(crate) fn transcribe(job: WhisperJob<'_>) -> Result<EngineOutput, CapabilityError> {
    let WhisperJob {
        runtime,
        models,
        cache,
        request: req,
        track,
        run,
        control,
        duration,
        started,
        directory,
    } = job;
    whisper_rs::install_logging_hooks();
    let file = directory.join(format!(
        "ggml-{}.bin",
        req.model.strip_prefix("whisper-").unwrap_or(&req.model)
    ));
    let key = models.cache_identity(&req.model)?;
    let loaded = Instant::now();
    let mut cached = cache
        .whisper
        .lock()
        .map_err(|_| CapabilityError::message("ASR model cache lock failed"))?;
    let reused = cached.as_ref().is_some_and(|c| c.key == key);
    if !reused {
        *cached = None;
        let model = whisper_rs::WhisperContext::new_with_params(
            file.to_str()
                .ok_or_else(|| CapabilityError::message("invalid model path"))?,
            whisper_rs::WhisperContextParameters::default(),
        )
        .map_err(CapabilityError::new)?;
        *cached = Some(Cached {
            key: key.clone(),
            model,
            last_used: Instant::now(),
        });
    }
    let context = &cached
        .as_ref()
        .ok_or_else(|| CapabilityError::message("ASR cache missing after initialization"))?
        .model;
    let load_seconds = if reused {
        0.
    } else {
        loaded.elapsed().as_secs_f64()
    };
    let offset = track.meeting_offset_ms();
    let mut segments: Vec<TranscriptSegment> = vec![];
    let mut raw: Vec<Value> = vec![];
    let (mut decode_seconds, mut audio_seconds, mut speech_seconds) = (0., 0., 0.);
    let mut chunks_done = 0usize;
    let mut position = 0i64;
    while position < duration {
        control.check()?;
        control.publish(
            ProcessingStage::Decoding,
            position,
            Some(duration),
            chunks_done,
            None,
        );
        let length = (duration - position).min(BLOCK_MS);
        let decoding = Instant::now();
        let samples = runtime.block_on(async {
            tokio::select! {
                r = audio::decode(track, req.start_ms + position, length) => r,
                _ = control.cancelled() => Err(CapabilityError::message("decoding cancelled")),
            }
        })?;
        decode_seconds += decoding.elapsed().as_secs_f64();
        if samples.is_empty() {
            break;
        }
        audio_seconds += samples.len() as f64 / vad::SAMPLE_RATE as f64;
        // A short read means the source ended; otherwise leave room for speech that continues.
        let last_block =
            position + length >= duration || (samples.len() as i64) < length * 16 - 1600;
        let (chunks, consumed) = vad::plan_chunks(&samples, last_block);
        for chunk in &chunks {
            control.check()?;
            let chunk_start = position + chunk.start as i64 / 16;
            let pcm = &samples[chunk.start..chunk.end];
            speech_seconds += chunk.speech as f64 / vad::SAMPLE_RATE as f64;
            let mark = (segments.len(), raw.len());
            let decode = |pcm: &[f32], start: i64, segments: &mut Vec<_>, raw: &mut Vec<_>| {
                transcribe_chunk(
                    context,
                    req,
                    track,
                    run,
                    control,
                    pcm,
                    start,
                    duration,
                    chunks_done,
                    offset,
                    segments,
                    raw,
                )
            };
            decode(pcm, chunk_start, &mut segments, &mut raw)?;
            // Whisper sometimes silently skips a sentence inside a long chunk. Speech this sparse
            // is re-decoded in halves, keeping whichever pass recovered more words.
            let speech = chunk.speech as f64 / vad::SAMPLE_RATE as f64;
            let words = |segments: &[TranscriptSegment]| {
                segments
                    .iter()
                    .map(|s| s.text.split_whitespace().count())
                    .sum::<usize>()
            };
            if speech >= SPARSE_MIN_SPEECH_SECONDS
                && (words(&segments[mark.0..]) as f64) < speech * SPARSE_WORDS_PER_SECOND
            {
                let first = (segments.split_off(mark.0), raw.split_off(mark.1));
                let cut = vad::quietest_cut(pcm);
                decode(&pcm[..cut], chunk_start, &mut segments, &mut raw)?;
                decode(
                    &pcm[cut..],
                    chunk_start + cut as i64 / 16,
                    &mut segments,
                    &mut raw,
                )?;
                if words(&segments[mark.0..]) <= words(&first.0) {
                    segments.truncate(mark.0);
                    raw.truncate(mark.1);
                    segments.extend(first.0);
                    raw.extend(first.1);
                }
            }
            chunks_done += 1;
            control.publish(
                ProcessingStage::Transcribing,
                (chunk_start + pcm.len() as i64 / 16).min(duration),
                Some(duration),
                chunks_done,
                None,
            );
        }
        if last_block {
            break;
        }
        position += (consumed as i64 / 16).max(1_000);
    }
    if let Some(c) = cached.as_mut() {
        c.last_used = Instant::now();
    }
    order_segments(&mut segments);
    let wall = started.elapsed().as_secs_f64();
    Ok(EngineOutput {
        segments,
        intervals: vec![],
        raw: json!(raw),
        metrics: json!({
            "wall_seconds": wall,
            "model_load_seconds": load_seconds,
            "decode_seconds": decode_seconds,
            "audio_seconds": audio_seconds,
            "speech_seconds": speech_seconds,
            "skipped_non_speech_seconds": (audio_seconds - speech_seconds).max(0.),
            "real_time_factor": wall / audio_seconds.max(f64::EPSILON),
            "process_lifetime_peak_rss_bytes": super::speech::peak_rss_bytes(),
            "model_context_reused": reused,
            "cache_idle_seconds": 60,
            "runtime_configuration": "whisper-rs-0.16.0:context-cache-v1:no-context-vad-chunks",
            "chunking": "energy-vad-speech-chunks-v1",
            "chunk_overlap_ms": 0,
            "inference_chunks": chunks_done,
            "experimental_word_timestamps": req.word_timestamps,
            "word_timing_algorithm": if req.word_timestamps { Some("whisper-token-estimates-v1:whitespace-byte-grouping:no-interpolation") } else { None },
            "threads": 4,
            "timing_space": if offset.is_some() { "meeting" } else { "untimed_with_track_local_evidence" },
        }),
    })
}
/// Why `text` is Whisper noise rather than speech: a bracketed sound annotation such as
/// `[BLANK_AUDIO]` or `(air whooshing)`, or a caption sign-off it learned from video subtitles.
/// Ordinary phrases like "thank you" are speech and are never matched.
fn non_speech_text(text: &str) -> Option<&'static str> {
    let text = text.trim();
    let wrapped = |open: char, close: char| {
        text.starts_with(open) && text.ends_with(close) && !text[1..].contains(open)
    };
    if wrapped('[', ']')
        || wrapped('(', ')')
        || (text.len() > 1 && text.starts_with('*') && text.ends_with('*'))
    {
        return Some("sound_annotation");
    }
    let plain: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    const SIGN_OFFS: [&str; 5] = [
        "thank you for watching",
        "thanks for watching",
        "please subscribe",
        "thank you for watching and see you next time",
        "subtitles by the amara org community",
    ];
    SIGN_OFFS
        .contains(&plain.as_str())
        .then_some("caption_sign_off")
}
#[allow(clippy::too_many_arguments)]
fn transcribe_chunk(
    context: &whisper_rs::WhisperContext,
    req: &RunProcessingInput,
    track: &AudioTrack,
    run: &ProcessingRun,
    control: &ProcessingControl,
    samples: &[f32],
    chunk_start: i64,
    duration: i64,
    chunks_done: usize,
    offset: Option<i64>,
    segments: &mut Vec<TranscriptSegment>,
    raw: &mut Vec<Value>,
) -> Result<(), CapabilityError> {
    let actual_ms = samples.len() as i64 / 16;
    let rms = (samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>()
        / samples.len().max(1) as f64)
        .sqrt();
    let mut state = context.create_state().map_err(CapabilityError::new)?;
    let mut params = whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::BeamSearch {
        beam_size: 5,
        patience: -1.,
    });
    params.set_language(Some("en"));
    params.set_token_timestamps(req.word_timestamps);
    params.set_n_threads(4);
    params.set_no_context(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_temperature(0.);
    params.set_temperature_inc(0.);
    params.set_suppress_blank(true);
    control.check()?;
    let callbacks = Box::new(crate::whisper_control::Callbacks {
        control: control.clone(),
        consumed: chunk_start,
        duration,
        chunks: chunks_done,
    });
    let inference = callbacks.run(&mut state, params, samples);
    drop(callbacks);
    inference.map_err(CapabilityError::new)?;
    let origin = req.start_ms + chunk_start;
    for s in state.as_iter() {
        let ordinal = segments.len();
        let text = s.to_str_lossy().map_err(CapabilityError::new)?.to_string();
        let local_start = origin + s.start_timestamp() * 10;
        let local_end = (origin + s.end_timestamp() * 10).min(origin + actual_ms);
        let no_speech = s.no_speech_probability();
        let mut evidence = json!({"local_start_ms": local_start, "local_end_ms": local_end, "no_speech_probability": no_speech, "chunk_rms": rms, "uncertain_speech": rms < 0.0003 || no_speech > 0.6, "attribution": "unknown", "timing_quality": "model_segment"});
        if req.word_timestamps {
            let mut tokens = vec![];
            let mut raw_tokens = vec![];
            for index in 0..s.n_tokens() {
                let token = s
                    .get_token(index)
                    .ok_or_else(|| CapabilityError::message("missing ASR token"))?;
                if token.token_id() >= context.token_eot() {
                    continue;
                }
                let data = token.token_data();
                let bytes = token.to_bytes().map_err(CapabilityError::new)?.to_vec();
                raw_tokens.push(json!({"id": data.id, "bytes": bytes, "probability": data.p, "t0_10ms": data.t0, "t1_10ms": data.t1}));
                tokens.push(crate::word_timing::TokenTiming {
                    bytes,
                    start_ms: data.t0.saturating_mul(10),
                    end_ms: data.t1.saturating_mul(10),
                });
            }
            evidence["word_timing_version"] = json!(1);
            evidence["words"] = json!(crate::word_timing::words(
                &text, &tokens, origin, actual_ms, offset
            ));
            evidence["raw_token_timing"] = json!(raw_tokens);
        }
        if let Some(reason) = non_speech_text(&text) {
            // Kept as evidence, never published as something the speaker said.
            evidence["suppressed"] = json!(reason);
            raw.push(json!({"text": "", "suppressed_text": text, "evidence": evidence}));
            continue;
        }
        raw.push(json!({"text": text, "evidence": evidence}));
        // Low-energy recognition stays inspectable; it is never silently turned into a named speaker.
        segments.push(TranscriptSegment {
            id: stable_id("segment", &format!("{}:{ordinal}", run.id)),
            recording_id: run.recording_id.clone(),
            run_id: run.id.clone(),
            track_id: Some(track.id.clone()),
            ordinal,
            start_ms: offset.map(|o| o + local_start),
            end_ms: offset.map(|o| o + local_end.max(local_start)),
            text,
            speaker_claim: None,
            evidence,
        });
    }
    Ok(())
}

#[cfg(test)]
mod non_speech_tests {
    use super::non_speech_text;
    #[test]
    fn annotations_and_sign_offs_are_suppressed() {
        for text in [
            " [BLANK_AUDIO]",
            "(air whooshing)",
            " *sigh*",
            " Thank you for watching.",
            "Thanks for watching!",
        ] {
            assert!(non_speech_text(text).is_some(), "{text}");
        }
    }
    #[test]
    fn real_speech_is_kept() {
        for text in [
            " Thank you.",
            " Thanks, that was helpful.",
            " I said (quietly) hello",
            " Yeah.",
            " Thank you for watching the kids.",
        ] {
            assert!(non_speech_text(text).is_none(), "{text}");
        }
    }
}
