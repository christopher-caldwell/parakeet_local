//! Naming the voices found in a recording: listen to a clip, type who it is, move on. Naming a
//! voice goes through the shared `identify_cluster` use case and can be undone as one unit.
use super::*;

/// One numbered instruction row: a small badge and a title, with the control beneath it.
fn step(number: usize, title: &str, body: impl IntoElement) -> Div {
    div()
        .flex()
        .gap_3()
        .child(
            div()
                .mt_0p5()
                .size(px(22.))
                .flex_shrink_0()
                .rounded_full()
                .bg(rgb(SELECTED))
                .text_color(rgb(ACCENT))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(number.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_owned()),
                )
                .child(body),
        )
}

/// Clips of the current voice the user has played: candidate id, next sample, ranges heard.
pub(super) type HeardClips = (String, usize, Vec<(i64, i64)>);

impl Workbench {
    /// Voices found in the open recording that nobody has named or skipped yet.
    pub(super) fn waiting_voices(&self) -> Vec<EnrollmentCandidate> {
        self.candidates
            .iter()
            .filter(|c| !self.voice_skipped(c) && !self.train_named(c))
            .cloned()
            .collect()
    }
    fn voice_skipped(&self, candidate: &EnrollmentCandidate) -> bool {
        self.audio_preferences
            .skipped_voices
            .contains(&candidate.id)
    }
    /// Skipping is remembered, so people who will not come up again stop being offered.
    fn set_voices_skipped(&mut self, ids: Vec<String>, skipped: bool, cx: &mut Context<Self>) {
        self.change_preferences(
            "Saving voices to skip",
            "Saved",
            move |p| {
                for id in &ids {
                    p.skipped_voices.retain(|existing| existing != id);
                    if skipped {
                        p.skipped_voices.push(id.clone());
                    }
                }
            },
            cx,
        );
    }
    pub(super) fn has_waiting_voices(&self) -> bool {
        self.candidates
            .iter()
            .any(|c| !self.voice_skipped(c) && !self.train_named(c))
    }
    /// Reads the prepared voices and the names already given for a recording.
    pub(super) fn load_voices(&mut self, recording: String, cx: &mut Context<Self>) {
        self.voices_generation += 1;
        let generation = self.voices_generation;
        self.voices_loading = true;
        self.voices_error = None;
        let service = self.speakers.clone();
        let analysis = self.analysis.clone();
        self.task(
            "Finding the voices in this recording",
            async move {
                let result = async {
                    let candidates = service.candidates(&recording).await?;
                    let named = service.identifications(Some(&recording)).await?;
                    Ok::<_, anyhow::Error>((
                        candidates,
                        named,
                        analysis.is_active(&recording).await?,
                    ))
                }
                .await
                .map_err(|e| e.to_string());
                Ok(Update::Candidates(recording, generation, result))
            },
            cx,
        );
    }
    /// Jumps to the naming screen of the open recording.
    pub(super) fn open_voices(&mut self, cx: &mut Context<Self>) {
        self.area = Area::Library;
        self.recording_tab = library::RecordingTab::Voices;
        self.train_hint = None;
        self.train_person = None;
        self.train_heard = None;
        // People shares this field; a name left from there must not pre-fill a voice.
        let name = self.person_name.clone();
        let _ = cx.update_window(self.main_window, move |_, window, cx| {
            name.update(cx, |input, cx| input.set_value("", window, cx));
        });
        self.reset_reader_rows(true);
        self.refresh_profiles(cx);
        if let Some(detail) = &self.detail {
            let id = detail.recording.id.clone();
            self.load_voices(id, cx);
        }
        cx.notify();
    }
    pub(super) fn voices_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div().p_5().flex().flex_col().gap_4().child(section(
            "Name the voices",
            "Listen to a voice, then choose a name. Names are only saved when you confirm them, and can be undone.",
        ));
        let Some(detail) = &self.detail else {
            return view;
        };
        if let Some(error) = &self.voices_error {
            return view
                .child(
                    panel()
                        .child("Voices could not be loaded")
                        .child(caption(error.clone())),
                )
                .child(
                    button("retry-voices", "Try again")
                        .on_click(cx.listener(|this, _, _, cx| this.open_voices(cx))),
                );
        }
        if self.voices_loading && self.candidates.is_empty() {
            return view.child(panel().child("Loading voices…"));
        }
        let queue = self.waiting_voices();
        if let Some(candidate) = queue.first().cloned() {
            let position = self.candidates.len() - queue.len() + 1;
            let heard = detail
                .tracks
                .iter()
                .find(|t| t.id == candidate.track_id)
                .map(|t| match t.kind {
                    TrackKind::Microphone => "on your microphone",
                    TrackKind::System => "on the meeting audio",
                    _ => "in this recording",
                })
                .unwrap_or("in this recording");
            let mut card = div()
                .p_5()
                .rounded_lg()
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(BORDER))
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(18.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("Voice {position} of {}", self.candidates.len())),
                        )
                        .child(caption(format!(
                            "{} heard {heard} · {:.0} s of clear speech",
                            anonymous_label(&candidate.anonymous_speaker),
                            candidate.speech_ms as f64 / 1000.
                        ))),
                );
            card = card
                .child(step(
                    1,
                    "Listen",
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(self.play_button(&candidate, cx))
                        .child(caption(
                            "If it sounds like more than one person, choose Skip.",
                        ))
                        .when(
                            detail.runs.iter().any(|r| {
                                r.id == candidate.run_id
                                    && r.result["metrics"]["speaker_slots_saturated"] == true
                            }),
                            |column| {
                                column.child(
                                    div().text_size(px(12.)).text_color(rgb(WARNING)).child(
                                        "Speaker detection used all of its slots on this source, so different people may share this voice. Play a few samples before naming it.",
                                    ),
                                )
                            },
                        ),
                ))
                .child(step(2, "Type who it is", self.name_picker(cx)))
                .child(step(3, "Save", self.save_row(&candidate, cx)));
            view = view.child(card);
        } else if self.candidates.is_empty()
            && !detail
                .tracks
                .iter()
                .any(|t| t.available && t.kind != TrackKind::Playback)
        {
            view = view.child(
                div()
                    .p_5()
                    .rounded_lg()
                    .bg(rgb(PANEL))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("The audio is no longer on this Mac"),
                    )
                    .child(caption(
                        "Voices can only be detected from the original audio, so this recording can't be analyzed.",
                    )),
            );
        } else if self.candidates.is_empty() {
            let state = detail
                .analysis
                .as_ref()
                .ok()
                .and_then(|p| p.as_ref())
                .map(|p| p.status.as_str());
            let running = self.analysis_active;
            let interrupted = !running && matches!(state, Some("running" | "interrupted"));
            let (title, hint) = if running {
                (
                    "Detecting voices…",
                    "You can read the transcript while this runs. Voices will appear here when ready.",
                )
            } else if interrupted {
                (
                    "Voice detection was interrupted",
                    "Your recording and completed work are saved. Resume detection to finish preparing the voices.",
                )
            } else if state == Some("completed") {
                (
                    "No clear voices to name",
                    "Detection finished, but found no voice with enough clear speech. You can still read and correct the transcript.",
                )
            } else if state == Some("failed") {
                (
                    "Voice detection needs another try",
                    "Your recording is saved. Retry detection or see Processing for details.",
                )
            } else {
                (
                    "Find the voices in this recording",
                    "Detect speakers to prepare short clips you can listen to and name.",
                )
            };
            view = view.child(
                div()
                    .p_5()
                    .rounded_lg()
                    .bg(rgb(PANEL))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(caption(hint))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                primary(
                                    "voices-detect",
                                    if interrupted {
                                        "Resume detection"
                                    } else if state == Some("failed") {
                                        "Retry detection"
                                    } else {
                                        "Detect speakers"
                                    },
                                )
                                .disabled(running || self.pending > 0 || state == Some("completed"))
                                .on_click(cx.listener(
                                    |this, _, _, cx| this.analyze_meeting(false, false, cx),
                                )),
                            )
                            .child(
                                quiet("voices-refresh", "Look again")
                                    .disabled(self.pending > 0)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(d) = &this.detail {
                                            let id = d.recording.id.clone();
                                            this.load_voices(id, cx);
                                        }
                                    })),
                            ),
                    ),
            );
        } else {
            view = view.child(
                div()
                    .p_5()
                    .rounded_lg()
                    .bg(rgb(PANEL))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(SUCCESS))
                            .child("Every voice in this recording is handled"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(primary("voices-done", "Back to the transcript").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.recording_tab = library::RecordingTab::Transcript;
                                    this.reset_reader_rows(true);
                                    cx.notify();
                                }),
                            ))
                            .when(
                                self.candidates.iter().any(|c| self.voice_skipped(c)),
                                |row| {
                                    row.child(
                                        quiet("voices-unskip", "Bring back the voices I skipped")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let ids = this
                                                    .candidates
                                                    .iter()
                                                    .map(|c| c.id.clone())
                                                    .collect();
                                                this.set_voices_skipped(ids, false, cx);
                                            })),
                                    )
                                },
                            ),
                    ),
            );
        }
        let mut named = div().flex().flex_col().gap_2();
        let mut any = false;
        for (index, done) in self.train_done.iter().enumerate() {
            if done.undone_at.is_some() || done.recording_id != detail.recording.id {
                continue;
            }
            any = true;
            let id = done.id.clone();
            let source = detail
                .tracks
                .iter()
                .find(|t| t.id == done.track_id)
                .map(|t| match t.kind {
                    TrackKind::Microphone => " (microphone)",
                    TrackKind::System => " (meeting audio)",
                    _ => "",
                })
                .unwrap_or("");
            let name = self
                .profiles
                .iter()
                .find(|p| p.id == done.profile_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "removed person".into());
            named = named.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(format!(
                        "{}{source} → {name} · {} lines named",
                        anonymous_label(&done.cluster),
                        done.segments.len()
                    )))
                    .child(
                        quiet(("train-undo", index), "Undo")
                            .disabled(self.pending > 0)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let service = this.speakers.clone();
                                let id = id.clone();
                                this.task(
                                    "Undoing the name",
                                    async move {
                                        let report = service.undo_identification(&id).await?;
                                        Ok(Update::IdentificationUndone(report))
                                    },
                                    cx,
                                );
                            })),
                    ),
            );
        }
        if any {
            view = view.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(caption("NAMED IN THIS RECORDING"))
                    .child(named),
            );
        }
        view
    }
    /// Every clip of a voice: the representative turn first, then the other pooled turns.
    fn samples(candidate: &EnrollmentCandidate) -> Vec<(i64, i64)> {
        std::iter::once((candidate.local_start_ms, candidate.local_end_ms))
            .chain(candidate.more_ranges.iter().copied())
            .collect()
    }
    /// Ranges of this voice the user has played; only these become voice examples.
    fn heard(&self, candidate: &EnrollmentCandidate) -> Vec<(i64, i64)> {
        self.train_heard
            .as_ref()
            .filter(|(id, _, _)| *id == candidate.id)
            .map(|(_, _, heard)| heard.clone())
            .unwrap_or_default()
    }
    fn play_sample(&mut self, candidate: &EnrollmentCandidate, cx: &mut Context<Self>) {
        let samples = Self::samples(candidate);
        let (next, mut heard) = match self.train_heard.take() {
            Some((id, next, heard)) if id == candidate.id => (next, heard),
            _ => (0, vec![]),
        };
        let (start, end) = samples[next % samples.len()];
        if !heard.contains(&(start, end)) {
            heard.push((start, end));
        }
        self.train_heard = Some((candidate.id.clone(), next + 1, heard));
        self.train_hint = None;
        self.play_range(candidate.track_id.clone(), start, end, cx);
    }
    fn play_button(&self, candidate: &EnrollmentCandidate, cx: &mut Context<Self>) -> Div {
        let first = candidate.clone();
        let another = candidate.clone();
        let heard = self.heard(candidate).len();
        let total = Self::samples(candidate).len();
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                primary("train-play", "Play a clip")
                    .self_start()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.train_heard = None;
                        this.play_sample(&first, cx)
                    })),
            )
            .when(total > 1 && heard > 0, |row| {
                row.child(
                    quiet(
                        "train-play-another",
                        format!("Play another sample ({heard} of {total} heard)"),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.play_sample(&another, cx))),
                )
            })
    }
    /// A name field, with the people already named offered as shortcuts that fill it in.
    fn name_picker(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(field(&self.person_name).w_full().max_w(px(360.)));
        if !self.profiles.is_empty() {
            let mut people = div().flex().flex_wrap().items_center().gap_1();
            people = people.child(caption("Already named:"));
            for person in &self.profiles {
                let id = person.id.clone();
                let name = person.name.clone();
                people = people.child(
                    chip(
                        ("train-person", people_key(&person.id)),
                        person.name.clone(),
                        self.train_person.as_ref() == Some(&person.id),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.train_person = Some(id.clone());
                        this.train_hint = None;
                        this.person_name
                            .update(cx, |input, cx| input.set_value(name.clone(), window, cx));
                        cx.notify();
                    })),
                );
            }
            view = view.child(people);
        }
        view
    }
    fn save_row(&self, candidate: &EnrollmentCandidate, cx: &mut Context<Self>) -> Div {
        let skip = candidate.id.clone();
        let save = candidate.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        primary("train-save", "Save name")
                            .disabled(
                                self.pending > 0
                                    || (self.train_person.is_none()
                                        && self.person_name.read(cx).value().trim().is_empty()),
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.train_save(save.clone(), cx)
                                }),
                            ),
                    )
                    .child(
                        button("train-skip", "Skip this voice").on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.set_voices_skipped(vec![skip.clone()], true, cx);
                                this.train_person = None;
                                this.train_hint = None;
                                this.person_name
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                cx.notify();
                            },
                        )),
                    )
                    .when(self.waiting_voices().len() > 1, |row| {
                        row.child(
                            quiet("train-skip-rest", "Skip all remaining voices").on_click(
                                cx.listener(|this, _, window, cx| {
                                    let ids = this
                                        .waiting_voices()
                                        .iter()
                                        .map(|c| c.id.clone())
                                        .collect();
                                    this.set_voices_skipped(ids, true, cx);
                                    this.train_person = None;
                                    this.train_hint = None;
                                    this.person_name
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    cx.notify();
                                }),
                            ),
                        )
                    }),
            )
            .when_some(self.train_hint.clone(), |col, hint| {
                col.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(WARNING))
                        .child(hint),
                )
            })
    }
    /// Evaluates verified examples across recordings and proposes criteria; nothing changes
    /// until the user adopts them.
    pub(super) fn calibration_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div().flex().flex_col().gap_2().child(
            button(
                "train-calibrate",
                "Check how well my verified voices are recognized",
            )
            .self_start()
            .disabled(self.pending > 0)
            .on_click(cx.listener(|this, _, _, cx| {
                let service = this.speakers.clone();
                this.task(
                    "Evaluating verified voices across recordings",
                    async move { Ok(Update::Calibration(service.calibrate().await?)) },
                    cx,
                );
            })),
        );
        let Some(report) = &self.calibration else {
            return view;
        };
        view = view.child(caption(format!(
            "Checked {} voice examples of {} people: {} recordings to choose the settings and {} others to test them. {}",
            report.examples,
            report.profiles,
            report.tuning_recordings,
            report.held_out_recordings,
            report.reason
        )));
        if let Some(policy) = report.policy.clone() {
            let held = &report.held_out;
            view = view
                .child(caption(format!(
                    "On the test recordings, {} of {} known voices were recognized, and {} of {} checks matched the wrong person.",
                    held.correct_accepts,
                    held.known_trials,
                    held.false_accepts(),
                    held.trials
                )))
                .when(self.show_status_details, |view| {
                    view.child(caption(format!(
                        "Settings: similarity ≥ {:.2}, separation ≥ {:.2}, at least {} examples per person.",
                        policy.minimum_score, policy.minimum_margin, policy.minimum_examples
                    )))
                })
                .child(
                    button("train-adopt", "Use these criteria for suggestions")
                        .self_start()
                        .disabled(self.pending > 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let Some(report) = this.calibration.clone() else { return };
                            let Some(policy) = report.policy.clone() else { return };
                            let saved = SavedMatchingPolicy {
                                policy,
                                saved_at: now(),
                                provenance: report,
                            };
                            this.change_preferences(
                                "Saving calibrated criteria",
                                "Calibrated criteria saved; suggestions stay opt-in and are never confirmed names",
                                move |p| p.matching_policy = Some(saved.clone()),
                                cx,
                            );
                        })),
                );
        }
        view
    }
    fn train_named(&self, candidate: &EnrollmentCandidate) -> bool {
        self.train_done.iter().any(|i| {
            i.undone_at.is_none()
                && i.run_id == candidate.run_id
                && i.track_id == candidate.track_id
                && i.cluster == candidate.anonymous_speaker
        })
    }
    fn train_save(&mut self, candidate: EnrollmentCandidate, cx: &mut Context<Self>) {
        let heard = self.heard(&candidate);
        if heard.is_empty() {
            self.train_hint = Some("Play the clip first, so you know whose voice it is.".into());
            cx.notify();
            return;
        }
        let typed = self.person_name.read(cx).value().trim().to_string();
        let person = if typed.is_empty() {
            match &self.train_person {
                Some(id) => PersonChoice::Existing(id.clone()),
                None => {
                    self.train_hint = Some("Type a name for this voice first.".into());
                    cx.notify();
                    return;
                }
            }
        } else if let Some(known) = self
            .profiles
            .iter()
            .find(|p| p.name.trim().eq_ignore_ascii_case(&typed))
        {
            PersonChoice::Existing(known.id.clone())
        } else {
            PersonChoice::New(typed)
        };
        self.train_hint = None;
        let service = self.speakers.clone();
        let library = self.library.clone();
        let recording = candidate.recording_id.clone();
        let candidate_id = candidate.id.clone();
        self.task(
            "Naming this voice",
            async move {
                let identification = service
                    .identify_cluster(IdentifyClusterInput {
                        recording_id: candidate.recording_id,
                        track_id: candidate.track_id,
                        run_id: candidate.run_id,
                        cluster: candidate.anonymous_speaker,
                        person,
                        confirmed: true,
                        heard,
                    })
                    .await?;
                let profiles = service.profiles().await?;
                let detail = library.show(&recording).await.map(reader::Document::new)?;
                Ok(Update::Identified(Box::new((
                    candidate_id,
                    identification,
                    profiles,
                    detail,
                ))))
            },
            cx,
        );
    }
    pub(super) fn play_range(
        &mut self,
        track_id: String,
        start_ms: i64,
        end_ms: i64,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = &self.detail else { return };
        let recording_id = detail.recording.id.clone();
        let service = self.playback.clone();
        self.playback_task(
            "Playing representative speech",
            async move {
                service
                    .play(PlayAudioInput {
                        recording_id,
                        track_id,
                        start_ms,
                        duration_ms: (end_ms - start_ms).min(20_000),
                    })
                    .await?;
                Ok(Update::Message("Playback completed".into()))
            },
            cx,
        );
    }
}
fn people_key(id: &str) -> usize {
    id.bytes().fold(0usize, |hash, b| {
        hash.wrapping_mul(31).wrapping_add(b as usize)
    })
}
