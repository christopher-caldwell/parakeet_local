use super::*;
use crate::processing::RunWrites;
operation_error!(EditClustersError,sources{Persistence:PersistenceError=>"persistence",Ownership:CapabilityError=>"capability",Invalid:ValidationError=>"invalid_input"},values{NotFound=>"not_found",Conflict=>"conflict"});
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClusterEdit {
    Split { speaker: String, at_ms: i64 },
    Merge { speakers: Vec<String> },
    Undo,
}
#[derive(Debug, Clone)]
pub struct EditClustersInput {
    pub recording_id: String,
    pub run_id: String,
    pub edit: ClusterEdit,
}
#[derive(Debug, Clone)]
pub struct InspectClustersInput {
    pub recording_id: String,
    pub track_id: String,
    pub run_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterSummary {
    pub id: String,
    pub interval_count: usize,
    pub speech_ms: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub preview: Option<PlayAudioInput>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterReview {
    pub recording_id: String,
    pub track_id: String,
    pub run_id: String,
    pub model: String,
    pub can_undo: bool,
    pub clusters: Vec<ClusterSummary>,
}
operation_error!(InspectClustersError,sources{Persistence:PersistenceError=>"persistence",Invalid:ValidationError=>"invalid_input"},values{NotFound=>"not_found"});
pub struct ClusterEditingService {
    queries: Arc<dyn LibraryQueries>,
    writes: Arc<dyn RunWrites>,
    ownership: Arc<dyn crate::jobs::ProcessingOwnership>,
}
impl ClusterEditingService {
    pub fn new(
        queries: Arc<dyn LibraryQueries>,
        writes: Arc<dyn RunWrites>,
        ownership: Arc<dyn crate::jobs::ProcessingOwnership>,
    ) -> Self {
        Self {
            queries,
            writes,
            ownership,
        }
    }
    pub async fn inspect(
        &self,
        input: InspectClustersInput,
    ) -> Result<ClusterReview, InspectClustersError> {
        let detail = self
            .queries
            .detail(&input.recording_id)
            .await?
            .ok_or_else(|| InspectClustersError::NotFound(input.recording_id.clone()))?;
        let track = detail
            .tracks
            .iter()
            .find(|t| t.id == input.track_id)
            .ok_or_else(|| InspectClustersError::NotFound(input.track_id.clone()))?;
        let run = detail
            .runs
            .iter()
            .rev()
            .find(|r| {
                r.status == "completed"
                    && r.kind == "diarization"
                    && r.configuration["track"]["id"] == input.track_id
                    && input.run_id.as_ref().is_none_or(|id| *id == r.id)
            })
            .ok_or_else(|| {
                InspectClustersError::NotFound(
                    "no completed diarization for this source/run".into(),
                )
            })?;
        let intervals: Vec<SpeakerInterval> =
            serde_json::from_value(run.result["intervals"].clone())
                .map_err(|e| ValidationError(e.to_string()))?;
        for interval in &intervals {
            interval.validate()?;
            if interval.track_id.as_deref() != Some(input.track_id.as_str()) {
                return Err(ValidationError(
                    "diarization contains activity for a different source".into(),
                )
                .into());
            }
        }
        let ids: std::collections::BTreeSet<_> =
            intervals.iter().map(|i| i.speaker.clone()).collect();
        let mut clusters = vec![];
        for id in ids {
            let selected: Vec<_> = intervals.iter().filter(|i| i.speaker == id).collect();
            let preview = selected
                .iter()
                .filter(|i| {
                    !intervals.iter().any(|other| {
                        other.speaker != id
                            && other.start_ms < i.end_ms
                            && other.end_ms > i.start_ms
                    })
                })
                .max_by_key(|i| i.end_ms - i.start_ms)
                .and_then(|i| {
                    let local = i.start_ms.checked_sub(track.meeting_offset_ms()?)?;
                    (local >= 0 && track.available).then(|| PlayAudioInput {
                        recording_id: input.recording_id.clone(),
                        track_id: input.track_id.clone(),
                        start_ms: local,
                        duration_ms: (i.end_ms - i.start_ms).min(15_000),
                    })
                });
            clusters.push(ClusterSummary {
                id,
                interval_count: selected.len(),
                speech_ms: selected
                    .iter()
                    .try_fold(0i64, |total, i| total.checked_add(i.end_ms - i.start_ms))
                    .ok_or_else(|| ValidationError("cluster duration overflows".into()))?,
                start_ms: selected
                    .iter()
                    .map(|i| i.start_ms)
                    .min()
                    .expect("nonempty cluster"),
                end_ms: selected
                    .iter()
                    .map(|i| i.end_ms)
                    .max()
                    .expect("nonempty cluster"),
                preview,
            });
        }
        Ok(ClusterReview {
            recording_id: input.recording_id,
            track_id: input.track_id,
            run_id: run.id.clone(),
            model: run.model.clone(),
            can_undo: run.configuration["human_edit_parent_run"].is_string(),
            clusters,
        })
    }
    pub async fn edit(&self, input: EditClustersInput) -> Result<ProcessingRun, EditClustersError> {
        let d = self
            .queries
            .detail(&input.recording_id)
            .await?
            .ok_or_else(|| EditClustersError::NotFound(input.recording_id.clone()))?;
        let parent = d
            .runs
            .iter()
            .find(|r| r.id == input.run_id && r.kind == "diarization" && r.status == "completed")
            .ok_or_else(|| EditClustersError::NotFound(input.run_id.clone()))?;
        let intervals: Vec<SpeakerInterval> =
            serde_json::from_value(parent.result["intervals"].clone())
                .map_err(|e| ValidationError(e.to_string()))?;
        let id = new_id();
        let cluster = format!("manual:{id}");
        let output = match &input.edit {
            ClusterEdit::Split { speaker, at_ms } => {
                parakeet_domain::cluster_edits::split(&intervals, speaker, *at_ms, &cluster)?
            }
            ClusterEdit::Merge { speakers } => {
                parakeet_domain::cluster_edits::merge(&intervals, speakers, &cluster)?
            }
            ClusterEdit::Undo => {
                let previous = parent.configuration["human_edit_parent_run"]
                    .as_str()
                    .ok_or_else(|| {
                        ValidationError("selected run is not a human cluster edit".into())
                    })?;
                d.runs
                    .iter()
                    .find(|r| r.id == previous)
                    .ok_or_else(|| EditClustersError::NotFound(previous.into()))?
                    .result["intervals"]
                    .as_array()
                    .ok_or_else(|| ValidationError("parent intervals missing".into()))?
                    .iter()
                    .map(|i| {
                        serde_json::from_value(i.clone())
                            .map_err(|e| ValidationError(e.to_string()))
                    })
                    .collect::<Result<Vec<SpeakerInterval>, _>>()?
            }
        };
        let previous_state = if matches!(input.edit, ClusterEdit::Undo) {
            let target = parent.configuration["human_edit_parent_run"]
                .as_str()
                .and_then(|id| d.runs.iter().find(|r| r.id == id));
            target
                .map(|r| r.configuration["human_edit_parent_run"].clone())
                .unwrap_or(serde_json::Value::Null)
        } else {
            json!(parent.id)
        };
        let mut run = ProcessingRun {
            id,
            recording_id: input.recording_id,
            kind: "diarization".into(),
            model: parent.model.clone(),
            model_revision: parent.model_revision.clone(),
            status: "running".into(),
            started_at: now(),
            completed_at: None,
            configuration: json!({"track":parent.configuration["track"],"human_edit_parent_run":previous_state,"human_edit_source_run":parent.id,"human_edit":input.edit,"receipt":receipt()}),
            result: serde_json::Value::Null,
            error: None,
        };
        run.configuration["ownership_protocol"] = json!(crate::jobs::OWNERSHIP_PROTOCOL);
        let _lease = self.ownership.try_acquire(&run.id).await?.ok_or_else(|| {
            EditClustersError::Conflict("new run ownership is already held".into())
        })?;
        self.writes.start(&run).await?;
        run.status = "completed".into();
        run.completed_at = Some(now());
        run.result = json!({"intervals":output,"raw":{"original_evidence_run":parent.id},"metrics":{"human_edit":true,"model_inference_performed":false}});
        self.writes.finish(&run, &[]).await?;
        Ok(run)
    }
}
