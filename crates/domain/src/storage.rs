//! Conservative retention decisions. Observations are not permission to delete.
use crate::ValidationError;
use serde::{Deserialize, Serialize};
/// Raw capture audio stays untouched this long, then becomes a verified lossless copy.
pub const RAW_AUDIO_RETENTION_DAYS: u64 = 7;
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub completed_scratch_after_days: Option<u16>,
}
impl RetentionPolicy {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self
            .completed_scratch_after_days
            .is_some_and(|d| d == 0 || d > 3650)
        {
            return Err(ValidationError(
                "completed scratch age must be 1–3650 days, or keep all".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageProtection {
    EligibleCompletedScratch,
    KeepAll,
    ActiveRun,
    RecoverableRun,
    UnknownRun,
    PendingMeeting,
    SpeakerReference,
    ManualCorrection,
    CompletionTimeUnknown,
    TooRecentOrFuture,
    RecordedAudio,
    Database,
    Models,
    Recovery,
    Playback,
    OtherFiles,
    UncertainArtifact,
    LinkedAudio,
    MultipleHardLinks,
}
#[derive(Debug, Clone)]
pub struct RunRetentionEvidence {
    pub status: String,
    pub ownership_verified: bool,
    pub completed_unix_ms: Option<i64>,
    pub pending_meeting: bool,
    pub speaker_reference: bool,
    pub manual_correction: bool,
}
impl RunRetentionEvidence {
    pub fn protection(&self, policy: &RetentionPolicy, now_ms: i64) -> StorageProtection {
        use StorageProtection::*;
        match self.status.as_str() {
            "running" => return ActiveRun,
            "failed" | "interrupted" => return RecoverableRun,
            "completed" => {}
            _ => return UnknownRun,
        }
        if !self.ownership_verified {
            return UnknownRun;
        }
        if self.pending_meeting {
            return PendingMeeting;
        }
        if self.speaker_reference {
            return SpeakerReference;
        }
        if self.manual_correction {
            return ManualCorrection;
        }
        let Some(days) = policy.completed_scratch_after_days else {
            return KeepAll;
        };
        // Invalid stored policies must never accidentally admit files.
        if policy.validate().is_err() {
            return KeepAll;
        }
        let Some(finished) = self.completed_unix_ms.filter(|t| *t >= 0) else {
            return CompletionTimeUnknown;
        };
        if now_ms < finished || now_ms.saturating_sub(finished) < i64::from(days) * 86_400_000 {
            return TooRecentOrFuture;
        }
        EligibleCompletedScratch
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn completed() -> RunRetentionEvidence {
        RunRetentionEvidence {
            status: "completed".into(),
            ownership_verified: true,
            completed_unix_ms: Some(1000),
            pending_meeting: false,
            speaker_reference: false,
            manual_correction: false,
        }
    }
    #[test]
    fn retention_defaults_to_keep_all_and_age_requires_valid_completion() {
        use StorageProtection::*;
        let mut r = completed();
        let p = RetentionPolicy {
            completed_scratch_after_days: Some(1),
        };
        assert_eq!(
            r.protection(&RetentionPolicy::default(), 100_000_000),
            KeepAll
        );
        assert_eq!(r.protection(&p, 86_400_999), TooRecentOrFuture);
        assert_eq!(r.protection(&p, 86_401_000), EligibleCompletedScratch);
        assert_eq!(r.protection(&p, 0), TooRecentOrFuture);
        r.completed_unix_ms = None;
        assert_eq!(r.protection(&p, 100_000_000), CompletionTimeUnknown);
        r.completed_unix_ms = Some(-1);
        assert_eq!(r.protection(&p, 100_000_000), CompletionTimeUnknown);
        for days in [0, 3651] {
            let p = RetentionPolicy {
                completed_scratch_after_days: Some(days),
            };
            assert!(p.validate().is_err());
            assert_eq!(completed().protection(&p, 1_000_000_000), KeepAll);
        }
    }
    #[test]
    fn active_recoverable_and_human_evidence_override_age() {
        use StorageProtection::*;
        let p = RetentionPolicy {
            completed_scratch_after_days: Some(1),
        };
        for (state, expected) in [
            ("running", ActiveRun),
            ("interrupted", RecoverableRun),
            ("failed", RecoverableRun),
            ("unknown", UnknownRun),
        ] {
            let mut r = completed();
            r.status = state.into();
            assert_eq!(r.protection(&p, 1_000_000_000), expected);
        }
        let mut r = completed();
        r.pending_meeting = true;
        assert_eq!(r.protection(&p, 1_000_000_000), PendingMeeting);
        r.pending_meeting = false;
        r.speaker_reference = true;
        assert_eq!(r.protection(&p, 1_000_000_000), SpeakerReference);
        r.speaker_reference = false;
        r.manual_correction = true;
        assert_eq!(r.protection(&p, 1_000_000_000), ManualCorrection);
    }
}
