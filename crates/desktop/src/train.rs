//! Speaker training session: play a voice, say who it is, move on. Naming a voice goes through
//! the shared `identify_cluster` use case and can be undone as one unit.
use super::*;

impl Workbench {
    pub(super) fn train_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div().flex().flex_col().gap_3();
        let Some(detail) = &self.detail else {
            return view.child(caption(
                "Open a recording in the Library, then come back to identify its voices.",
            ));
        };
        let title = detail.recording.title().to_owned();
        let queue: Vec<_> = self
            .candidates
            .iter()
            .filter(|c| !self.train_skipped.contains(&c.id) && !self.train_named(&c.id))
            .cloned()
            .collect();
        view = view
            .child(caption(format!(
                "{title} · {} voices waiting · {} named this session",
                queue.len(),
                self.train_done
                    .iter()
                    .filter(|i| i.undone_at.is_none())
                    .count()
            )))
            .child(
                quiet("train-prepare", "Find the voices in this recording")
                    .self_start()
                    .disabled(self.pending > 0)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let Some(detail) = &this.detail else { return };
                        let id = detail.recording.id.clone();
                        let service = this.speakers.clone();
                        this.task(
                            "Finding clean speech for each voice",
                            async move {
                                Ok(Update::Candidates(
                                    id.clone(),
                                    service.candidates(&id).await?,
                                ))
                            },
                            cx,
                        );
                    })),
            );
        if let Some(candidate) = queue.first().cloned() {
            let source = detail
                .tracks
                .iter()
                .find(|t| t.id == candidate.track_id)
                .map(|t| format!("{:?}", t.kind))
                .unwrap_or_default();
            view = view.child(
                div()
                    .p_4()
                    .rounded_lg()
                    .bg(rgb(PANEL))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(section(
                        format!("{} · {source}", anonymous_label(&candidate.anonymous_speaker)),
                        format!(
                            "{:.0} s of clean speech. Listen, then say who this is. Naming applies to every line this voice speaks alone and can be undone.",
                            candidate.speech_ms as f64 / 1000.
                        ),
                    ))
                    .child(self.train_actions(&candidate, cx)),
            );
        } else if self.candidates.is_empty() {
            view = view.child(caption(
                "No voices prepared yet. Find the voices, or run the meeting analysis first.",
            ));
        } else {
            view = view.child(caption("All prepared voices are handled for this session."));
        }
        view = view.child(self.imported_clip_view(cx));
        for (index, done) in self.train_done.iter().enumerate() {
            let id = done.id.clone();
            let name = self
                .profiles
                .iter()
                .find(|p| p.id == done.profile_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "removed person".into());
            let undone = done.undone_at.is_some();
            view = view.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(caption(format!(
                        "{} → {name}: {} lines named, {} ambiguous left alone, {} voice examples{}",
                        anonymous_label(&done.cluster),
                        done.segments.len(),
                        done.skipped.len(),
                        done.example_ids.len(),
                        if undone { " · undone" } else { "" }
                    )))
                    .child(
                        quiet(("train-undo", index), "Undo")
                            .disabled(undone || self.pending > 0)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let service = this.speakers.clone();
                                let id = id.clone();
                                this.task(
                                    "Undoing identification",
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
        view
    }
    /// Labels imported from another app are claims. They only suggest clips; each one is
    /// enrolled after the user listens and picks the person.
    fn imported_clip_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div().flex().flex_col().gap_2().child(
            quiet("train-imported", "Suggest clips from imported labels")
                .self_start()
                .on_click(cx.listener(|this, _, _, cx| {
                    let Some(detail) = &this.detail else { return };
                    let id = detail.recording.id.clone();
                    let service = this.speakers.clone();
                    this.task(
                        "Finding clips for imported labels",
                        async move {
                            Ok(Update::ImportedClips(
                                id.clone(),
                                service.imported_label_candidates(&id).await?,
                            ))
                        },
                        cx,
                    );
                })),
        );
        for (group_index, group) in self.imported_clips.iter().enumerate() {
            view = view.child(caption(format!(
                "Imported label \"{}\" (unverified claim) — pick the person above, listen, then confirm each clip",
                group.claim
            )));
            for (clip_index, clip) in group.clips.iter().enumerate() {
                let play = clip.clone();
                let enroll = clip.clone();
                let key = group_index * 100 + clip_index;
                view = view.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            quiet(
                                ("train-clip-play", key),
                                format!("Play {:.0}s", clip.local_start_ms as f64 / 1000.),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.play_range(
                                        play.track_id.clone(),
                                        play.local_start_ms,
                                        play.local_end_ms,
                                        cx,
                                    )
                                },
                            )),
                        )
                        .child(
                            button(("train-clip-enroll", key), "This is the selected person")
                                .disabled(self.train_person.is_none() || self.pending > 0)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let (Some(person), Some(detail)) =
                                        (this.train_person.clone(), &this.detail)
                                    else {
                                        return;
                                    };
                                    let input = EnrollSpeakerInput {
                                        profile_id: person.clone(),
                                        recording_id: detail.recording.id.clone(),
                                        track_id: enroll.track_id.clone(),
                                        start_ms: enroll.local_start_ms,
                                        end_ms: enroll.local_end_ms,
                                        confirmed: true,
                                    };
                                    let service = this.speakers.clone();
                                    this.task(
                                        "Extracting local verified embedding",
                                        async move {
                                            service.enroll(input).await?;
                                            let examples = service.examples(Some(&person)).await?;
                                            Ok(Update::Examples(person, examples))
                                        },
                                        cx,
                                    );
                                })),
                        ),
                );
            }
        }
        view
    }
    fn train_named(&self, candidate_id: &str) -> bool {
        self.train_done.iter().any(|i| {
            i.undone_at.is_none()
                && self
                    .train_candidates_named
                    .contains(&(candidate_id.to_owned(), i.id.clone()))
        })
    }
    fn train_actions(&self, candidate: &EnrollmentCandidate, cx: &mut Context<Self>) -> Div {
        let mut people = div().flex().flex_wrap().gap_2();
        for person in &self.profiles {
            let id = person.id.clone();
            people = people.child(
                choice(
                    ("train-person", people_key(&person.id)),
                    person.name.clone(),
                    self.train_person.as_ref() == Some(&person.id),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.train_person = Some(id.clone());
                    cx.notify();
                })),
            );
        }
        let play = candidate.clone();
        let skip = candidate.id.clone();
        let save = candidate.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                button("train-play", "Play representative speech")
                    .self_start()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.play_range(
                            play.track_id.clone(),
                            play.local_start_ms,
                            play.local_end_ms,
                            cx,
                        )
                    })),
            )
            .child(caption("Who is this?"))
            .child(people)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(caption("or a new person"))
                    .child(field(&self.person_name).w(px(220.))),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        primary("train-save", "Save: I listened and this is them")
                            .disabled(self.pending > 0)
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.train_save(save.clone(), cx)
                                }),
                            ),
                    )
                    .child(
                        quiet("train-skip", "Unknown / skip for now").on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.train_skipped.push(skip.clone());
                                this.train_person = None;
                                cx.notify();
                            },
                        )),
                    ),
            )
    }
    fn train_save(&mut self, candidate: EnrollmentCandidate, cx: &mut Context<Self>) {
        let typed = self.person_name.read(cx).value().trim().to_string();
        let person = match (&self.train_person, typed.is_empty()) {
            (Some(id), _) => PersonChoice::Existing(id.clone()),
            (None, false) => PersonChoice::New(typed),
            (None, true) => {
                self.status = "Choose a person or type a new name first".into();
                cx.notify();
                return;
            }
        };
        let service = self.speakers.clone();
        let library = self.library.clone();
        let recording = candidate.recording_id.clone();
        let candidate_id = candidate.id.clone();
        self.task(
            "Naming this voice and saving verified examples",
            async move {
                let identification = service
                    .identify_cluster(IdentifyClusterInput {
                        recording_id: candidate.recording_id,
                        track_id: candidate.track_id,
                        run_id: candidate.run_id,
                        cluster: candidate.anonymous_speaker,
                        person,
                        confirmed: true,
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
