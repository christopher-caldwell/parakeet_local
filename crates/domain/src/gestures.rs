use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Shortcut {
    pub id: String,
    pub key: String,
    pub modifiers: Vec<String>,
    pub enabled: bool,
}
/// Keys held on their own as the shortcut (e.g. Right Option), distinguishing left from right.
pub const MODIFIER_ONLY_KEYS: [&str; 9] = [
    "rightoption",
    "leftoption",
    "rightcommand",
    "leftcommand",
    "rightcontrol",
    "leftcontrol",
    "rightshift",
    "leftshift",
    "fn",
];
impl Shortcut {
    pub fn validate(&self) -> Result<(), crate::ValidationError> {
        if self.id.is_empty()
            || self.key.is_empty()
            || self
                .modifiers
                .iter()
                .any(|m| !["cmd", "ctrl", "alt", "shift"].contains(&m.as_str()))
        {
            return Err(crate::ValidationError(
                "shortcut needs an ID, key, and canonical cmd/ctrl/alt/shift modifiers".into(),
            ));
        }
        let key = self.key.to_lowercase();
        let special = [
            "home", "end", "pageup", "pagedown", "space", "escape", "f1", "f2", "f3", "f4", "f5",
            "f6", "f7", "f8", "f9", "f10", "f11", "f12",
        ];
        let typing = key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric());
        let modifier_only = MODIFIER_ONLY_KEYS.contains(&key.as_str());
        if !special.contains(&key.as_str()) && !typing && !modifier_only {
            return Err(crate::ValidationError(format!(
                "unsupported key; use Home/End/PageUp/PageDown/Space/Escape, F1–F12, a letter or digit with a modifier, or a modifier alone ({})",
                MODIFIER_ONLY_KEYS.join(", ")
            )));
        }
        // A bare letter or digit would hijack typing everywhere.
        if typing && self.modifiers.is_empty() {
            return Err(crate::ValidationError(
                "a letter or digit shortcut needs at least one modifier".into(),
            ));
        }
        if modifier_only && !self.modifiers.is_empty() {
            return Err(crate::ValidationError(
                "a modifier-only shortcut cannot also have modifiers".into(),
            ));
        }
        if self
            .modifiers
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != self.modifiers.len()
        {
            return Err(crate::ValidationError("shortcut modifier repeated".into()));
        }
        if self.enabled && self.modifiers.iter().any(|m| m == "cmd") && key == "space" {
            return Err(crate::ValidationError(
                "Command-Space conflicts with the standard macOS Spotlight shortcut".into(),
            ));
        }
        Ok(())
    }

    pub fn conflicts(&self, other: &Self) -> bool {
        let mut a = self.modifiers.clone();
        a.sort();
        a.dedup();
        let mut b = other.modifiers.clone();
        b.sort();
        b.dedup();
        self.enabled && other.enabled && self.key.eq_ignore_ascii_case(&other.key) && a == b
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InsertionTarget {
    pub bundle_id: String,
    pub process_id: i32,
    pub element_token: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GestureEffect {
    None,
    Start(InsertionTarget),
    Finish(InsertionTarget),
    Cancel,
}
#[derive(Debug, Clone, Default)]
pub struct HoldGesture {
    owner: Option<String>,
    target: Option<InsertionTarget>,
    deadline_ms: u64,
}
impl HoldGesture {
    pub fn down(&mut self, binding: &str, target: InsertionTarget, now_ms: u64) -> GestureEffect {
        if self.owner.is_some() {
            return GestureEffect::None;
        }
        self.owner = Some(binding.into());
        self.target = Some(target.clone());
        self.deadline_ms = now_ms.saturating_add(120_000);
        GestureEffect::Start(target)
    }
    pub fn up(&mut self, binding: &str) -> GestureEffect {
        if self.owner.as_deref() != Some(binding) {
            return GestureEffect::None;
        }
        self.owner = None;
        self.target
            .take()
            .map(GestureEffect::Finish)
            .unwrap_or(GestureEffect::None)
    }
    pub fn cancel(&mut self) -> GestureEffect {
        self.owner = None;
        self.target = None;
        GestureEffect::Cancel
    }
    pub fn tick(&mut self, now_ms: u64) -> GestureEffect {
        if self.owner.is_some() && now_ms >= self.deadline_ms {
            self.cancel()
        } else {
            GestureEffect::None
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> InsertionTarget {
        InsertionTarget {
            bundle_id: "com.example".into(),
            process_id: 1,
            element_token: "focus".into(),
        }
    }
    fn shortcut(key: &str, modifiers: &[&str]) -> Shortcut {
        Shortcut {
            id: key.into(),
            key: key.into(),
            modifiers: modifiers.iter().map(|m| m.to_string()).collect(),
            enabled: true,
        }
    }
    #[test]
    fn keys_for_a_laptop_and_an_external_keyboard_validate() {
        assert!(shortcut("home", &[]).validate().is_ok());
        assert!(shortcut("rightoption", &[]).validate().is_ok());
        assert!(shortcut("fn", &[]).validate().is_ok());
        assert!(shortcut("d", &["ctrl", "alt"]).validate().is_ok());
        assert!(shortcut("7", &["cmd", "shift"]).validate().is_ok());
        // Bare typing keys, modifier-only with extra modifiers, and unknown keys are refused.
        assert!(shortcut("d", &[]).validate().is_err());
        assert!(shortcut("rightoption", &["cmd"]).validate().is_err());
        assert!(shortcut("volumeup", &[]).validate().is_err());
    }
    #[test]
    fn modifier_only_holds_own_their_release_and_conflicts_are_detected() {
        let mut g = HoldGesture::default();
        let left = shortcut("leftoption", &[]);
        let right = shortcut("rightoption", &[]);
        assert!(!left.conflicts(&right));
        assert!(left.conflicts(&shortcut("leftoption", &[])));
        // Same state machine as any other key: only the owner's release ends the hold.
        assert_eq!(g.down("left", target(), 0), GestureEffect::Start(target()));
        assert_eq!(g.down("right", target(), 1), GestureEffect::None);
        assert_eq!(g.up("right"), GestureEffect::None);
        assert_eq!(g.up("left"), GestureEffect::Finish(target()));
    }
    #[test]
    fn ownership_repeat_and_target_are_stable() {
        let mut g = HoldGesture::default();
        assert!(matches!(
            g.down("home", target(), 0),
            GestureEffect::Start(_)
        ));
        assert_eq!(g.down("home", target(), 1), GestureEffect::None);
        assert_eq!(g.down("f6", target(), 2), GestureEffect::None);
        assert_eq!(g.up("f6"), GestureEffect::None);
        assert_eq!(g.up("home"), GestureEffect::Finish(target()));
    }
    #[test]
    fn lost_release_cancels_without_paste() {
        let mut g = HoldGesture::default();
        g.down("home", target(), 0);
        assert_eq!(g.tick(120_000), GestureEffect::Cancel);
        assert_eq!(g.up("home"), GestureEffect::None);
    }
}
