//! Prepared reader data. Raw evidence stays in RecordingDetail.
use super::*;
use parakeet_domain::corrections::{self, CorrectionState};
use std::{ops::Deref, rc::Rc};

pub(super) struct Document {
    detail: RecordingDetail,
    pub row_corrections: Vec<CorrectionState>,
    folded: Vec<String>,
}
impl Deref for Document {
    type Target = RecordingDetail;
    fn deref(&self) -> &Self::Target {
        &self.detail
    }
}
impl Document {
    /// Called in the existing background application task, never during rendering.
    pub fn new(detail: RecordingDetail) -> Self {
        let corrections = detail
            .segments
            .iter()
            .map(|s| corrections::resolve(&s.id, &detail.corrections))
            .collect::<Vec<_>>();
        let folded = detail
            .segments
            .iter()
            .zip(&corrections)
            .map(|(s, c)| c.text.as_deref().unwrap_or(&s.text).to_lowercase())
            .collect();
        Self {
            detail,
            row_corrections: corrections,
            folded,
        }
    }
    pub fn matching(&self, query: &str) -> Rc<Vec<usize>> {
        let query = query.to_lowercase();
        Rc::new(
            self.folded
                .iter()
                .enumerate()
                .filter_map(|(i, text)| (query.is_empty() || text.contains(&query)).then_some(i))
                .collect(),
        )
    }
}
impl Workbench {
    /// Invalidate variable-height measurements after page/data/editor changes.
    /// Preserve the logical top when only the row's content changed.
    pub(super) fn reset_reader_rows(&self, from_top: bool) {
        let offset = self.reader_list.logical_scroll_top();
        let count = self
            .reader_matches
            .len()
            .saturating_sub(self.segment_page * 40)
            .min(40);
        self.reader_list.reset(0);
        let start = self.segment_page * 40;
        self.reader_list.splice_focusable(
            0..0,
            self.reader_matches
                .iter()
                .skip(start)
                .take(count)
                .map(|index| {
                    let editing = self
                        .detail
                        .as_ref()
                        .and_then(|d| d.segments.get(*index))
                        .is_some_and(|segment| {
                            self.editing_segment.as_deref() == Some(segment.id.as_str())
                        });
                    editing.then(|| self.reader_editor_focus.clone())
                }),
        );
        if !from_top {
            self.reader_list.scroll_to(offset);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn detail() -> RecordingDetail {
        serde_json::from_value(serde_json::json!({
            "recording":{"id":"r","source_key":"fixture","source_kind":"meeting","source_title":"Fixture","title_override":null,"created_at":"2026-01-01","duration_ms":10000,"metadata":{}},
            "tracks":[],"runs":[],"corrections":[],"attributions":[],"word_attributions":[],"unmapped_corrected_segments":[],
            "segments":[{"id":"first","recording_id":"r","run_id":"run","track_id":null,"ordinal":0,"start_ms":0,"end_ms":1000,"text":"Original words","speaker_claim":null,"evidence":{}},
                        {"id":"second","recording_id":"r","run_id":"run","track_id":null,"ordinal":1,"start_ms":2000,"end_ms":3000,"text":"Other speech","speaker_claim":null,"evidence":{}}]
        })).unwrap()
    }
    #[::core::prelude::v1::test]
    fn search_tracks_corrections_and_undo_without_overwriting_source_evidence() {
        let mut raw = detail();
        raw.corrections = vec![Correction {
            id: "c".into(),
            segment_id: "first".into(),
            profile_id: None,
            replacement_text: Some("Corrected wording".into()),
            action: "text".into(),
            created_at: "fixture".into(),
        }];
        let prepared = Document::new(raw.clone());
        assert_eq!(&*prepared.matching("CORRECTED"), &[0]);
        assert!(prepared.matching("original").is_empty());
        assert_eq!(prepared.segments[0].text, "Original words");
        assert_eq!(&*prepared.matching(""), &[0, 1]);
        raw.corrections.push(Correction {
            id: "undo".into(),
            segment_id: "first".into(),
            profile_id: None,
            replacement_text: None,
            action: "undo".into(),
            created_at: "fixture".into(),
        });
        let refreshed = Document::new(raw);
        assert_eq!(&*refreshed.matching("original"), &[0]);
        assert!(refreshed.matching("corrected").is_empty());
        assert_eq!(&*refreshed.matching("OTHER"), &[1]);
    }
}
