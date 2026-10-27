use crate::Correction;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpeakerOverride {
    Profile(String),
    Unknown,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CorrectionState {
    pub text: Option<String>,
    pub speaker: Option<SpeakerOverride>,
    pub can_undo: bool,
}
/// Corrections are immutable revisions. Text changes inherit speaker state;
/// explicit Unknown survives them, while undo restores the previous revision.
pub fn resolve(segment_id: &str, history: &[Correction]) -> CorrectionState {
    let mut active: Vec<&Correction> = vec![];
    for revision in history.iter().filter(|r| r.segment_id == segment_id) {
        if revision.action == "undo" {
            active.pop();
        } else {
            active.push(revision);
        }
    }
    let speaker = active.iter().rev().find_map(|r| match r.action.as_str() {
        "unknown" | "reject" => Some(SpeakerOverride::Unknown),
        "assign" => Some(
            r.profile_id
                .clone()
                .map(SpeakerOverride::Profile)
                .unwrap_or(SpeakerOverride::Unknown),
        ),
        _ => None,
    });
    CorrectionState {
        text: active.last().and_then(|r| r.replacement_text.clone()),
        speaker,
        can_undo: !active.is_empty(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn r(action: &str, profile: Option<&str>, text: Option<&str>) -> Correction {
        Correction {
            id: action.into(),
            segment_id: "segment".into(),
            profile_id: profile.map(str::to_owned),
            replacement_text: text.map(str::to_owned),
            action: action.into(),
            created_at: "fixture".into(),
        }
    }
    #[test]
    fn unknown_survives_text_edits_and_undo_restores_identity_separately() {
        let mut history = vec![
            r("assign", Some("person"), None),
            r("unknown", None, None),
            r("text", None, Some("corrected")),
        ];
        let state = resolve("segment", &history);
        assert_eq!(state.speaker, Some(SpeakerOverride::Unknown));
        assert_eq!(state.text.as_deref(), Some("corrected"));
        history.push(r("undo", None, None));
        assert_eq!(
            resolve("segment", &history).speaker,
            Some(SpeakerOverride::Unknown)
        );
        history.push(r("undo", Some("person"), None));
        assert_eq!(
            resolve("segment", &history).speaker,
            Some(SpeakerOverride::Profile("person".into()))
        );
        history.push(r("undo", None, None));
        assert!(!resolve("segment", &history).can_undo);
        assert!(resolve("segment", &history).speaker.is_none());
    }
    #[test]
    fn text_alone_does_not_claim_identity_and_other_segments_do_not_leak() {
        let mut unrelated = r("assign", Some("other"), None);
        unrelated.segment_id = "different".into();
        let history = [unrelated, r("text", None, Some("words"))];
        let state = resolve("segment", &history);
        assert!(state.speaker.is_none());
        assert!(state.can_undo);
        assert_eq!(state.text.as_deref(), Some("words"));
    }
}
