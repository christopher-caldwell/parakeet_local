use super::*;
use crate::processing::RunWrites;
operation_error!(EditClustersError,sources{Persistence:PersistenceError=>"persistence",Invalid:ValidationError=>"invalid_input"},values{NotFound=>"not_found"});
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
pub struct ClusterEditingService {
    queries: Arc<dyn LibraryQueries>,
    writes: Arc<dyn RunWrites>,
}
impl ClusterEditingService {
    pub fn new(queries: Arc<dyn LibraryQueries>, writes: Arc<dyn RunWrites>) -> Self {
        Self { queries, writes }
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
        self.writes.start(&run).await?;
        run.status = "completed".into();
        run.completed_at = Some(now());
        run.result = json!({"intervals":output,"raw":{"original_evidence_run":parent.id},"metrics":{"human_edit":true,"model_inference_performed":false}});
        self.writes.finish(&run, &[]).await?;
        Ok(run)
    }
}
