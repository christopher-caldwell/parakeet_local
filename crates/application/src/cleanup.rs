use super::*;
use crate::storage::*;
use parakeet_domain::storage::{RetentionPolicy, StorageProtection};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_FILES: usize = 128;
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupScope {
    pub root: String,
    pub database: String,
    pub device: u64,
    pub inode: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupFile {
    pub relative_path: String,
    pub run_id: String,
    pub bytes: u64,
    pub device: u64,
    pub inode: u64,
    pub modified_ns: i128,
    pub changed_ns: i128,
    pub sha256: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CleanupState {
    Reviewed,
    Applying,
    Staged,
    Restoring,
    Restored,
    NeedsAttention,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupReview {
    pub id: String,
    pub scope: CleanupScope,
    pub saved_policy: RetentionPolicySnapshot,
    pub effective_policy: RetentionPolicy,
    pub files: Vec<CleanupFile>,
    pub reviewed_at: String,
    pub digest: String,
    pub state: CleanupState,
    pub updated_at: String,
    pub errors: Vec<String>,
    pub reclaimed_bytes: u64,
}
impl CleanupReview {
    pub fn computed_digest(&self) -> Result<String, serde_json::Error> {
        // Only immutable review evidence belongs in the approval digest.
        let data = serde_json::to_vec(&(
            &self.id,
            &self.scope,
            &self.saved_policy,
            &self.effective_policy,
            &self.files,
            &self.reviewed_at,
        ))?;
        Ok(format!("{:x}", Sha256::digest(data)))
    }
}
pub struct CleanupContext {
    pub policy: RetentionPolicySnapshot,
    pub audio: Vec<AudioStorageReference>,
    pub runs: Vec<StorageRunFact>,
}
#[async_trait]
pub trait CleanupUnit: Send {
    async fn commit(self) -> Result<(), PersistenceError>;
}
#[async_trait]
pub trait CleanupTransactions: Send + Sync {
    type Unit: CleanupUnit;
    async fn begin(&self) -> Result<Self::Unit, PersistenceError>;
}
#[async_trait]
pub trait CleanupQueries: Send + Sync {
    async fn cleanup_review(&self, id: &str) -> Result<Option<CleanupReview>, PersistenceError>;
    async fn cleanup_reviews(&self) -> Result<Vec<CleanupReview>, PersistenceError>;
}
#[async_trait]
pub trait CleanupStore<U: CleanupUnit>: CleanupQueries {
    async fn context(&self, unit: &mut U) -> Result<CleanupContext, PersistenceError>;
    async fn load(&self, unit: &mut U, id: &str)
    -> Result<Option<CleanupReview>, PersistenceError>;
    async fn save(&self, unit: &mut U, review: &CleanupReview) -> Result<(), PersistenceError>;
}
// Access holds filesystem/model/run ownership. No database connection is acquired
// inside this capability, including while its caller holds a unit of work.
#[async_trait]
pub trait CleanupAccess: Send + Sync {
    fn scope(&self) -> CleanupScope;
    async fn observe(
        &self,
        audio: &[AudioStorageReference],
    ) -> Result<StorageObservation, CapabilityError>;
    async fn identify(
        &self,
        candidate: &RetentionCandidate,
    ) -> Result<CleanupFile, CapabilityError>;
    async fn transfer(
        &self,
        review: &CleanupReview,
        restore: bool,
    ) -> Result<Vec<String>, CapabilityError>;
}
#[async_trait]
pub trait CleanupFiles: Send + Sync {
    async fn acquire(&self, run_ids: &[String]) -> Result<Box<dyn CleanupAccess>, CapabilityError>;
}
operation_error!(ListCleanupReviewsError,sources{Persistence:PersistenceError=>"persistence"},values{});
operation_error!(ShowCleanupReviewError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});
operation_error!(ReviewCleanupError,sources{Persistence:PersistenceError=>"persistence",Files:CapabilityError=>"storage",Invalid:ValidationError=>"invalid_input"},values{Conflict=>"conflict"});
operation_error!(ApplyCleanupError,sources{Persistence:PersistenceError=>"persistence",Files:CapabilityError=>"storage"},values{NotFound=>"not_found",Conflict=>"conflict",Confirmation=>"confirmation_required"});
operation_error!(RestoreCleanupError,sources{Persistence:PersistenceError=>"persistence",Files:CapabilityError=>"storage"},values{NotFound=>"not_found",Conflict=>"conflict"});
#[derive(Debug, thiserror::Error)]
enum CleanupFailure {
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Files(#[from] CapabilityError),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
}
impl From<CleanupFailure> for ApplyCleanupError {
    fn from(e: CleanupFailure) -> Self {
        match e {
            CleanupFailure::Persistence(e) => Self::Persistence(e),
            CleanupFailure::Files(e) => Self::Files(e),
            CleanupFailure::NotFound(e) => Self::NotFound(e),
            CleanupFailure::Conflict(e) => Self::Conflict(e),
        }
    }
}
impl From<CleanupFailure> for RestoreCleanupError {
    fn from(e: CleanupFailure) -> Self {
        match e {
            CleanupFailure::Persistence(e) => Self::Persistence(e),
            CleanupFailure::Files(e) => Self::Files(e),
            CleanupFailure::NotFound(e) => Self::NotFound(e),
            CleanupFailure::Conflict(e) => Self::Conflict(e),
        }
    }
}
#[derive(Debug, Clone)]
pub struct ReviewCleanupInput {
    pub policy: Option<RetentionPolicy>,
    pub limit: usize,
}
#[derive(Debug, Clone)]
pub struct ApplyCleanupInput {
    pub id: String,
    pub review_digest: String,
    pub confirmed: bool,
}

pub struct CleanupService<T: CleanupTransactions, S: CleanupStore<T::Unit>> {
    transactions: Arc<T>,
    store: Arc<S>,
    files: Arc<dyn CleanupFiles>,
}
impl<T: CleanupTransactions + 'static, S: CleanupStore<T::Unit> + 'static> CleanupService<T, S> {
    pub fn new(transactions: Arc<T>, store: Arc<S>, files: Arc<dyn CleanupFiles>) -> Self {
        Self {
            transactions,
            store,
            files,
        }
    }
    pub async fn list(&self) -> Result<Vec<CleanupReview>, ListCleanupReviewsError> {
        Ok(self.store.cleanup_reviews().await?)
    }
    pub async fn show(&self, id: &str) -> Result<CleanupReview, ShowCleanupReviewError> {
        self.store
            .cleanup_review(id)
            .await?
            .ok_or_else(|| ShowCleanupReviewError::NotFound("cleanup review not found".into()))
    }
    pub async fn review(
        &self,
        input: ReviewCleanupInput,
    ) -> Result<Option<CleanupReview>, ReviewCleanupError> {
        let ReviewCleanupInput { policy, limit } = input;
        if !(1..=MAX_FILES).contains(&limit) {
            return Err(ValidationError("review limit must be 1..128".into()).into());
        }
        let access = self.files.acquire(&[]).await?;
        let mut unit = self.transactions.begin().await?;
        let context = self.store.context(&mut unit).await?;
        let effective = policy.unwrap_or_else(|| context.policy.policy.clone());
        effective.validate()?;
        let observation = access.observe(&context.audio).await?;
        let runs: BTreeMap<_, _> = context.runs.iter().map(|r| (r.id.as_str(), r)).collect();
        let blocked = unresolved_recordings(&context, &observation);
        let mut files = Vec::new();
        let mut bytes = 0u64;
        for file in &observation.files {
            if files.len() == limit {
                break;
            }
            if storage::protection(file, &runs, &effective, Utc::now().timestamp_millis())
                != StorageProtection::EligibleCompletedScratch
            {
                continue;
            }
            let run = runs[file.run_id.as_deref().expect("eligible run")];
            if blocked.contains(&run.recording_id)
                || bytes.saturating_add(file.logical_bytes) > MAX_BYTES
            {
                continue;
            }
            let identified = access
                .identify(&RetentionCandidate {
                    relative_path: file.relative_path.clone(),
                    run_id: run.id.clone(),
                    logical_bytes: file.logical_bytes,
                    allocated_bytes: file.allocated_bytes,
                })
                .await?;
            bytes = bytes.saturating_add(identified.bytes);
            if bytes > MAX_BYTES {
                return Err(ReviewCleanupError::Conflict(
                    "files grew during review; retry".into(),
                ));
            }
            files.push(identified);
        }
        if files.is_empty() {
            return Ok(None);
        }
        let at = now();
        let mut review = CleanupReview {
            id: new_id(),
            scope: access.scope(),
            saved_policy: context.policy,
            effective_policy: effective,
            files,
            reviewed_at: at.clone(),
            updated_at: at,
            digest: String::new(),
            state: CleanupState::Reviewed,
            errors: vec![],
            reclaimed_bytes: 0,
        };
        review.digest = review.computed_digest().map_err(PersistenceError::new)?;
        self.store.save(&mut unit, &review).await?;
        unit.commit().await?;
        Ok(Some(review))
    }
    pub async fn apply(
        &self,
        input: ApplyCleanupInput,
    ) -> Result<CleanupReview, ApplyCleanupError> {
        if !input.confirmed {
            return Err(ApplyCleanupError::Confirmation("review the exact file list, then supply --confirm and its --review-digest; staging does not reclaim disk space".into()));
        }
        Ok(self.execute(&input.id, Some(&input.review_digest)).await?)
    }
    pub async fn restore(&self, id: &str) -> Result<CleanupReview, RestoreCleanupError> {
        Ok(self.execute(id, None).await?)
    }
    async fn execute(
        &self,
        id: &str,
        digest: Option<&str>,
    ) -> Result<CleanupReview, CleanupFailure> {
        // The driver owns its unit and leases through filesystem worker exit and
        // settlement, even when the awaiting UI/CLI task is dropped.
        let service = Self {
            transactions: self.transactions.clone(),
            store: self.store.clone(),
            files: self.files.clone(),
        };
        let id = id.to_owned();
        let digest = digest.map(str::to_owned);
        tokio::spawn(async move { service.execute_owned(&id, digest.as_deref()).await })
            .await
            .map_err(CapabilityError::new)?
    }
    async fn execute_owned(
        &self,
        id: &str,
        digest: Option<&str>,
    ) -> Result<CleanupReview, CleanupFailure> {
        let initial = self
            .store
            .cleanup_review(id)
            .await?
            .ok_or_else(|| CleanupFailure::NotFound("cleanup review not found".into()))?;
        let run_ids: Vec<_> = initial
            .files
            .iter()
            .map(|f| f.run_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let access = self.files.acquire(&run_ids).await?;
        let mut unit = self.transactions.begin().await?;
        let mut review = self
            .store
            .load(&mut unit, id)
            .await?
            .ok_or_else(|| CleanupFailure::NotFound("cleanup review not found".into()))?;
        if review.scope != access.scope()
            || review.digest != review.computed_digest().map_err(PersistenceError::new)?
        {
            return Err(CleanupFailure::Conflict(
                "review scope or evidence changed".into(),
            ));
        }
        let restore = digest.is_none();
        if let Some(digest) = digest {
            if digest != review.digest {
                return Err(CleanupFailure::Conflict(
                    "review digest does not match".into(),
                ));
            }
            if review.state == CleanupState::Staged {
                return Ok(review);
            }
            if review.state != CleanupState::Reviewed {
                return Err(CleanupFailure::Conflict(
                    "review cannot be applied again; inspect and restore retained files".into(),
                ));
            }
            let context = self.store.context(&mut unit).await?;
            validate(&review, &context, &access.observe(&context.audio).await?)?;
        } else {
            if review.state == CleanupState::Restored {
                return Ok(review);
            }
            if review.state == CleanupState::Reviewed {
                return Err(CleanupFailure::Conflict(
                    "this review has not moved any files".into(),
                ));
            }
        }
        review.state = if restore {
            CleanupState::Restoring
        } else {
            CleanupState::Applying
        };
        review.updated_at = now();
        review.errors.clear();
        self.store.save(&mut unit, &review).await?;
        // Durable intent precedes every move; interruption is recovered by checking
        // both locations against the immutable identities in this review.
        unit.commit().await?;
        let mut unit = self.transactions.begin().await?;
        if !restore {
            let context = self.store.context(&mut unit).await?;
            if let Err(error) = validate(&review, &context, &access.observe(&context.audio).await?)
            {
                review.errors.push(error.to_string());
            }
        }
        if review.errors.is_empty() {
            match access.transfer(&review, restore).await {
                Ok(errors) => review.errors = errors,
                Err(error) => review.errors.push(error.to_string()),
            }
        }
        review.state = if !review.errors.is_empty() {
            CleanupState::NeedsAttention
        } else if restore {
            CleanupState::Restored
        } else {
            CleanupState::Staged
        };
        review.updated_at = now();
        self.store.save(&mut unit, &review).await?;
        unit.commit().await?;
        Ok(review)
    }
}
fn unresolved_recordings(
    context: &CleanupContext,
    observation: &StorageObservation,
) -> BTreeSet<String> {
    let missing: BTreeSet<_> = observation
        .external_audio
        .iter()
        .filter(|a| a.missing_or_unresolved)
        .flat_map(|a| &a.track_ids)
        .collect();
    context
        .audio
        .iter()
        .filter(|a| missing.contains(&a.track_id))
        .map(|a| a.recording_id.clone())
        .collect()
}
fn validate(
    review: &CleanupReview,
    context: &CleanupContext,
    observation: &StorageObservation,
) -> Result<(), CleanupFailure> {
    if review.files.is_empty()
        || review.files.len() > MAX_FILES
        || review
            .files
            .iter()
            .map(|f| f.bytes)
            .try_fold(0u64, u64::checked_add)
            .is_none_or(|n| n > MAX_BYTES)
    {
        return Err(CleanupFailure::Conflict(
            "invalid reviewed file bounds".into(),
        ));
    }
    if context.policy.revision != review.saved_policy.revision
        || context.policy.policy != review.saved_policy.policy
    {
        return Err(CleanupFailure::Conflict(
            "retention policy changed; create a new review".into(),
        ));
    }
    let runs: BTreeMap<_, _> = context.runs.iter().map(|r| (r.id.as_str(), r)).collect();
    let blocked = unresolved_recordings(context, observation);
    for file in &review.files {
        let observed = observation
            .files
            .iter()
            .find(|o| o.relative_path == file.relative_path)
            .ok_or_else(|| {
                CleanupFailure::Conflict(format!(
                    "reviewed file is missing: {}",
                    file.relative_path
                ))
            })?;
        if observed.run_id.as_deref() != Some(&file.run_id)
            || observed.device != file.device
            || observed.inode != file.inode
            || observed.logical_bytes != file.bytes
            || storage::protection(
                observed,
                &runs,
                &review.effective_policy,
                Utc::now().timestamp_millis(),
            ) != StorageProtection::EligibleCompletedScratch
            || runs
                .get(file.run_id.as_str())
                .is_none_or(|r| blocked.contains(&r.recording_id))
        {
            return Err(CleanupFailure::Conflict(format!(
                "file is changed or protected: {}",
                file.relative_path
            )));
        }
    }
    Ok(())
}
