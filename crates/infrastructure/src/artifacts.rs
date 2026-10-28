use async_trait::async_trait;
use parakeet_application::*;
use parakeet_domain::*;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone, Default)]
pub struct MacParakeetArtifacts;
fn read_json(path: &Path, warnings: &mut Vec<String>) -> Value {
    if !path.exists() {
        return Value::Null;
    }
    let read = || -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        if std::fs::metadata(path)?.len() > 64 * 1024 * 1024 {
            return Err(std::io::Error::other("JSON exceeds 64 MiB safety bound").into());
        }
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    };
    match read() {
        Ok(v) => v,
        Err(e) => {
            warnings.push(format!("{}: {e}", path.display()));
            Value::Null
        }
    }
}
pub fn track_at(
    path: PathBuf,
    recording_id: &str,
    kind: TrackKind,
    alignment: &Value,
) -> AudioTrack {
    let meta = std::fs::metadata(&path).ok();
    let key = format!("{:?}", kind).to_lowercase();
    AudioTrack {
        id: stable_id("track", &format!("{recording_id}:{key}")),
        recording_id: recording_id.into(),
        kind,
        path: path.to_string_lossy().to_string(),
        start_offset_ms: alignment["startOffsetMs"].as_i64().filter(|v| *v >= 0),
        sample_rate: alignment["sampleRate"]
            .as_f64()
            .filter(|s| s.is_finite() && *s > 0.)
            .map(|s| s as u32),
        duration_ms: alignment["timelineFrameCount"]
            .as_f64()
            .or_else(|| alignment["writtenFrameCount"].as_f64())
            .zip(alignment["sampleRate"].as_f64())
            .filter(|(_, rate)| *rate > 0.)
            .map(|(n, rate)| (n * 1000. / rate).round() as i64),
        available: meta.as_ref().is_some_and(|m| m.is_file() && m.len() > 0),
        size_bytes: meta.as_ref().map(|m| m.len()),
        modified_unix_ms: meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64),
        provenance: json!({"ownership":"external_read_only","alignment":alignment}),
    }
}
fn parse_folder(folder: &Path) -> Result<ImportBundle, CapabilityError> {
    let mut warnings = vec![];
    let manifest = read_json(&folder.join("manifest.json"), &mut warnings);
    let transcript = read_json(&folder.join("transcript.json"), &mut warnings);
    let metadata = read_json(
        &folder.join("meeting-recording-metadata.json"),
        &mut warnings,
    );
    if manifest.is_null() && transcript.is_null() {
        return Err(CapabilityError::message(format!(
            "{} has neither readable manifest nor transcript",
            folder.display()
        )));
    }
    if manifest["schemaVersion"].as_i64().is_some_and(|v| v != 1) {
        return Err(CapabilityError::message(format!(
            "{}: unsupported artifact schema version",
            folder.display()
        )));
    }
    let meeting = &manifest["meeting"];
    let source_id = meeting["id"]
        .as_str()
        .or_else(|| transcript["id"].as_str())
        .or_else(|| folder.file_name().and_then(|s| s.to_str()))
        .ok_or_else(|| CapabilityError::message("missing source identity"))?;
    let native = manifest["schema"].as_str() == Some("com.parakeet.local.capture");
    let source_key = format!(
        "{}:{}",
        if native {
            "parakeet:capture"
        } else {
            "macparakeet:meeting"
        },
        source_id.to_lowercase()
    );
    let id = stable_id("recording", &source_key);
    let title = meeting["title"]
        .as_str()
        .or_else(|| transcript["title"].as_str())
        .unwrap_or(source_id)
        .to_owned();
    let recording = Recording {
        id: id.clone(),
        source_key,
        source_kind: "meeting".into(),
        source_title: title,
        title_override: None,
        created_at: meeting["createdAt"]
            .as_str()
            .or_else(|| transcript["createdAt"].as_str())
            .unwrap_or("unknown")
            .into(),
        duration_ms: meeting["durationMs"]
            .as_i64()
            .or_else(|| transcript["durationMs"].as_i64())
            .filter(|x| *x >= 0),
        metadata: json!({"source_id":source_id,"source_folder":folder,"manifest":manifest,"capture":metadata,"imported_claims_verified":false}),
    };
    let mut tracks = vec![];
    for (kind, key, current, legacy) in [
        (
            TrackKind::Microphone,
            "microphone",
            "microphone-raw.m4a",
            "microphone.m4a",
        ),
        (TrackKind::System, "system", "system-raw.m4a", "system.m4a"),
    ] {
        if native
            && ((key == "microphone" && metadata["requestedMicrophoneUID"].is_null())
                || (key == "system" && metadata["requestedSystemAudio"] == false))
        {
            continue;
        }
        if native {
            let report = &metadata["sourceAlignment"][key];
            if report["errors"].as_array().is_some_and(|e| !e.is_empty())
                || report["droppedBuffers"].as_u64().unwrap_or(0) > 0
                || report["gaps"].as_array().is_some_and(|g| !g.is_empty())
            {
                warnings.push(format!(
                    "{source_id}: {key} capture gaps, drops or errors; inspect source report"
                ));
            }
        }
        let caf = current.replace(".m4a", ".caf");
        let p = if folder.join(&caf).exists() {
            folder.join(&caf)
        } else if folder.join(current).exists() {
            folder.join(current)
        } else if folder.join(legacy).exists() {
            warnings.push(format!(
                "{}: legacy audio filename {legacy}",
                folder.display()
            ));
            folder.join(legacy)
        } else {
            folder.join(current)
        };
        let mut track = track_at(p, &id, kind, &metadata["sourceAlignment"][key]);
        track.provenance["unmapped_capture_gaps"] = json!(
            metadata["sourceAlignment"][key]["gaps"].as_str().is_some()
                || (metadata["captureReport"]["interruptedSources"]
                    .as_array()
                    .is_some_and(|list| list.iter().any(|s| s.as_str() == Some(key)))
                    && metadata["sourceAlignment"][key]["timelineFrameCount"].is_null())
        );
        if native {
            track.provenance["ownership"] = json!("application_owned");
            let report = &metadata["sourceAlignment"][key];
            if report["writtenFrameCount"].as_i64().unwrap_or(0) > 0
                && report["peakAmplitude"]
                    .as_f64()
                    .is_some_and(|peak| peak < SILENT_SOURCE_PEAK)
            {
                track.provenance["silent_source"] = json!(true);
                warnings.push(format!(
                    "{source_id}: {key} source delivered audio but never reached a speech level; check the input device"
                ));
            }
        }
        if !track.available {
            warnings.push(format!("{source_id}: missing {key} source audio"));
        }
        if track.start_offset_ms.is_none() {
            warnings.push(format!("{source_id}: {key} alignment unknown"));
        }
        tracks.push(track);
    }
    if folder.join("meeting-playback.m4a").exists() {
        tracks.push(track_at(
            folder.join("meeting-playback.m4a"),
            &id,
            TrackKind::Playback,
            &json!({"startOffsetMs":0}),
        ));
    }
    let mut segments = vec![];
    let run_id = format!("import:{id}");
    if let Some(values) = transcript["transcriptSegments"].as_array() {
        for (ordinal, s) in values.iter().enumerate() {
            let Some(text) = s["text"].as_str() else {
                warnings.push(format!("{source_id}: segment {ordinal} missing text"));
                continue;
            };
            let mut start = s["startMs"].as_i64();
            let mut end = s["endMs"].as_i64();
            if !matches!((start,end),(Some(a),Some(b)) if a>=0 && b>=a) {
                if start.is_some() || end.is_some() {
                    warnings.push(format!(
                        "{source_id}: segment {ordinal} has invalid timing; retained untimed"
                    ));
                }
                start = None;
                end = None;
            }
            let external = s["id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| ordinal.to_string());
            // MacParakeet speaker ids are source-prefixed ("microphone", "system", "system:N").
            // The mapped track is imported, unverified evidence of the source, not an identity.
            let imported_kind = match s["speakerId"].as_str() {
                Some("microphone") => Some(TrackKind::Microphone),
                Some(id) if id == "system" || id.starts_with("system:") => Some(TrackKind::System),
                _ => None,
            };
            let imported_track = imported_kind.as_ref().and_then(|kind| {
                tracks
                    .iter()
                    .find(|t| &t.kind == kind)
                    .map(|t| t.id.clone())
            });
            let source_attribution = match &imported_kind {
                Some(TrackKind::Microphone) => "imported_microphone",
                Some(_) => "imported_system",
                None => "unknown",
            };
            segments.push(TranscriptSegment{id:stable_id("segment",&format!("{id}:{external}:{ordinal}")),recording_id:id.clone(),run_id:run_id.clone(),track_id:imported_track,ordinal,start_ms:start,end_ms:end,text:text.into(),speaker_claim:s["speakerLabel"].as_str().or_else(||s["speakerId"].as_str()).map(str::to_owned),evidence:json!({"origin":"imported_unverified","source_segment":s,"source_attribution":source_attribution})});
        }
    }
    if segments.is_empty()
        && let Some(text) = transcript["transcript"]
            .as_str()
            .or_else(|| transcript["rawTranscript"].as_str())
            .filter(|s| !s.is_empty())
    {
        segments.push(TranscriptSegment {
            id: stable_id("segment", &format!("{id}:untimed")),
            recording_id: id.clone(),
            run_id,
            track_id: None,
            ordinal: 0,
            start_ms: None,
            end_ms: None,
            text: text.into(),
            speaker_claim: None,
            evidence: json!({"origin":"imported_unverified","timing":"unavailable"}),
        });
    }
    order_segments(&mut segments);
    Ok(ImportBundle {
        recording,
        tracks,
        segments,
        original: json!({"manifest":manifest,"transcript":transcript,"metadata":metadata}),
        warnings,
    })
}
#[async_trait]
impl RecordingSource for MacParakeetArtifacts {
    async fn discover(&self, root: &Path) -> Result<Discovery, CapabilityError> {
        let root = std::fs::canonicalize(root).map_err(CapabilityError::new)?;
        let mut result = tokio::task::spawn_blocking(move || {
            let mut folders = vec![];
            if root.join("manifest.json").exists() || root.join("transcript.json").exists() {
                folders.push(root.clone());
            } else {
                for entry in std::fs::read_dir(&root).map_err(CapabilityError::new)? {
                    let entry = entry.map_err(CapabilityError::new)?;
                    if entry.file_type().map_err(CapabilityError::new)?.is_dir() {
                        folders.push(entry.path());
                    }
                }
            }
            folders.sort();
            let mut d = Discovery {
                root: root.display().to_string(),
                recordings: vec![],
                warnings: vec![],
            };
            for folder in folders {
                match parse_folder(&folder) {
                    Ok(b) => d.recordings.push(b),
                    Err(e) => d.warnings.push(e.to_string()),
                }
            }
            Ok::<_, CapabilityError>(d)
        })
        .await
        .map_err(CapabilityError::new)??;
        for bundle in &mut result.recordings {
            for track in &mut bundle.tracks {
                if !track.available {
                    continue;
                }
                match crate::audio::probe(Path::new(&track.path)).await {
                    Ok(p) => {
                        if let (Some(expected), Some(actual)) =
                            (track.duration_ms, p["duration_ms"].as_i64())
                            && (expected - actual).abs() > 2000
                        {
                            bundle.warnings.push(format!(
                                "{}: {:?} metadata duration {}ms differs from decoded media {}ms",
                                bundle.recording.id, track.kind, expected, actual
                            ));
                        }
                        track.duration_ms = p["duration_ms"].as_i64().or(track.duration_ms);
                        track.sample_rate = p["sample_rate"]
                            .as_u64()
                            .map(|s| s as u32)
                            .or(track.sample_rate);
                        track.provenance["probe"] = p;
                        if let Err(error) = fingerprint_import(track).await {
                            track.available = false;
                            bundle.warnings.push(format!("{}: {error}", track.path));
                        }
                    }
                    Err(e) => {
                        track.available = false;
                        bundle.warnings.push(format!("{}: {e}", track.path));
                    }
                }
            }
        }
        Ok(result)
    }
    async fn import_file(&self, path: &Path) -> Result<ImportBundle, CapabilityError> {
        let path = std::fs::canonicalize(path).map_err(CapabilityError::new)?;
        let probe = crate::audio::probe(&path).await?;
        let source_key = format!("file:{}", path.display());
        let id = stable_id("recording", &source_key);
        let mut track = track_at(
            path.clone(),
            &id,
            TrackKind::File,
            &json!({"startOffsetMs":0}),
        );
        track.duration_ms = probe["duration_ms"].as_i64();
        track.sample_rate = probe["sample_rate"].as_u64().map(|s| s as u32);
        track.provenance["probe"] = probe.clone();
        fingerprint_import(&mut track).await?;
        Ok(ImportBundle {
            recording: Recording {
                id,
                source_key,
                source_kind: "file".into(),
                source_title: path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                title_override: None,
                created_at: now(),
                duration_ms: track.duration_ms,
                metadata: json!({"path":path,"probe":probe}),
            },
            tracks: vec![track],
            segments: vec![],
            original: json!({"path":path,"probe":probe}),
            warnings: vec![],
        })
    }
    async fn inspect_tracks(&self, tracks: &[AudioTrack]) -> Result<Vec<Value>, CapabilityError> {
        let mut out = vec![];
        for t in tracks {
            let fingerprint = crate::audio::validate_source(t);
            let probe = if fingerprint.is_ok() {
                crate::audio::probe(Path::new(&t.path)).await.ok()
            } else {
                None
            };
            out.push(json!({"track":t,"current_available":fingerprint.is_ok(),"integrity_error":fingerprint.err().map(|e|e.to_string()),"probe":probe}));
        }
        Ok(out)
    }
}

#[async_trait]
impl LegacyNormalization for MacParakeetArtifacts {
    async fn normalize(
        &self,
        source: &LegacySourceRecord,
    ) -> Result<ImportBundle, CapabilityError> {
        let key = format!(
            "macparakeet:{}:{}",
            source.source_type,
            source.source_id.to_lowercase()
        );
        let id = stable_id("recording", &key);
        let mut warnings = vec![];
        let mut tracks = vec![];
        if let Some(path) = &source.file_path {
            let kind = if source.source_type == "meeting" {
                TrackKind::Playback
            } else {
                TrackKind::File
            };
            let mut track = track_at(PathBuf::from(path), &id, kind, &json!({"startOffsetMs":0}));
            if track.available {
                match crate::audio::probe(Path::new(path)).await {
                    Ok(probe) => {
                        track.duration_ms = probe["duration_ms"].as_i64();
                        track.sample_rate = probe["sample_rate"].as_u64().map(|r| r as u32);
                        track.provenance["probe"] = probe;
                        if let Err(error) = fingerprint_import(&mut track).await {
                            track.available = false;
                            warnings.push(error.to_string());
                        }
                    }
                    Err(e) => {
                        track.available = false;
                        warnings.push(e.to_string());
                    }
                }
            } else {
                warnings.push(format!("{}: source audio missing", source.source_id));
            }
            tracks.push(track);
        } else {
            warnings.push(format!(
                "{}: source has no retained audio path",
                source.source_id
            ));
        }
        let parse = |key: &str| {
            source.original[key]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null)
        };
        let mut segments = vec![];
        let raw_segments = parse("transcriptSegments");
        let run_id = format!("import:{id}");
        if let Some(list) = raw_segments.as_array() {
            for (ordinal, s) in list.iter().enumerate() {
                let Some(text) = s["text"].as_str() else {
                    continue;
                };
                let start = s["startMs"].as_i64();
                let end = s["endMs"].as_i64();
                let timed = matches!((start,end),(Some(a),Some(b))if a>=0&&b>=a);
                segments.push(TranscriptSegment {
                    id: stable_id(
                        "segment",
                        &format!(
                            "{id}:{}:{ordinal}",
                            s["id"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| ordinal.to_string())
                        ),
                    ),
                    recording_id: id.clone(),
                    run_id: run_id.clone(),
                    track_id: None,
                    ordinal,
                    start_ms: if timed { start } else { None },
                    end_ms: if timed { end } else { None },
                    text: text.into(),
                    speaker_claim: s["speakerLabel"]
                        .as_str()
                        .or_else(|| s["speakerId"].as_str())
                        .map(str::to_owned),
                    evidence: json!({"origin":"imported_database_unverified","original":s}),
                });
            }
        }
        if segments.is_empty()
            && let Some(text) = source.original["cleanTranscript"]
                .as_str()
                .or_else(|| source.original["rawTranscript"].as_str())
                .filter(|s| !s.is_empty())
        {
            segments.push(TranscriptSegment {
                id: stable_id("segment", &format!("{id}:untimed")),
                recording_id: id.clone(),
                run_id,
                track_id: None,
                ordinal: 0,
                start_ms: None,
                end_ms: None,
                text: text.into(),
                speaker_claim: None,
                evidence: json!({"origin":"imported_database_unverified"}),
            });
        }
        order_segments(&mut segments);
        Ok(ImportBundle {
            recording: Recording {
                id,
                source_key: key,
                source_kind: source.source_type.clone(),
                source_title: source.title.clone(),
                title_override: None,
                created_at: source.created_at.clone(),
                duration_ms: source.duration_ms,
                metadata: json!({"source_id":source.source_id,"source_url":source.original["sourceURL"],"channel":source.original["channelName"],"origin":"macparakeet_database_read_only"}),
            },
            tracks,
            segments,
            original: source.original.clone(),
            warnings,
        })
    }
}
async fn fingerprint_import(track: &mut AudioTrack) -> Result<(), CapabilityError> {
    let identity = parakeet_application::audio_locations::AudioFileIdentityReader::identify(
        &crate::audio::LocalAudioIdentity,
        Path::new(&track.path),
    )
    .await?;
    crate::audio::validate_source(track)?;
    if track.size_bytes != Some(identity.size_bytes)
        || track.modified_unix_ms != Some(identity.modified_unix_ms)
    {
        return Err(CapabilityError::message(
            "source changed during import fingerprinting",
        ));
    }
    track.provenance["content_sha256"] = json!(identity.sha256);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn legacy_offset_untimed_and_missing_audio() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a");
        std::fs::create_dir(&f).unwrap();
        std::fs::write(f.join("manifest.json"),r#"{"schemaVersion":1,"meeting":{"id":"source-id","title":"Original","createdAt":"2026-01-01"}}"#).unwrap();
        std::fs::write(f.join("meeting-recording-metadata.json"),r#"{"sourceAlignment":{"system":{"startOffsetMs":2400,"sampleRate":48000,"writtenFrameCount":480000}}}"#).unwrap();
        std::fs::write(f.join("transcript.json"),r#"{"transcriptSegments":[{"text":"late","startMs":5000,"endMs":6000,"speakerLabel":"Me"},{"text":"early","startMs":2000,"endMs":3000},{"text":"untimed"}]}"#).unwrap();
        let d = MacParakeetArtifacts.discover(tmp.path()).await.unwrap();
        let b = &d.recordings[0];
        assert_eq!(b.segments[0].text, "early");
        assert_eq!(b.segments[2].start_ms, None);
        assert_eq!(b.tracks[1].start_offset_ms, Some(2400));
        assert_eq!(d.audit().missing_source_audio, 1);
        assert_eq!(b.segments[1].speaker_claim.as_deref(), Some("Me"));
        assert_eq!(b.segments[1].track_id, None);
    }
    #[test]
    fn imported_source_prefixed_speaker_ids_map_to_unverified_tracks() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("manifest.json"),
            r#"{"schemaVersion":1,"meeting":{"id":"s","title":"T","createdAt":"2026-01-01"}}"#,
        )
        .unwrap();
        std::fs::write(tmp.path().join("transcript.json"),r#"{"transcriptSegments":[{"text":"a","startMs":0,"endMs":1,"speakerId":"microphone","speakerLabel":"Alex"},{"text":"b","startMs":2,"endMs":3,"speakerId":"system:2","speakerLabel":"Others 2"},{"text":"c","startMs":4,"endMs":5,"speakerLabel":"x"}]}"#).unwrap();
        std::fs::write(tmp.path().join("meeting-recording-metadata.json"), "{}").unwrap();
        let b = parse_folder(tmp.path()).unwrap();
        let mic = b
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Microphone)
            .unwrap();
        let sys = b
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::System)
            .unwrap();
        assert_eq!(b.segments[0].track_id.as_deref(), Some(mic.id.as_str()));
        assert_eq!(b.segments[1].track_id.as_deref(), Some(sys.id.as_str()));
        assert_eq!(b.segments[2].track_id, None);
        assert_eq!(
            b.segments[0].evidence["source_attribution"],
            "imported_microphone"
        );
        assert_eq!(b.segments[0].evidence["origin"], "imported_unverified");
    }
    #[test]
    fn captured_source_that_never_reaches_speech_level_is_marked_silent() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("manifest.json"),r#"{"schema":"com.parakeet.local.capture","schemaVersion":1,"meeting":{"id":"s","title":"T","createdAt":"2026-01-01"}}"#).unwrap();
        std::fs::write(tmp.path().join("meeting-recording-metadata.json"),r#"{"requestedMicrophoneUID":"mic","requestedSystemAudio":true,"sourceAlignment":{"microphone":{"writtenFrameCount":480000,"peakAmplitude":0.0,"startOffsetMs":0},"system":{"writtenFrameCount":480000,"peakAmplitude":0.4,"startOffsetMs":0}}}"#).unwrap();
        std::fs::write(tmp.path().join("transcript.json"), "{}").unwrap();
        let b = parse_folder(tmp.path()).unwrap();
        let mic = b
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Microphone)
            .unwrap();
        let sys = b
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::System)
            .unwrap();
        assert_eq!(mic.provenance["silent_source"], true);
        assert!(sys.provenance["silent_source"].is_null());
        assert!(
            b.warnings
                .iter()
                .any(|w| w.contains("microphone source delivered audio"))
        );
    }
    #[test]
    fn malformed_manifest_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("manifest.json"), "{").unwrap();
        assert!(parse_folder(tmp.path()).is_err());
    }
}
