//! Native controls over the shared durable analysis controller.
use super::*;
use parakeet_application::analysis::AnalyzeMeetingInput;
impl Workbench {
    pub(super) fn analysis_panel(&self, detail: &reader::Document, cx: &mut Context<Self>) -> Div {
        let waiting = self.waiting_voices().len();
        let mut panel = panel().child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Speakers"),
                )
                .child(caption(
                    "Finds the separate voices in this meeting. Names stay unconfirmed until you name them.",
                )),
        );
        match &detail.analysis {
            Ok(Some(plan)) => {
                let (label, color) = match plan.status.as_str() {
                    "completed" => ("Speakers detected", SUCCESS),
                    "failed" => ("Detection failed", DANGER),
                    _ => ("Detection in progress", WARNING),
                };
                panel = panel.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(pill(label, color))
                        .child(caption(match waiting {
                            0 => "No voices are waiting to be named.".to_owned(),
                            1 => "1 voice is waiting to be named.".to_owned(),
                            n => format!("{n} voices are waiting to be named."),
                        })),
                );
                if self.show_status_details {
                    panel = panel
                        .child(caption(format!(
                            "{} · {} sources · {} comparisons",
                            plan.stage,
                            plan.sources.len(),
                            plan.suggestions.len()
                        )))
                        .child(caption(format!(
                            "Transcription {} · speaker detection {}",
                            plan.asr.id, plan.diarization.id
                        )))
                        .child(caption(if plan.matching_policy.is_some() {
                            "Saved explicit identity policy; resume uses the same thresholds and model."
                        } else {
                            "Anonymous analysis. No identity comparison requested."
                        }));
                }
                for failure in plan.failures.iter().take(8) {
                    panel = panel.child(caption(failure.clone()).text_color(rgb(WARNING)));
                }
            }
            Ok(None) => {
                panel = panel.child(caption(
                    "Not done yet. Detecting speakers uses the speaker model marked In use under Local models.",
                ));
            }
            Err(error) => {
                panel = panel.child(
                    caption(format!("Saved analysis needs inspection: {error}"))
                        .text_color(rgb(WARNING)),
                );
            }
        }
        let any_candidates = waiting > 0;
        panel = panel.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(if any_candidates {
                    primary("analysis-review-speakers", "Name the voices")
                        .on_click(cx.listener(|this, _, _, cx| this.open_voices(cx)))
                } else {
                    button("analysis-review-speakers", "Name the voices")
                        .disabled(true)
                        .on_click(cx.listener(|this, _, _, cx| this.open_voices(cx)))
                })
                .child(
                    button("analyze-meeting", "Detect speakers")
                        .disabled(self.pending > 0)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.analyze_meeting(false, false, cx)),
                        ),
                )
                .child(
                    quiet("analysis-new-anonymous", "Start over")
                        .disabled(self.pending > 0)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.analyze_meeting(true, false, cx)),
                        ),
                ),
        );
        if self.show_status_details {
            panel=panel
                .child(caption("Advanced: compare voices with named people while detecting. Enter all three thresholds; similarity is not a probability and these have not been calibrated for you."))
                .child(div().flex().flex_wrap().gap_2().child(field(&self.match_score).w(px(200.))).child(field(&self.match_margin).w(px(160.))).child(field(&self.match_examples).w(px(190.))))
                .child(quiet("analysis-new-matching","New analysis with entered policy").disabled(self.pending>0).on_click(cx.listener(|this,_,_,cx|this.analyze_meeting(true,true,cx))));
        }
        panel
    }
    pub(super) fn analyze_meeting(
        &mut self,
        new_run: bool,
        matching: bool,
        cx: &mut Context<Self>,
    ) {
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
