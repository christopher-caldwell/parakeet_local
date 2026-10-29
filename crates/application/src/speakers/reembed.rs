//! Recomputing stored voice examples after the embedding model or backend changes.
use super::*;

operation_error!(ReembedError,sources{Persistence:PersistenceError=>"persistence"},values{});

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReembedReport {
    pub replaced: usize,
    pub already_current: usize,
    /// Examples whose source audio is no longer available; they keep their old vectors.
    pub missing_audio: Vec<String>,
    pub failed: Vec<String>,
}
impl SpeakerService {
    /// Re-embeds each example from its stored recording, source and interval with the selected
    /// embedding model. Provenance and verification are preserved; vectors from a different
    /// space are replaced rather than compared.
    pub async fn reembed(&self, profile: Option<&str>) -> Result<ReembedReport, ReembedError> {
        let mut report = ReembedReport::default();
        let model = match self.models.selection().await {
            Ok(selection) => selection.embedding,
            Err(error) => {
                report.failed.push(error.to_string());
                return Ok(report);
            }
        };
        for example in self.queries.examples(profile).await? {
            let Some(detail) = self.library.detail(&example.recording_id).await? else {
                report.missing_audio.push(example.id);
                continue;
            };
            let Some(track) = detail
                .tracks
                .iter()
                .find(|t| t.id == example.track_id && t.available)
            else {
                report.missing_audio.push(example.id);
                continue;
            };
            let output = match self
                .engine
                .embed(&EmbeddingRequest {
                    track: track.clone(),
                    start_ms: example.start_ms,
                    end_ms: example.end_ms,
                    more_ranges: vec![],
                    model: model.clone(),
                })
                .await
            {
                Ok(output) => output,
                Err(error) => {
                    report.failed.push(format!("{}: {error}", example.id));
                    continue;
                }
            };
            if output.model == example.model && output.revision == example.revision {
                report.already_current += 1;
                continue;
            }
            if recognition::cosine(&output.vector, &output.vector).is_none() {
                report
                    .failed
                    .push(format!("{}: invalid embedding", example.id));
                continue;
            }
            let replacement = VoiceExample {
                id: stable_id(
                    "voice-example",
                    &format!(
                        "{}:{}:{}:{}:{}:{}:{}",
                        example.profile_id,
                        example.recording_id,
                        example.track_id,
                        example.start_ms,
                        example.end_ms,
                        output.model,
                        output.revision
                    ),
                ),
                model: output.model,
                revision: output.revision,
                vector: output.vector,
                ..example.clone()
            };
            self.writes.enroll(&replacement).await?;
            self.writes.remove_example(&example.id).await?;
            report.replaced += 1;
        }
        Ok(report)
    }
}
