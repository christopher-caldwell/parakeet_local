//! Prepared reader data. Raw evidence stays in RecordingDetail.
use super::*;
use parakeet_domain::corrections::{self, CorrectionState};
use std::{collections::HashMap, ops::Deref, rc::Rc};

pub(super) struct Document {
    detail: RecordingDetail,
    pub analysis: Result<Option<parakeet_application::analysis::MeetingAnalysis>, String>,
    pub row_corrections: Vec<CorrectionState>,
    pub row_attributions: Vec<Option<usize>>,
    pub row_word_indices: Vec<Vec<usize>>,
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
        let mut attributions = HashMap::new();
        for (i, attribution) in detail.attributions.iter().enumerate() {
            // Match the prior reader's first-claim semantics if malformed input repeats an ID.
            attributions
                .entry(attribution.segment_id.as_str())
                .or_insert(i);
        }
        let row_attributions = detail
            .segments
            .iter()
            .map(|s| attributions.get(s.id.as_str()).copied())
            .collect();
        let rows: HashMap<_, _> = detail
            .segments
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.as_str(), i))
            .collect();
        let mut row_word_indices = vec![Vec::new(); detail.segments.len()];
        for (i, word) in detail.word_attributions.iter().enumerate() {
            if let Some(row) = rows.get(word.segment_id.as_str()) {
                let indices = &mut row_word_indices[*row];
                // The expanded reader displays at most 100 words per row, as before.
                // Raw evidence remains complete in detail.word_attributions.
                if indices.len() < 100 {
                    indices.push(i);
                }
            }
        }
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
        let analysis = detail
            .recording
            .metadata
            .get("meeting_analysis")
            .filter(|v| !v.is_null())
            .map(|v| {
                let plan: parakeet_application::analysis::MeetingAnalysis =
                    serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
                plan.validate().map_err(|e| e.to_string())?;
                Ok(plan)
            })
            .transpose();
        Self {
            detail,
            analysis,
            row_corrections: corrections,
            row_attributions,
            row_word_indices,
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
    pub(super) fn refresh_reader_matches(&mut self) {
        self.reader_matches = self
            .detail
            .as_ref()
            .map(|d| d.matching(&self.transcript_query))
            .unwrap_or_default();
        self.reset_reader_rows(true);
    }
    /// Re-measures only the line being edited as its editor grows or shrinks; every other row
    /// keeps its measured height.
    pub(super) fn remeasure_editing_row(&self) {
        let Some(detail) = &self.detail else { return };
        let editing = self.editing_segment.as_deref();
        if let Some(row) = self.reader_matches.iter().position(|&index| {
            detail
                .segments
                .get(index)
                .is_some_and(|segment| Some(segment.id.as_str()) == editing)
        }) {
            self.reader_list.remeasure_items(row..row + 1);
        }
    }
    /// Invalidate variable-height measurements after data/editor changes. The whole transcript
    /// is one list; only rows near the viewport are measured and drawn.
    /// Preserve the logical top when only the row's content changed.
    pub(super) fn reset_reader_rows(&self, from_top: bool) {
        let offset = self.reader_list.logical_scroll_top();
        self.reader_list.reset(0);
        self.reader_list.splice_focusable(
            0..0,
            self.reader_matches.iter().map(|index| {
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

    #[::core::prelude::v1::test]
    fn evidence_indices_keep_source_order_and_bounds_without_losing_raw_words() {
        use parakeet_domain::{reconciliation::SegmentAttribution, word_timing::*};
        let mut raw = detail();
        let attribution = |id: &str| SegmentAttribution {
            segment_id: id.into(),
            state: "unknown".into(),
            anonymous_speakers: vec![],
            speech_coverage: 0.,
            overlapping_speech: false,
            overlapping_source_audio: false,
            possible_cross_source_duplicate: false,
        };
        raw.attributions = vec![
            attribution("second"),
            attribution("missing"),
            attribution("second"),
        ];
        let word = |id: &str, ordinal| WordAttribution {
            segment_id: id.into(),
            ordinal,
            word: RecognizedWord {
                text: format!("word {ordinal}"),
                local_start_ms: None,
                local_end_ms: None,
                start_ms: None,
                end_ms: None,
                timing_quality: "unknown".into(),
            },
            attribution: attribution(id),
        };
        raw.word_attributions = vec![
            word("missing", 0),
            word("second", 5),
            word("first", 8),
            word("second", 1),
        ];
        raw.word_attributions
            .extend((0..105).map(|n| word("first", n)));
        let doc = Document::new(raw);
        assert_eq!(doc.row_attributions, vec![None, Some(0)]);
        assert_eq!(doc.row_word_indices[1], vec![1, 3]);
        assert_eq!(doc.row_word_indices[0].len(), 100);
        assert_eq!(doc.row_word_indices[0][0], 2);
        assert_eq!(doc.row_word_indices[0][99], 102);
        assert_eq!(doc.word_attributions.len(), 109);
        assert_eq!(doc.word_attributions[2].word.timing_quality, "unknown");
        assert_eq!(doc.word_attributions[1].ordinal, 5);
        assert_eq!(doc.word_attributions[3].ordinal, 1);
    }
}
