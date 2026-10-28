//! Persistent user control and coalesced, provider-neutral worker observations.
use super::*;
use crate::jobs::ProcessingQueries;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
pub const CONTROL_PROTOCOL: &str = "processing-control-v1";
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingStage {
    Preparing,
    VerifyingSource,
    RestoringChunks,
    LoadingModel,
    Decoding,
    Transcribing,
    Diarizing,
    Finishing,
    Cancelling,
    Completed,
    Interrupted,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunProgress {
    pub sequence: u64,
    pub stage: ProcessingStage,
    pub audio_completed_ms: i64,
    pub audio_total_ms: Option<i64>,
    pub completed_chunks: usize,
    pub current_chunk_percent: Option<u8>,
    pub observed_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunControlState {
    pub status: String,
    pub cancellation_requested_at: Option<String>,
}
#[async_trait]
pub trait RunControlWrites: Send + Sync {
    async fn save_progress(
        &self,
        id: &str,
        progress: &RunProgress,
    ) -> Result<bool, PersistenceError>;
    async fn request_cancellation(&self, id: &str, at: &str) -> Result<bool, PersistenceError>;
}
struct SharedControl {
    retain_work: bool,
    cancelled: AtomicBool,
    wake: tokio::sync::Notify,
    progress: Mutex<RunProgress>,
}
#[derive(Clone)]
pub struct ProcessingControl(Arc<SharedControl>);
impl Default for ProcessingControl {
    fn default() -> Self {
        Self::new(true)
    }
}
impl ProcessingControl {
    fn new(retain_work: bool) -> Self {
        Self(Arc::new(SharedControl {
            retain_work,
            cancelled: AtomicBool::new(false),
            wake: tokio::sync::Notify::new(),
            progress: Mutex::new(RunProgress {
                sequence: 0,
                stage: ProcessingStage::Preparing,
                audio_completed_ms: 0,
                audio_total_ms: None,
                completed_chunks: 0,
                current_chunk_percent: None,
                observed_at: now(),
            }),
        }))
    }
}
impl ProcessingControl {
    pub fn unobserved() -> Self {
        Self::new(false)
    }
    pub fn retains_work(&self) -> bool {
        self.0.retain_work
    }
    pub fn request_cancellation(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        self.0.wake.notify_one();
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    pub fn check(&self) -> Result<(), CapabilityError> {
        if self.is_cancelled() {
            Err(CapabilityError::message(
                "processing cancellation requested",
            ))
        } else {
            Ok(())
        }
    }
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            self.0.wake.notified().await;
        }
    }
    pub fn publish(
        &self,
        stage: ProcessingStage,
        completed: i64,
        total: Option<i64>,
        chunks: usize,
        percent: Option<u8>,
    ) {
        if completed < 0 || total.is_some_and(|t| t <= 0 || completed > t) || chunks > 120 {
            return;
        }
        if let Ok(mut p) = self.0.progress.lock() {
            p.sequence += 1;
            p.stage = stage;
            p.audio_completed_ms = completed;
            p.audio_total_ms = total;
            p.completed_chunks = chunks;
            p.current_chunk_percent = percent.map(|n| n.min(100));
            p.observed_at = now();
        }
    }
    pub fn snapshot(&self) -> RunProgress {
        let mut p = self
            .0
            .progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if self.is_cancelled()
            && !matches!(
                p.stage,
                ProcessingStage::Interrupted | ProcessingStage::Failed
            )
        {
            p.stage = ProcessingStage::Cancelling;
        }
        p
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ControlProcessingError {
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error("processing run not found: {0}")]
    NotFound(String),
    #[error("processing control conflict: {0}")]
    Conflict(String),
}
impl ControlProcessingError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Persistence(_) => "persistence",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CancellationReceipt {
    pub run_id: String,
    pub requested: bool,
    pub status: String,
}
pub struct ProcessingControlService {
    queries: Arc<dyn ProcessingQueries>,
    writes: Arc<dyn RunControlWrites>,
}
impl ProcessingControlService {
    pub fn new(queries: Arc<dyn ProcessingQueries>, writes: Arc<dyn RunControlWrites>) -> Self {
        Self { queries, writes }
    }
    pub async fn show(&self, id: &str) -> Result<ProcessingRun, ControlProcessingError> {
        self.queries
            .run(id)
            .await?
            .ok_or_else(|| ControlProcessingError::NotFound(id.into()))
    }
    pub async fn cancel(&self, id: &str) -> Result<CancellationReceipt, ControlProcessingError> {
        let run = self.show(id).await?;
        if run.status != "running" {
            return Ok(CancellationReceipt {
                run_id: id.into(),
                requested: false,
                status: run.status,
            });
        }
        if run.configuration["control_protocol"] != CONTROL_PROTOCOL {
            return Err(ControlProcessingError::Conflict("this older run has no cancellation protocol; inspect ownership rather than terminating an arbitrary process".into()));
        }
        let requested = self.writes.request_cancellation(id, &now()).await?;
        let state = self
            .queries
            .control_state(id)
            .await?
            .ok_or_else(|| ControlProcessingError::NotFound(id.into()))?;
        Ok(CancellationReceipt {
            run_id: id.into(),
            requested,
            status: state.status,
        })
    }
}
