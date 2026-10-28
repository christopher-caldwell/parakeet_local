//! Native controls over the shared durable analysis controller.
use super::*;
use parakeet_application::analysis::AnalyzeMeetingInput;
impl Workbench {
    pub(super) fn analysis_panel(&self, detail: &reader::Document, cx: &mut Context<Self>) -> Div {
        let mut panel=div().p_4().rounded_lg().bg(rgb(PANEL)).flex().flex_col().gap_3()
            .child(section("Meeting & speakers", "Transcribe sources, diarize and prepare playable voice examples. Names stay unconfirmed until you identify them."));
        match &detail.analysis {
            Ok(Some(plan)) => {
                panel = panel
                    .child(caption(format!(
                        "{} · {} · {} sources · {} voice clips · {} comparisons",
                        plan.status,
                        plan.stage,
                        plan.sources.len(),
                        plan.candidates.len(),
                        plan.suggestions.len()
                    )))
                    .child(caption(format!(
                        "ASR {} · diarization {}",
                        plan.asr.id, plan.diarization.id
                    )))
                    .child(caption(if plan.matching_policy.is_some() {
                        "Saved explicit identity policy; resume uses the same thresholds and model."
                    } else {
                        "Anonymous analysis. No identity comparison requested."
                    }));
                for failure in plan.failures.iter().take(8) {
                    panel = panel.child(caption(failure.clone()).text_color(rgb(WARNING)));
                }
                let candidates = plan.candidates.clone();
                panel = panel.child(
                    quiet("analysis-review-speakers", "Review prepared voices")
                        .disabled(candidates.is_empty())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.candidates = candidates.clone();
                            this.speaker_mode = SpeakerMode::Profiles;
                            this.area = Area::Speakers;
                            this.refresh_profiles(cx);
                            cx.notify();
                        })),
                );
            }
            Ok(None) => {
                panel=panel.child(caption("No saved analysis. Uses the installed models selected in Models; no weights are downloaded."));
            }
            Err(error) => {
                panel = panel.child(
                    caption(format!("Saved analysis needs inspection: {error}"))
                        .text_color(rgb(WARNING)),
                );
            }
        }
        panel=panel.child(div().flex().flex_wrap().gap_2()
            .child(button("analyze-meeting","Analyze / resume meeting").disabled(self.pending>0).on_click(cx.listener(|this,_,_,cx|this.analyze_meeting(false,false,cx))))
            .child(quiet("analysis-new-anonymous","New anonymous analysis").disabled(self.pending>0).on_click(cx.listener(|this,_,_,cx|this.analyze_meeting(true,false,cx)))))
            .child(caption("Optional identity comparison: enter all three thresholds. Similarity is not a probability; personal thresholds have not been calibrated."))
            .child(div().flex().flex_wrap().gap_2().child(field(&self.match_score).w(px(200.))).child(field(&self.match_margin).w(px(160.))).child(field(&self.match_examples).w(px(190.))))
            .child(quiet("analysis-new-matching","New analysis with entered policy").disabled(self.pending>0).on_click(cx.listener(|this,_,_,cx|this.analyze_meeting(true,true,cx))));
        panel
    }
    fn analyze_meeting(&mut self, new_run: bool, matching: bool, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else { return };
        let recording_id = detail.recording.id.clone();
        let matching_policy = if matching {
            match (
                self.match_score.read(cx).value().parse::<f64>(),
                self.match_margin.read(cx).value().parse::<f64>(),
                self.match_examples.read(cx).value().parse::<usize>(),
            ) {
                (Ok(minimum_score), Ok(minimum_margin), Ok(minimum_examples)) => {
                    Some(parakeet_domain::recognition::MatchPolicy {
                        minimum_score,
                        minimum_margin,
                        minimum_examples,
                    })
                }
                _ => {
                    self.status="Enter similarity, separation and verified reference count before requesting comparison".into();
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let service = self.analysis.clone();
        let library = self.library.clone();
        self.task(
            "Analyzing meeting locally; source audio and corrections retained",
            async move {
                let warning = service
                    .analyze(AnalyzeMeetingInput {
                        recording_id: recording_id.clone(),
                        matching_policy,
                        new_run,
                    })
                    .await
                    .err()
                    .map(|e| e.to_string());
                let detail = library
                    .show(&recording_id)
                    .await
                    .map(reader::Document::new)
                    .map_err(|e| e.to_string());
                Ok(Update::MeetingFinalized(detail, warning))
            },
            cx,
        );
    }
}
