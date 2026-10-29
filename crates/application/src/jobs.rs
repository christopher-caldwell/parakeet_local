use super::*;

pub const OWNERSHIP_PROTOCOL: &str = "file-lock-v1";

/// Dropping this guard ends ownership. It must be acquired before publishing a
/// running row, and held through terminal persistence, including errors.
pub trait ProcessingLease: Send + Sync {}
#[async_trait]
pub trait ProcessingOwnership: Send + Sync {
    async fn try_acquire(
        &self,
        run_id: &str,
    ) -> Result<Option<Box<dyn ProcessingLease>>, CapabilityError>;
}
#[async_trait]
pub trait ProcessingQueries: Send + Sync {
    async fn pending_runs(&self) -> Result<Vec<ProcessingRun>, PersistenceError>;
    async fn run(&self, id: &str) -> Result<Option<ProcessingRun>, PersistenceError>;
    async fn control_state(
        &self,
        id: &str,
    ) -> Result<Option<crate::processing_control::RunControlState>, PersistenceError> {
        Ok(self
            .run(id)
            .await?
            .map(|r| crate::processing_control::RunControlState {
                status: r.status,
                cancellation_requested_at: r.configuration["cancellation_requested_at"]
                    .as_str()
                    .map(str::to_owned),
            }))
    }
}
#[async_trait]
pub trait RecoveryWrites: Send + Sync {
    /// Compare-and-set a still-running, ownership-aware row. A false result means
    /// another operation settled it; no segments or source evidence are changed.
    async fn interrupt(&self, run: &ProcessingRun) -> Result<bool, PersistenceError>;
}

operation_error!(ListProcessingJobsError,sources{Persistence:PersistenceError=>"persistence"},values{});
operation_error!(RecoverProcessingJobsError,sources{Persistence:PersistenceError=>"persistence",Ownership:CapabilityError=>"capability"},values{});
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProcessingRecoveryReport {
    pub interrupted: Vec<String>,
    pub active: Vec<String>,
    pub unmanaged: Vec<String>,
    pub already_settled: Vec<String>,
}
pub struct ProcessingRecoveryService {
    queries: Arc<dyn ProcessingQueries>,
    writes: Arc<dyn RecoveryWrites>,
    ownership: Arc<dyn ProcessingOwnership>,
}
impl ProcessingRecoveryService {
    pub fn new(
        queries: Arc<dyn ProcessingQueries>,
        writes: Arc<dyn RecoveryWrites>,
        ownership: Arc<dyn ProcessingOwnership>,
    ) -> Self {
        Self {
            queries,
            writes,
            ownership,
        }
    }
    /// Read-only: does not create lock directories or mutate pending jobs.
    pub async fn pending(&self) -> Result<Vec<ProcessingRun>, ListProcessingJobsError> {
        Ok(self.queries.pending_runs().await?)
    }
    pub async fn recover(&self) -> Result<ProcessingRecoveryReport, RecoverProcessingJobsError> {
        self.recover_matching(None).await
    }
    /// Resume one recording without changing abandoned jobs from other recordings.
    pub async fn recover_recording(
        &self,
        recording_id: &str,
    ) -> Result<ProcessingRecoveryReport, RecoverProcessingJobsError> {
        self.recover_matching(Some(recording_id)).await
    }
    async fn recover_matching(
        &self,
        recording_id: Option<&str>,
    ) -> Result<ProcessingRecoveryReport, RecoverProcessingJobsError> {
        let mut report = ProcessingRecoveryReport::default();
        for mut run in self.queries.pending_runs().await? {
            if recording_id.is_some_and(|id| id != run.recording_id) {
                continue;
            }
            if run.configuration["ownership_protocol"] != OWNERSHIP_PROTOCOL {
                // Older processes cannot participate in this protocol. Time or
                // PID guesses are insufficient evidence that they stopped.
                report.unmanaged.push(run.id);
                continue;
            }
            let Some(_lease) = self.ownership.try_acquire(&run.id).await? else {
                report.active.push(run.id);
                continue;
            };
            run.status = "interrupted".into();
            run.completed_at = Some(now());
            run.error = Some("Processing owner ended before terminal persistence; original audio retained. Retry creates a new run.".into());
            run.configuration["recovery"] = json!({"reason":"owner_lock_available","recovered_at":run.completed_at,"protocol":OWNERSHIP_PROTOCOL});
            if self.writes.interrupt(&run).await? {
                report.interrupted.push(run.id);
            } else {
                report.already_settled.push(run.id);
            }
        }
        Ok(report)
    }
}
