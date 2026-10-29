use super::*;

#[derive(Clone)]
pub enum ConfirmAction {
    DeleteProfile(String),
    RemoveModel(String),
    RemoveExample { id: String, profile: String },
}

impl Workbench {
    pub(super) fn load_meeting_plans(&mut self, cx: &mut Context<Self>) {
        let service = self.meeting.clone();
        self.task(
            "Checking unfinished meeting transcripts",
            async move { Ok(Update::MeetingPlans(service.pending().await?)) },
            cx,
        );
    }
    pub(super) fn finalize_meeting(&mut self, id: String, cx: &mut Context<Self>) {
        let service = self.meeting.clone();
        let library = self.library.clone();
        self.task(
            "Finishing transcription from saved source audio",
            async move {
                let warning = service
                    .finalize(parakeet_application::meeting::FinalizeMeetingInput {
                        id: id.to_string(),
                        model: None,
                    })
                    .await
                    .err()
                    .map(|e| e.to_string());
                let detail = library
                    .show(&id)
                    .await
                    .map(reader::Document::new)
                    .map_err(|e| e.to_string());
                Ok(Update::MeetingFinalized(detail, warning))
            },
            cx,
        );
    }
    pub(super) fn navigate(&mut self, area: Area, cx: &mut Context<Self>) {
        self.area = area;
        match area {
            Area::Storage => self.measure_storage(cx),
            Area::Jobs => {
                let service = self.recovery.clone();
                self.task(
                    "Inspecting pending jobs",
                    async move { Ok(Update::Jobs(service.pending().await?)) },
                    cx,
                );
            }
            Area::Capture => {
                let capture = self.audio_inspection.clone();
                self.task(
                    "Enumerating input devices",
                    async move { Ok(Update::Devices(capture.devices().await?)) },
                    cx,
                );
            }
            Area::Speakers => {
                self.refresh_profiles(cx);
                if let Some(d) = &self.detail {
                    let id = d.recording.id.clone();
                    self.load_voices(id, cx);
                }
            }
            Area::Dictation => {
                self.refresh_permissions(cx);
                let capture = self.audio_inspection.clone();
                self.task(
                    "Checking selected microphone",
                    async move { Ok(Update::Devices(capture.devices().await?)) },
                    cx,
                );
                let service = self.dictation_history.clone();
                self.task(
                    "Reading retained dictations",
                    async move { Ok(Update::Dictations(service.list().await?)) },
                    cx,
                );
                let settings = self.settings.clone();
                self.task(
                    "Reading shortcut configuration",
                    async move {
                        Ok(Update::Settings(
                            settings.shortcuts().await?,
                            settings.prompts().await?,
                        ))
                    },
                    cx,
                );
            }
            Area::Models => {
                let service = self.models.clone();
                self.task(
                    "Reading local model registry",
                    async move { Ok(Update::Models(service.list().await?)) },
                    cx,
                );
            }
            Area::Privacy => {
                self.refresh_permissions(cx);
                let service = self.settings.clone();
                self.task(
                    "Reading settings",
                    async move {
                        Ok(Update::Settings(
                            service.shortcuts().await?,
                            service.prompts().await?,
                        ))
                    },
                    cx,
                );
            }
            _ => {}
        }
        cx.notify();
    }
    pub(super) fn sidebar(&self, cx: &mut Context<Self>) -> Div {
        let mut nav = div()
            .w(px(196.))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(SIDEBAR))
            .border_r_1()
            .border_color(rgb(BORDER))
            .px_3()
            .py_4()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .mb_5()
                    .child(
                        div()
                            .size(px(32.))
                            .flex_shrink_0()
                            .rounded_lg()
                            .bg(rgb(SELECTED))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(ACCENT))
                            .child(icon("wave").size(px(22.)).text_color(rgb(ACCENT))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Parakeet Local"),
                            )
                            .child(caption("Your speech workspace")),
                    ),
            )
            .child(caption("WORKSPACE").px_2().mb_1());
        for (i, (area, label, glyph, shortcut)) in [
            (Area::Library, "Library", "library", "⌘1"),
            (Area::Capture, "Record a meeting", "mic", "⌘2"),
            (Area::Dictation, "Dictation", "keyboard", "⌘3"),
            (Area::Speakers, "People", "people", "⌘4"),
            (Area::Models, "Local models", "models", ""),
            (Area::Jobs, "Background jobs", "jobs", ""),
            (Area::Storage, "Storage", "models", ""),
            (Area::Privacy, "Permissions", "shield", ""),
        ]
        .into_iter()
        .enumerate()
        {
            if i == 4 {
                nav = nav.child(caption("SETTINGS").px_2().mt_5().mb_1());
            }
            let active = self.area == area;
            nav = nav.child(
                choice(("nav", i), "", active)
                    .accessibility_label(label)
                    .w_full()
                    .h(px(34.))
                    .px_2()
                    .justify_start()
                    .gap_2()
                    .text_size(px(12.))
                    .child(icon(glyph).text_color(rgb(if active { ACCENT } else { MUTED })))
                    .child(div().flex_1().child(label))
                    .child(caption(shortcut).text_size(px(10.)))
                    .on_click(cx.listener(move |this, _, _, cx| this.navigate(area, cx))),
            );
        }
        nav.child(
            div()
                .mt_auto()
                .pt_5()
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .child(div().size(px(6.)).rounded_full().bg(rgb(SUCCESS)))
                .child(caption("Local audio processing")),
        )
    }
    pub(super) fn status_bar(&self, cx: &mut Context<Self>) -> Div {
        let working = self.pending > 0;
        let color = if self.last_error.is_some() {
            DANGER
        } else if working {
            WARNING
        } else {
            SUCCESS
        };
        let message = if working && self.status == "Ready" {
            "Working in the background".to_string()
        } else {
            self.status.clone()
        };
        let mut status = div()
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SIDEBAR))
            .px_4()
            .py_1()
            .flex()
            .flex_col()
            .gap_1()
            .when(self.playback_abort.is_some(), |status| {
                status.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .py_1()
                        .child(icon("wave").text_color(rgb(ACCENT)))
                        .child(caption(self.playback_label.clone()).flex_1())
                        .child(
                            button("stop-playback", "Stop playback")
                                .on_click(cx.listener(|this, _, _, cx| this.stop_playback(cx))),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(6.)).rounded_full().bg(rgb(color)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .truncate()
                            .child(message),
                    )
                    .when(working, |row| {
                        row.child(pill(format!("{} active", self.pending), WARNING))
                    })
                    .child(
                        quiet(
                            "activity-details",
                            if self.show_status_details {
                                "Hide details"
                            } else {
                                "Details"
                            },
                        )
                        .py_0p5()
                        .min_h(px(22.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_status_details = !this.show_status_details;
                            cx.notify();
                        })),
                    ),
            );
        if self.show_status_details {
            status = status.child(
                div()
                    .id("activity-detail-body")
                    .max_h(px(140.))
                    .overflow_y_scroll()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(caption(format!(
                        "{} background tasks · render peak {:.2} ms · heartbeat max {:.0} ms",
                        self.pending, self.render_peak_ms, self.heartbeat_max_ms
                    )))
                    .child(
                        self.last_error
                            .clone()
                            .unwrap_or_else(|| self.status.clone()),
                    ),
            );
        }
        status
    }
    pub(super) fn error_banner(&self, cx: &mut Context<Self>) -> Div {
        let error = self.last_error.as_deref().unwrap_or_default();
        let summary = error.chars().take(180).collect::<String>();
        div()
            .flex_shrink_0()
            .bg(rgb(0x34262c))
            .border_1()
            .border_color(rgb(0x67434a))
            .rounded_lg()
            .mx_5()
            .mt_3()
            .max_w(px(900.))
            .px_4()
            .py_3()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_color(rgb(DANGER))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("The action couldn’t be completed"),
                    )
                    .child(caption(summary)),
            )
            .child(
                quiet("error-details", "Details")
                    .self_center()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_status_details = true;
                        cx.notify();
                    })),
            )
            .child(
                quiet("dismiss-error", "Dismiss")
                    .self_center()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.last_error = None;
                        this.show_status_details = false;
                        cx.notify();
                    })),
            )
    }
    pub(super) fn confirmation(&self, cx: &mut Context<Self>) -> Div {
        let label = match &self.confirm_action {
            Some(ConfirmAction::DeleteProfile(id)) => format!(
                "Delete {} and their verified voice examples? This cannot be undone.",
                self.profiles
                    .iter()
                    .find(|p| &p.id == id)
                    .map(|p| p.name.as_str())
                    .unwrap_or("this person")
            ),
            Some(ConfirmAction::RemoveExample { .. }) => "Remove this verified voice example? Future recognition will no longer use it. This cannot be undone.".into(),
            Some(ConfirmAction::RemoveModel(id)) => {
                format!("Remove the downloaded files for {id}? You can download them again later.")
            }
            None => String::new(),
        };
        div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .bg(rgb(PANEL))
            .border_b_1()
            .border_color(rgb(BORDER))
            .px_5()
            .py_3()
            .flex()
            .items_center()
            .gap_3()
            .child(div().flex_1().min_w_0().whitespace_normal().child(label))
            .child(
                quiet("cancel-delete", "Cancel")
                    .flex_shrink_0()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_action = None;
                        cx.notify();
                    })),
            )
            .child(
                danger("confirm-delete", "Confirm removal")
                    .flex_shrink_0()
                    .disabled(self.pending > 0)
                    .on_click(cx.listener(|this, _, _, cx| {
                        match this.confirm_action.take() {
                            Some(ConfirmAction::DeleteProfile(id)) => {
                                let service = this.speakers.clone();
                                this.task(
                                    "Deleting local voice profile",
                                    async move {
                                        service.delete(&id).await?;
                                        Ok(Update::Profiles(service.profiles().await?))
                                    },
                                    cx,
                                );
                            }
                            Some(ConfirmAction::RemoveExample { id, profile }) => {
                                let service = this.speakers.clone();
                                this.task(
                                    "Removing voice example",
                                    async move {
                                        service.remove_example(&id).await?;
                                        Ok(Update::Examples(
                                            profile.clone(),
                                            service.examples(Some(&profile)).await?,
                                        ))
                                    },
                                    cx,
                                );
                            }
                            Some(ConfirmAction::RemoveModel(id)) => {
                                let service = this.models.clone();
                                this.task(
                                    "Removing downloaded model files",
                                    async move {
                                        service.remove(&id).await?;
                                        Ok(Update::Models(service.list().await?))
                                    },
                                    cx,
                                );
                            }
                            None => {}
                        }
                        cx.notify();
                    })),
            )
    }
    pub(super) fn receive_detail(&mut self, d: reader::Document, cx: &mut Context<Self>) {
        if self.requested_recording.as_deref() != Some(d.recording.id.as_str()) {
            return;
        }
        self.selected_track = self
            .selected_track
            .clone()
            .filter(|id| d.tracks.iter().any(|t| t.id == *id))
            .or_else(|| {
                d.tracks
                    .iter()
                    .find(|t| t.available && t.kind != TrackKind::Playback)
                    .map(|t| t.id.clone())
            });
        if let Some(row) = self.records.iter_mut().find(|r| r.id == d.recording.id) {
            *row = d.recording.clone();
        }
        self.reader_matches = d.matching(&self.transcript_query);
        self.segment_page = self
            .segment_page
            .min(self.reader_matches.len().saturating_sub(1) / 40);
        let id = d.recording.id.clone();
        // Voices only change with the analysis runs; a text or title save must not reload them.
        let key = serde_json::json!([
            id,
            d.runs
                .iter()
                .map(|r| (&r.id, &r.status))
                .collect::<Vec<_>>(),
            d.analysis
                .as_ref()
                .ok()
                .and_then(|p| p.as_ref())
                .map(|p| &p.status),
        ])
        .to_string();
        self.detail = Some(d);
        if self.voices_key.as_ref() != Some(&key) {
            self.voices_key = Some(key);
            self.load_voices(id, cx);
        }
        self.reset_reader_rows(false);
    }
    pub(super) fn subscribe_search(&mut self, cx: &mut Context<Self>) {
        cx.subscribe(&self.search, |this, _, event, cx| match event {
            InputEvent::PressEnter { .. } => this.reload(cx),
            InputEvent::Change => {
                this.search_generation += 1;
                let generation = this.search_generation;
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(220))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if generation == this.search_generation {
                            this.reload(cx);
                        }
                    });
                })
                .detach();
            }
            _ => {}
        })
        .detach();
        cx.subscribe(&self.transcript_find, |this, _, event, cx| {
            if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                this.transcript_query = this.transcript_find.read(cx).value().to_string();
                this.refresh_reader_matches();
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&self.edit_text, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) && this.editing_segment.is_some() {
                this.reset_reader_rows(false);
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&self.title, |this, _, event, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) && this.editing_title {
                this.save_title(cx);
            }
        })
        .detach();
    }
}
