use crate::{SpeakerInterval, ValidationError};
/// Split a run-local cluster at an explicit meeting time. Preserve overlap and all other voices.
pub fn split(
    intervals: &[SpeakerInterval],
    speaker: &str,
    at_ms: i64,
    new_speaker: &str,
) -> Result<Vec<SpeakerInterval>, ValidationError> {
    if at_ms < 0
        || speaker == new_speaker
        || new_speaker.is_empty()
        || intervals.iter().any(|i| i.speaker == new_speaker)
    {
        return Err(ValidationError(
            "new cluster needs a distinct unused identity and a nonnegative split time".into(),
        ));
    }
    let selected: Vec<_> = intervals.iter().filter(|i| i.speaker == speaker).collect();
    if !selected.iter().any(|i| i.start_ms < at_ms) || !selected.iter().any(|i| i.end_ms > at_ms) {
        return Err(ValidationError(
            "split must leave speech on both sides".into(),
        ));
    }
    let mut result = vec![];
    for i in intervals {
        if i.speaker != speaker || i.end_ms <= at_ms {
            result.push(i.clone());
            continue;
        }
        if i.start_ms < at_ms {
            let mut before = i.clone();
            before.end_ms = at_ms;
            result.push(before);
        }
        let mut after = i.clone();
        after.start_ms = after.start_ms.max(at_ms);
        after.speaker = new_speaker.into();
        result.push(after);
    }
    result.sort_by(|a, b| a.start_ms.cmp(&b.start_ms).then(a.speaker.cmp(&b.speaker)));
    Ok(result)
}
pub fn merge(
    intervals: &[SpeakerInterval],
    speakers: &[String],
    new_speaker: &str,
) -> Result<Vec<SpeakerInterval>, ValidationError> {
    let unique: std::collections::BTreeSet<_> = speakers.iter().collect();
    if unique.len() < 2 || new_speaker.is_empty() {
        return Err(ValidationError(
            "merge needs at least two distinct clusters".into(),
        ));
    }
    let mut track = None;
    for name in &unique {
        let selected: Vec<_> = intervals.iter().filter(|i| &i.speaker == *name).collect();
        if selected.is_empty() {
            return Err(ValidationError(format!("unknown cluster {name}")));
        }
        for i in selected {
            if let Some(previous) = &track {
                if previous != &i.track_id {
                    return Err(ValidationError("cannot merge across audio sources".into()));
                }
            } else {
                track = Some(i.track_id.clone());
            }
        }
    }
    if intervals
        .iter()
        .any(|i| i.speaker == new_speaker && !unique.contains(&i.speaker))
    {
        return Err(ValidationError(
            "new cluster collides with an unrelated speaker".into(),
        ));
    }
    let mut merged: Vec<SpeakerInterval> = vec![];
    let mut others = vec![];
    for i in intervals {
        let mut i = i.clone();
        if unique.contains(&i.speaker) {
            i.speaker = new_speaker.into();
            merged.push(i);
        } else {
            others.push(i);
        }
    }
    merged.sort_by_key(|i| i.start_ms);
    let mut union: Vec<SpeakerInterval> = vec![];
    for i in merged {
        if let Some(last) = union.last_mut()
            && last.end_ms >= i.start_ms
        {
            last.end_ms = last.end_ms.max(i.end_ms);
            continue;
        }
        union.push(i);
    }
    others.extend(union);
    others.sort_by(|a, b| a.start_ms.cmp(&b.start_ms).then(a.speaker.cmp(&b.speaker)));
    Ok(others)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn interval(a: i64, b: i64, name: &str) -> SpeakerInterval {
        SpeakerInterval {
            start_ms: a,
            end_ms: b,
            speaker: name.into(),
            track_id: Some("t".into()),
        }
    }
    #[test]
    fn split_and_merge_preserve_coverage_and_other_speakers() {
        let raw = vec![interval(0, 1000, "a"), interval(200, 700, "b")];
        let split = split(&raw, "a", 500, "c").unwrap();
        assert_eq!(split.len(), 3);
        assert_eq!(raw[0].end_ms, 1000);
        let merged = merge(&split, &["a".into(), "c".into()], "a").unwrap();
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].end_ms, 1000);
        assert_eq!(merged[1].speaker, "b");
    }
    #[test]
    fn invalid_cluster_and_cross_source_merge_are_rejected() {
        let mut raw = vec![interval(0, 1000, "a"), interval(200, 700, "b")];
        raw[1].track_id = Some("other".into());
        assert!(merge(&raw, &["a".into(), "b".into()], "c").is_err());
        assert!(split(&raw, "a", 2000, "c").is_err());
    }
}
