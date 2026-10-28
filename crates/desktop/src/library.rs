use super::*;
use parakeet_domain::corrections::{self, SpeakerOverride};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LibraryFilter {
    All,
    Meetings,
    Media,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RecordingTab {
    Transcript,
    Audio,
    Processing,
}
fn recording_kind(kind: &str) -> &'static str {
    match kind {
        "meeting" => "Meeting",
        "youtube" => "YouTube",
        "file" => "Audio file",
        _ => "Recording",
    }
}
fn track_name(kind: &TrackKind) -> &'static str {
    match kind {
        TrackKind::Microphone => "Microphone",
        TrackKind::System => "System audio",
        TrackKind::Playback => "Playback mix",
        TrackKind::File => "Audio file",
    }
}
pub(super) fn time_label(ms: Option<i64>) -> String {
    match ms {
        Some(ms) if ms >= 0 => {
            let sec = ms / 1000;
            if sec >= 3600 {
                format!("{}:{:02}:{:02}", sec / 3600, sec / 60 % 60, sec % 60)
            } else {
                format!("{}:{:02}", sec / 60, sec % 60)
            }
        }
        _ => "Untimed".into(),
    }
}
fn date_label(raw: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%b %-d, %Y")
                .to_string()
        })
        .unwrap_or_else(|_| raw.chars().take(10).collect())
}
fn voice_color(id: &str) -> u32 {
    [0x91b6ff, 0xc4a7ec, 0x90d0b1, 0xe5bc7c, 0xefabab, 0x92cdd4][id
        .bytes()
        .fold(0usize, |sum, b| sum.wrapping_add(b as usize))
        % 6]
}

impl Workbench {
    pub(super) fn choose_audio(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Import audio or recording folder".into()),
        });
        cx.spawn(async move |this, cx| match prompt.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| this.import_audio(path, cx));
                }
            }
            Ok(Ok(None)) => {}
            other => {
                let _ = this.update(cx, |this, cx| {
                    this.last_error = Some(format!("Couldn’t open file picker: {other:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }
    fn import_audio(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let library = self.library.clone();
        self.task(
            "Importing audio into the library",
            async move {
                Ok(Update::Imported(
                    library
                        .import_path(ImportRecordingsInput {
                            path,
                            dry_run: false,
                        })
                        .await?,
                ))
            },
            cx,
        );
    }
    pub(super) fn save_title(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else { return };
        let id = detail.recording.id.clone();
        let title = self.title.read(cx).value().to_string();
        let library = self.library.clone();
        self.task(
            "Saving recording title",
            async move {
                library
                    .rename(RenameRecordingInput {
                        id: id.clone(),
                        title,
                    })
                    .await?;
                Ok(Update::title_saved(library.show(&id).await?))
            },
            cx,
        );
    }
    pub(super) fn save_segment_text(&mut self, cx: &mut Context<Self>) {
        let Some(segment_id) = self.editing_segment.clone() else {
            return;
        };
        let text = self.edit_text.read(cx).value().to_string();
        let service = self.speakers.clone();
        self.task(
            "Saving transcript correction",
            async move {
                service
                    .correct(CorrectSegmentInput {
                        segment_id: segment_id.clone(),
                        profile_id: None,
                        text: Some(text),
                        action: "text".into(),
                    })
                    .await?;
                Ok(Update::SegmentSaved(segment_id))
            },
            cx,
        );
    }
    fn assign_segment(&mut self, id: String, profile: Option<String>, cx: &mut Context<Self>) {
        let service = self.speakers.clone();
        let action = if profile.is_some() {
            "assign"
        } else {
            "unknown"
        };
        self.task(
            "Saving speaker correction",
            async move {
                service
                    .correct(CorrectSegmentInput {
                        segment_id: id,
                        profile_id: profile,
                        text: None,
                        action: action.into(),
                    })
                    .await?;
                Ok(Update::RefreshSelected)
            },
            cx,
        );
    }
    fn undo_segment(&mut self, id: String, cx: &mut Context<Self>) {
        let service = self.speakers.clone();
        self.task(
            "Undoing latest correction",
            async move {
                service.undo(&id).await?;
                Ok(Update::SegmentSaved(id))
            },
            cx,
        );
    }
    fn play_speech(&mut self, recording: String, segment: String, cx: &mut Context<Self>) {
        let service = self.playback.clone();
        self.playback_task(
            "Playing source audio · up to 30 seconds",
            async move {
                service
                    .play_segment(PlaySegmentInput {
                        recording_id: recording,
                        segment_id: segment,
                    })
                    .await?;
                Ok(Update::Message("Playback finished".into()))
            },
            cx,
        );
    }
    pub(super) fn library_view(&self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = div()
            .h(px(64.))
            .flex_shrink_0()
            .px_5()
            .border_b_1()
            .border_color(rgb(BORDER))
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_size(px(21.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Library"),
                    )
                    .child(caption("Recordings, meetings, and imported media")),
            )
            .child(
                quiet("show-import-path", "From path…").on_click(cx.listener(|this, _, _, cx| {
                    this.show_import_path = !this.show_import_path;
                    this.show_media_import = false;
                    cx.notify();
                })),
            )
            .child(
                button("show-media-import", "YouTube URL").on_click(cx.listener(
                    |this, _, window, cx| {
                        this.show_media_import = !this.show_media_import;
                        this.show_import_path = false;
                        if this.show_media_import {
                            this.media_url.focus_handle(cx).focus(window, cx);
                        }
                        cx.notify();
                    },
                )),
            )
            .child(
                primary("import-file", "Import audio…")
                    .on_click(cx.listener(|this, _, _, cx| this.choose_audio(cx))),
            );
        let mut view = div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(header);
        if self.show_import_path {
            view = view.child(
                div()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(PANEL))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(field(&self.import_path)))
                    .child(
                        primary("import-from-path", "Import path").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.import_audio(
                                    PathBuf::from(this.import_path.read(cx).value().to_string()),
                                    cx,
                                )
                            },
                        )),
                    )
                    .child(quiet("close-import-path", "Cancel").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.show_import_path = false;
                            cx.notify();
                        },
                    ))),
            );
        }
        if self.show_media_import {
            view=view.child(div().px_5().py_3().border_b_1().border_color(rgb(BORDER)).bg(rgb(PANEL)).flex().flex_col().gap_2()
                .child(caption("Download the audio and transcribe it with your selected local model."))
                .child(div().flex().items_center().gap_2().child(div().flex_1().min_w_0().child(field(&self.media_url)))
                    .child(primary("import-youtube","Download & transcribe").disabled(self.pending>0).on_click(cx.listener(|this,_,_,cx|{
                        let service=this.media.clone();let library=this.library.clone();let url=this.media_url.read(cx).value().to_string();let generation=this.search_generation;let query=this.search.read(cx).value().to_string();
                        this.task("Downloading and transcribing media",async move{let result=service.transcribe(TranscribeMediaInput{url,model:"selected-asr".into()}).await?;if let Some(error)=result.transcription_failure{anyhow::bail!("Audio was saved as {}. Transcription failed: {error}",result.recording.id);}Ok(Update::LibrarySearch(generation,library.list(if query.is_empty(){None}else{Some(&query)}).await?))},cx);
                    })))
                    .child(quiet("close-media-import","Cancel").on_click(cx.listener(|this,_,_,cx|{this.show_media_import=false;cx.notify();})))));
        }
        view.child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(self.recording_list(cx))
                .child(self.recording_reader(cx)),
        )
    }
    fn recording_list(&self, cx: &mut Context<Self>) -> Div {
        let matching = |r: &&Recording| match self.library_filter {
            LibraryFilter::All => true,
            LibraryFilter::Meetings => r.source_kind == "meeting",
            LibraryFilter::Media => r.source_kind != "meeting",
        };
        let visible = self.records.iter().filter(matching).collect::<Vec<_>>();
        let mut filters = div().flex().gap_1();
        for (i, (filter, label)) in [
            (LibraryFilter::All, "All"),
            (LibraryFilter::Meetings, "Meetings"),
            (LibraryFilter::Media, "Media"),
        ]
        .into_iter()
        .enumerate()
        {
            filters = filters.child(
                choice(("library-filter", i), label, self.library_filter == filter)
                    .px_2()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.library_filter = filter;
                        cx.notify();
                    })),
            );
        }
        let mut rows = div()
            .id("library-recordings")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .pb_2()
            .flex()
            .flex_col()
            .gap_1();
        for recording in &visible {
            let id = recording.id.clone();
            let active = self.requested_recording.as_deref() == Some(id.as_str());
            let glyph = match recording.source_kind.as_str() {
                "meeting" => "mic",
                "youtube" => "link",
                _ => "file",
            };
            rows = rows.child(
                choice(format!("recording-{id}"), "", active)
                    .accessibility_label(recording.title().to_owned())
                    .w_full()
                    .py_3()
                    .px_3()
                    .justify_start()
                    .gap_2()
                    .child(icon(glyph).text_color(rgb(if active { ACCENT } else { MUTED })))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .w_full()
                                    .text_size(px(13.))
                                    .font_weight(if active {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .truncate()
                                    .child(recording.title().to_owned()),
                            )
                            .child(caption(format!(
                                "{} · {}",
                                date_label(&recording.created_at),
                                if let Some(duration) = recording.duration_ms {
                                    time_label(Some(duration))
                                } else {
                                    recording_kind(&recording.source_kind).into()
                                }
                            ))),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open(id.clone(), window, cx)),
                    ),
            );
        }
        if visible.is_empty() {
            rows = rows.child(
                div()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(caption(if self.records.is_empty() {
                        "No recordings found. Import audio or try another search."
                    } else {
                        "No recordings in this category."
                    }))
                    .child(
                        quiet("clear-library-filters", "Show all recordings").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.library_filter = LibraryFilter::All;
                                this.search
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                this.reload(cx);
                            }),
                        ),
                    ),
            )
        }
        div()
            .w(px(266.))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(rgb(BORDER))
            .bg(rgb(0x1d2026))
            .flex()
            .flex_col()
            .child(
                div()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(icon("search").text_color(rgb(MUTED)))
                            .child(field(&self.search).flex_1())
                            .child(caption("⌘⇧F")),
                    )
                    .child(filters),
            )
            .child(rows)
            .child(
                caption(format!(
                    "{} {}",
                    visible.len(),
                    if visible.len() == 1 {
                        "recording"
                    } else {
                        "recordings"
                    }
                ))
                .px_4()
                .py_2()
                .border_t_1()
                .border_color(rgb(BORDER)),
            )
    }
    fn recording_reader(&self, cx: &mut Context<Self>) -> Div {
        let mut reader = div().flex_1().min_w_0().h_full().flex().flex_col();
        let Some(detail) = &self.detail else {
            let loading = self.requested_recording.is_some() && self.pending > 0;
            return reader.items_center().justify_center().gap_3().p_8()
                .child(div().size(px(58.)).rounded_xl().bg(rgb(PANEL)).text_color(rgb(ACCENT)).flex().items_center().justify_center().child(icon("wave").size(px(30.))))
                .child(div().text_size(px(21.)).font_weight(FontWeight::SEMIBOLD).child(if loading{"Opening recording…"}else{"A place for every conversation"}))
                .child(div().max_w(px(340.)).text_center().text_color(rgb(MUTED)).child(if loading{"Loading the saved transcript and source details."}else{"Select a recording to read its transcript, listen to the original audio, or review its speakers."}))
                .when(!loading,|view|view.child(primary("empty-import","Import audio…").on_click(cx.listener(|this,_,_,cx|this.choose_audio(cx)))))
                .child(caption("⌘O  Import audio     ⌘F  Find words"));
        };
        let mut heading = div()
            .px_5()
            .pt_5()
            .pb_3()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2();
        if self.editing_title {
            heading = heading.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(field(&self.title).flex_1())
                    .child(
                        primary("save-title", "Save title")
                            .disabled(self.pending > 0)
                            .on_click(cx.listener(|this, _, _, cx| this.save_title(cx))),
                    )
                    .child(quiet("cancel-title", "Cancel").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.editing_title = false;
                            cx.notify();
                        },
                    ))),
            );
        } else {
            let title = detail.recording.title().to_owned();
            let copy = title.clone();
            heading = heading.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(quiet("rename-recording", "Rename").on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.editing_title = true;
                            this.title
                                .update(cx, |input, cx| input.set_value(copy.clone(), window, cx));
                            this.title.focus_handle(cx).focus(window, cx);
                            cx.notify();
                        },
                    )))
                    .child(
                        quiet("copy-transcript", "Copy transcript").on_click(cx.listener(
                            |this, _, _, cx| {
                                if let Some(detail) = &this.detail {
                                    let id = detail.recording.id.clone();
                                    let library = this.library.clone();
                                    this.task(
                                        "Preparing corrected transcript",
                                        async move {
                                            Ok(Update::CopyText(library.export_text(&id).await?))
                                        },
                                        cx,
                                    );
                                }
                            },
                        )),
                    ),
            );
        }
        heading = heading.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(pill(recording_kind(&detail.recording.source_kind), ACCENT))
                .child(caption(date_label(&detail.recording.created_at)))
                .when(detail.recording.duration_ms.is_some(), |row| {
                    row.child(caption(time_label(detail.recording.duration_ms)))
                })
                .when(!detail.tracks.iter().any(|t| t.available), |row| {
                    row.child(pill("Source unavailable", WARNING))
                }),
        );
        let mut tabs = div()
            .px_5()
            .pb_2()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(rgb(BORDER));
        for (i, (tab, label)) in [
            (RecordingTab::Transcript, "Transcript"),
            (RecordingTab::Audio, "Audio & sources"),
            (RecordingTab::Processing, "Processing"),
        ]
        .into_iter()
        .enumerate()
        {
            tabs = tabs.child(
                choice(("recording-tab", i), label, self.recording_tab == tab).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.recording_tab = tab;
                        this.reset_reader_rows(true);
                        cx.notify();
                    }),
                ),
            );
        }
        reader = reader.child(heading).child(tabs);
        match self.recording_tab {
            RecordingTab::Transcript => reader.child(self.transcript_view(detail, cx)),
            RecordingTab::Audio => reader.child(self.audio_view(detail, cx)),
            RecordingTab::Processing => reader.child(self.processing_view(detail, cx)),
        }
    }
    fn transcript_view(&self, detail: &reader::Document, cx: &mut Context<Self>) -> Div {
        let segments = &self.reader_matches;
        let page = self.segment_page.min(segments.len().saturating_sub(1) / 40);
        let begin = page * 40;
        let end = (begin + 40).min(segments.len());
        let toolbar = div()
            .flex_shrink_0()
            .px_5()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .child(
                caption(if segments.is_empty() {
                    "No matching speech".into()
                } else {
                    format!(
                        "{}–{} of {} {} · All sources",
                        begin + 1,
                        end,
                        segments.len(),
                        if segments.len() == 1 {
                            "segment"
                        } else {
                            "segments"
                        }
                    )
                })
                .flex_1(),
            )
            .child(
                quiet("find-transcript", "Find").on_click(cx.listener(|this, _, window, cx| {
                    this.show_transcript_find = !this.show_transcript_find;
                    if this.show_transcript_find {
                        this.transcript_find.focus_handle(cx).focus(window, cx);
                    } else {
                        this.transcript_query.clear();
                        this.refresh_reader_matches();
                        this.transcript_find
                            .update(cx, |s, cx| s.set_value("", window, cx));
                    }
                    cx.notify();
                })),
            )
            .when(!detail.word_attributions.is_empty(), |row| {
                row.child(
                    quiet(
                        "word-evidence",
                        if self.show_word_evidence {
                            "Hide word evidence"
                        } else {
                            "Word evidence"
                        },
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_word_evidence = !this.show_word_evidence;
                        this.reset_reader_rows(false);
                        cx.notify();
                    })),
                )
            })
            .child(
                quiet("previous-segments", "Previous")
                    .disabled(page == 0)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.segment_page = this.segment_page.saturating_sub(1);
                        this.reset_reader_rows(true);
                        this.editing_segment = None;
                        cx.notify();
                    })),
            )
            .child(
                quiet("next-segments", "Next")
                    .disabled(end >= segments.len())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.segment_page += 1;
                        this.reset_reader_rows(true);
                        this.editing_segment = None;
                        cx.notify();
                    })),
            );
        let mut toolbar = div().flex_shrink_0().flex().flex_col().child(toolbar);
        if self.show_transcript_find {
            toolbar = toolbar.child(
                div()
                    .px_5()
                    .pb_2()
                    .child(field(&self.transcript_find).w_full()),
            );
        }
        let indices = std::rc::Rc::clone(segments);
        let weak = cx.entity().downgrade();
        let recording_id = detail.recording.id.clone();
        let content = list(self.reader_list.clone(), move |i, _, cx| {
            weak.update(cx, |this, cx| {
                let Some(detail) = this
                    .detail
                    .as_ref()
                    .filter(|d| d.recording.id == recording_id)
                else {
                    return div().into_any_element();
                };
                let Some(index) = indices.get(begin + i) else {
                    return div().into_any_element();
                };
                let Some(segment) = detail.segments.get(*index) else {
                    return div().into_any_element();
                };
                div()
                    .w_full()
                    .px_5()
                    .child(this.transcript_row(detail, segment, *index, cx))
                    .into_any_element()
            })
            .unwrap_or_else(|_| div().into_any_element())
        })
        .flex_1()
        .min_h_0()
        .pb_5();
        let mut rows = div()
            .id("transcript-reader")
            .overflow_hidden()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col();
        if self.show_word_evidence {
            rows = rows.child(caption("Original estimated word timing is separate from your corrections. Unknown words remain unattributed.").px_5().py_3());
        }
        if !segments.is_empty() {
            rows = rows.child(content);
        }
        if segments.is_empty() {
            rows = rows.child(
                div()
                    .py_8()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .child(icon("search").size(px(28.)).text_color(rgb(MUTED)))
                    .child(
                        div()
                            .text_size(px(16.))
                            .child(if detail.segments.is_empty() {
                                "No transcript yet"
                            } else {
                                "No matching words"
                            }),
                    )
                    .child(caption(if detail.segments.is_empty() {
                        "Open Audio & sources to transcribe a saved source locally."
                    } else {
                        "Try another search. Your saved transcript is unchanged."
                    }))
                    .when(detail.segments.is_empty(), |row| {
                        row.child(button("open-audio-tab", "Audio & sources").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.recording_tab = RecordingTab::Audio;
                                cx.notify();
                            }),
                        ))
                    }),
            );
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(rows)
    }
    pub(super) fn transcript_row(
        &self,
        detail: &reader::Document,
        segment: &TranscriptSegment,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Div {
        let correction = &detail.row_corrections[index];
        let words = correction
            .text
            .clone()
            .unwrap_or_else(|| segment.text.clone());
        let attribution = detail.row_attributions[index].map(|i| &detail.attributions[i]);
        let (label, confirmed, color) = match &correction.speaker {
            Some(SpeakerOverride::Profile(id)) => {
                match self.profiles.iter().find(|p| &p.id == id) {
                    Some(profile) => (profile.name.clone(), true, voice_color(id)),
                    None => ("Unknown · profile removed".into(), false, WARNING),
                }
            }
            Some(SpeakerOverride::Unknown) => ("Unknown · reviewed".into(), false, WARNING),
            None => match attribution {
                Some(a) if a.state == "anonymous_model_attribution" => (
                    a.anonymous_speakers
                        .iter()
                        .map(|id| anonymous_label(id))
                        .collect::<Vec<_>>()
                        .join(", "),
                    false,
                    voice_color(
                        a.anonymous_speakers
                            .first()
                            .map(String::as_str)
                            .unwrap_or("unknown"),
                    ),
                ),
                _ => match (
                    imported_claim(segment),
                    suggested_name(self, detail, segment),
                ) {
                    (Some(claim), _) => (claim.to_owned(), false, MUTED),
                    (None, Some(name)) => (name, false, ACCENT),
                    (None, None) => ("Unknown speaker".into(), false, WARNING),
                },
            },
        };
        let imported = correction.speaker.is_none() && imported_claim(segment).is_some();
        let source_label = detail.row_attributions[index]
            .and_then(|i| detail.source_attributions.get(i))
            .map(|a| match a.category.as_str() {
                "me" => ("Me · microphone".to_owned(), MUTED),
                "them" => ("Them · system audio".to_owned(), MUTED),
                _ => (
                    format!(
                        "Unknown source · {}",
                        a.reasons.join(", ").replace('_', " ")
                    ),
                    WARNING,
                ),
            });
        let suggested = correction.speaker.is_none()
            && !imported
            && suggested_name(self, detail, segment).is_some();
        let source = segment
            .track_id
            .as_ref()
            .and_then(|id| detail.tracks.iter().find(|t| &t.id == id))
            .map(|t| track_name(&t.kind))
            .unwrap_or("Unresolved source");
        let segment_id = segment.id.clone();
        let play_id = segment.id.clone();
        let recording = detail.recording.id.clone();
        let copy = words.clone();
        let edit_text = words.clone();
        let editing = self.editing_segment.as_deref() == Some(segment.id.as_str());
        let mut body = div()
            .flex_1()
            .min_w_0()
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
                        div()
                            .id(format!("speaker-label-{}", segment.id))
                            .role(Role::Label)
                            .aria_label(label.clone())
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(color))
                            .child(label.clone()),
                    )
                    .when(confirmed, |row| row.child(pill("Confirmed", SUCCESS)))
                    .when(imported, |row| {
                        row.child(pill("Imported · unverified", MUTED))
                    })
                    .when(suggested, |row| {
                        row.child(pill("Suggested · not confirmed", ACCENT))
                    })
                    .when_some(source_label, |row, (text, color)| {
                        row.child(pill(text, color))
                    })
                    .child(caption(source))
                    .child(
                        quiet(
                            format!("play-speech-{}", segment.id),
                            time_label(segment.start_ms),
                        )
                        .accessibility_label(format!(
                            "Play speech at {}",
                            time_label(segment.start_ms)
                        ))
                        .text_size(px(11.))
                        .px_1()
                        .py_0p5()
                        .min_h(px(22.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.play_speech(recording.clone(), play_id.clone(), cx)
                        })),
                    )
                    .child(div().flex_1())
                    .child(
                        quiet(format!("copy-segment-{}", segment.id), "Copy")
                            .text_size(px(11.))
                            .px_2()
                            .py_0p5()
                            .min_h(px(22.))
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            }),
                    )
                    .child(
                        quiet(
                            format!("edit-segment-{}", segment.id),
                            if editing { "Close" } else { "Edit" },
                        )
                        .text_size(px(11.))
                        .px_2()
                        .py_0p5()
                        .min_h(px(22.))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if this.editing_segment.as_deref() == Some(segment_id.as_str()) {
                                    this.editing_segment = None;
                                } else {
                                    this.editing_segment = Some(segment_id.clone());
                                    this.edit_text.update(cx, |input, cx| {
                                        input.set_value(edit_text.clone(), window, cx)
                                    });
                                    this.edit_text.focus_handle(cx).focus(window, cx);
                                }
                                this.reset_reader_rows(false);
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(
                div()
                    .id(format!("segment-accessibility-{}", segment.id))
                    .role(Role::Label)
                    .aria_label(words.clone())
                    .text_size(px(15.))
                    .line_height(relative(1.55))
                    .child(
                        gpui_base::SelectableText::new(
                            format!("segment-text-{}", segment.id),
                            words,
                        )
                        .document_order(index as u64),
                    ),
            );
        if let Some(a) = attribution {
            let mut evidence = div().flex().flex_wrap().gap_2();
            if a.overlapping_speech || a.overlapping_source_audio {
                evidence = evidence.child(pill("Overlapping audio", WARNING));
            }
            if a.possible_cross_source_duplicate {
                evidence = evidence.child(pill("Possible duplicate across sources", WARNING));
            }
            if segment.evidence["uncertain_speech"] == true {
                evidence = evidence.child(pill("Uncertain speech", WARNING));
            }
            if a.state == "ambiguous" && correction.speaker.is_none() {
                evidence = evidence.child(caption(
                    "Multiple voices or a turn boundary · identity unresolved",
                ));
            }
            body = body.child(evidence);
        }
        if editing {
            body = body.child(self.segment_editor(segment, correction, cx));
        }
        if self.show_word_evidence {
            let estimated = detail.row_word_indices[index]
                .iter()
                .map(|i| &detail.word_attributions[*i])
                .map(|w| {
                    format!(
                        "{} [{} · {}]",
                        w.word.text,
                        time_label(w.word.start_ms),
                        if w.attribution.state == "anonymous_model_attribution" {
                            w.attribution
                                .anonymous_speakers
                                .iter()
                                .map(|s| anonymous_label(s))
                                .collect::<Vec<_>>()
                                .join(", ")
                        } else {
                            "Unknown".into()
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join(" · ");
            if !estimated.is_empty() {
                body = body.child(caption(estimated).p_3().bg(rgb(PANEL)).rounded_md());
            }
        }
        div()
            .flex_shrink_0()
            .py_4()
            .border_b_1()
            .border_color(rgb(0x2a2e36))
            .flex()
            .gap_3()
            .child(
                div()
                    .mt_0p5()
                    .size(px(30.))
                    .rounded_full()
                    .bg(rgba((color << 8) | 0x20))
                    .text_color(rgb(color))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if confirmed {
                        label.chars().take(2).collect::<String>()
                    } else if label.starts_with("Unknown") {
                        "?".into()
                    } else {
                        "S".into()
                    }),
            )
            .child(body)
    }
    fn segment_editor(
        &self,
        segment: &TranscriptSegment,
        correction: &corrections::CorrectionState,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = segment.id.clone();
        let mut people = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(caption("Speaker").py_1());
        let unknown_id = id.clone();
        people = people.child(button(format!("unknown-{}", id), "Unknown").on_click(
            cx.listener(move |this, _, _, cx| this.assign_segment(unknown_id.clone(), None, cx)),
        ));
        for profile in &self.profiles {
            let profile_id = profile.id.clone();
            let segment_id = id.clone();
            people = people.child(
                button(
                    format!("assign-{}-{}", id, profile.id),
                    profile.name.clone(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.assign_segment(segment_id.clone(), Some(profile_id.clone()), cx)
                })),
            );
        }
        let undo_id = id.clone();
        div()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(PANEL))
            .flex()
            .flex_col()
            .gap_3()
            .child(people)
            .child(caption(
                "Assign only after listening. When the speech is clean, your confirmation also saves a removable voice example; undoing the correction removes it.",
            ))
            .child(
                div()
                    .id("transcript-input-context")
                    .key_context("TranscriptEditor")
                    .p_3()
                    .rounded_md()
                    .bg(rgb(SIDEBAR))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(Textarea::new(&self.edit_text)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        primary(format!("save-text-{id}"), "Save text")
                            .disabled(self.pending > 0)
                            .on_click(cx.listener(|this, _, _, cx| this.save_segment_text(cx))),
                    )
                    .child(
                        quiet(format!("undo-{id}"), "Undo latest correction")
                            .disabled(!correction.can_undo || self.pending > 0)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.undo_segment(undo_id.clone(), cx)
                            })),
                    )
                    .child(caption("⌘↵ to save text")),
            )
    }
    fn audio_view(&self, detail: &RecordingDetail, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut view = div().p_5().flex().flex_col().gap_3().child(section(
            "Audio sources",
            "Sources preserve their own timing. A microphone track is not a verified identity.",
        ));
        let id = detail.recording.id.clone();
        view = view.child(
            primary("finalize-meeting", "Finish transcription of all sources")
                .disabled(self.pending > 0)
                .on_click(cx.listener(move |this, _, _, cx| this.finalize_meeting(id.clone(), cx))),
        );
        for run in self
            .pending_jobs
            .iter()
            .filter(|r| r.recording_id == detail.recording.id)
        {
            view = view.child(self.processing_job(run, cx));
        }
        for track in &detail.tracks {
            let id = track.id.clone();
            let selected = self.selected_track.as_deref() == Some(id.as_str());
            view = view.child(
                button(format!("source-{id}"), "")
                    .accessibility_label(format!("Select {}", track_name(&track.kind)))
                    .aria_selected(selected)
                    .w_full()
                    .justify_start()
                    .p_3()
                    .gap_3()
                    .when(selected, |b| b.bg(rgb(SELECTED)).border_color(rgb(ACCENT)))
                    .child(
                        icon(if track.kind == TrackKind::Microphone {
                            "mic"
                        } else {
                            "wave"
                        })
                        .text_color(rgb(ACCENT)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(track_name(&track.kind))
                            .when(track.provenance["silent_source"] == true, |column| {
                                column
                                    .child(pill("Silent source — no speech-level signal", WARNING))
                            })
                            .child(caption(format!(
                                "{} · {} · {}",
                                time_label(track.duration_ms),
                                track
                                    .sample_rate
                                    .map(|r| format!("{} Hz", r))
                                    .unwrap_or("Rate unknown".into()),
                                if track.meeting_offset_ms().is_some() {
                                    "Aligned to recording"
                                } else {
                                    "Timing unresolved"
                                }
                            ))),
                    )
                    .child(pill(
                        if track.available {
                            "Saved source"
                        } else {
                            "Source missing"
                        },
                        if track.available { SUCCESS } else { WARNING },
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_track = Some(id.clone());
                        this.cluster_review = None;
                        this.selected_clusters.clear();
                        cx.notify();
                    })),
            );
        }
        if let Some(track) = self
            .selected_track
            .as_ref()
            .and_then(|id| detail.tracks.iter().find(|t| &t.id == id))
        {
            view=view.child(div().mt_2().flex().flex_col().gap_2().child(caption(format!("Selected source: {}",track_name(&track.kind))))
                .child(div().flex().flex_wrap().gap_2()
                    .child(primary("transcribe-source","Transcribe source").disabled(!track.available||self.pending>0).on_click(cx.listener(|this,_,_,cx|this.process(false,false,cx))))
                    .child(button("diarize-source","Detect speakers").disabled(!track.available||self.pending>0).on_click(cx.listener(|this,_,_,cx|this.process(true,false,cx))))
                    .child(quiet("identify-voices","Identify voices").on_click(cx.listener(|this,_,_,cx|{this.speaker_mode=SpeakerMode::Train;this.navigate(Area::Speakers,cx);})))
                    .child(quiet("review-source-clusters","Review clusters").on_click(cx.listener(|this,_,_,cx|{this.speaker_mode=SpeakerMode::Clusters;this.navigate(Area::Speakers,cx);this.inspect_clusters(cx);}))))
                .child(caption("Processing creates a new result. Existing audio, title, and corrections are retained.")));
            view = view.child(
                div()
                    .mt_3()
                    .p_4()
                    .rounded_lg()
                    .bg(rgb(PANEL))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Listen to an interval"),
                    )
                    .child(caption(
                        "Track-local milliseconds · choose up to 30 seconds",
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(caption("Start"))
                            .child(field(&self.start).w(px(110.)))
                            .child(caption("End"))
                            .child(field(&self.end).w(px(110.)))
                            .child(
                                button("play-interval", "Play clip")
                                    .on_click(cx.listener(|this, _, _, cx| this.play(cx))),
                            ),
                    ),
            );
            view = view.child(
                quiet(
                    "audio-locations",
                    if self.show_location_tools {
                        "Hide file details"
                    } else {
                        "File details & moved audio"
                    },
                )
                .justify_start()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_location_tools = !this.show_location_tools;
                    cx.notify();
                })),
            );
            if self.show_location_tools {
                view=view.child(div().p_4().bg(rgb(PANEL)).rounded_lg().flex().flex_col().gap_2()
                    .child(caption(track.path.clone()))
                    .child(button("fingerprint-audio","Save content fingerprint").on_click(cx.listener(|this,_,_,cx|this.fingerprint_audio(cx))))
                    .child(caption("A moved file must be byte-identical. Relinking updates this library only."))
                    .child(field(&self.relink_path))
                    .child(div().flex().flex_wrap().gap_2().child(button("check-replacement","Check replacement").on_click(cx.listener(|this,_,_,cx|this.relink_audio(true,cx))))
                        .child(button("relink-audio","Relink audio").on_click(cx.listener(|this,_,_,cx|this.relink_audio(false,cx))))));
            }
            view=view.child(div().mt_3().p_4().rounded_lg().border_1().border_color(rgb(BORDER)).flex().flex_col().gap_2()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Experimental word timing"))
                .child(caption("Whisper can retain estimated word times for speaker review. These estimates are not verified alignments."))
                .child(button("transcribe-estimated-words","Transcribe with word estimates").disabled(!track.available||self.pending>0).on_click(cx.listener(|this,_,_,cx|this.process(false,true,cx)))));
        }
        div()
            .id("recording-audio")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .child(view)
    }
    fn processing_view(&self, detail: &reader::Document, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut view = div().p_5().flex().flex_col().gap_3().child(section(
            "Processing history",
            "Original results and errors remain available after reprocessing.",
        ));
        view = view.child(self.analysis_panel(detail, cx));
        for run in self
            .pending_jobs
            .iter()
            .filter(|r| r.recording_id == detail.recording.id)
        {
            view = view.child(self.processing_job(run, cx));
        }
        if let Some(plan) = detail.recording.metadata.get("meeting_finalization") {
            view = view.child(caption(format!(
                "Final transcription: {} · {} · {}/{} sources complete",
                plan["status"].as_str().unwrap_or("invalid plan"),
                plan["model"].as_str().unwrap_or("unknown"),
                plan["completed"].as_array().map_or(0, Vec::len),
                plan["track_ids"].as_array().map_or(0, Vec::len)
            )));
            if let Some(failures) = plan["failures"].as_array() {
                for failure in failures {
                    view = view.child(
                        caption(
                            failure["message"]
                                .as_str()
                                .unwrap_or("Unknown finalization error"),
                        )
                        .text_color(rgb(WARNING)),
                    );
                }
            }
            let id = detail.recording.id.clone();
            view = view.child(
                button("resume-finalization", "Resume final transcription")
                    .disabled(
                        self.pending > 0
                            || (plan["status"] == "completed"
                                && (plan["speaker_analysis"].is_null()
                                    || detail
                                        .analysis
                                        .as_ref()
                                        .ok()
                                        .and_then(Option::as_ref)
                                        .is_some_and(|a| a.status == "completed"))),
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.finalize_meeting(id.clone(), cx)),
                    ),
            );
        }
        if !detail.unmapped_corrected_segments.is_empty() {
            view = view.child(pill(
                format!(
                    "{} earlier corrected intervals retained for review",
                    detail.unmapped_corrected_segments.len()
                ),
                WARNING,
            ));
        }
        if detail.runs.is_empty() {
            view = view.child(caption(
                "No processing runs yet. Imported transcript evidence remains available.",
            ));
        }
        for run in detail.runs.iter().rev().take(if self.show_run_history {
            usize::MAX
        } else {
            10
        }) {
            let id = run.id.clone();
            let color = match run.status.as_str() {
                "completed" => SUCCESS,
                "failed" => DANGER,
                _ => WARNING,
            };
            view = view.child(
                div()
                    .p_4()
                    .rounded_lg()
                    .bg(rgb(PANEL))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(run.model.clone()),
                            )
                            .child(pill(run.status.clone(), color)),
                    )
                    .child(caption(format!(
                        "{} · {}{}",
                        date_label(&run.started_at),
                        run.kind,
                        if run.configuration["human_edit"].is_object() {
                            " · Manual cluster edit"
                        } else {
                            ""
                        }
                    )))
                    .child(caption(format!("Run {}", run.id)))
                    .when(run.result["metrics"]["resume"]["status"] == "reused", |row| {
                        let resume=&run.result["metrics"]["resume"];
                        row.child(caption(format!("Reused {} saved chunks · {} audio · {} new inference chunks",resume["reused_chunks"].as_u64().unwrap_or(0),time_label(resume["reused_audio_ms"].as_i64()),run.result["metrics"]["inference_chunks"].as_u64().unwrap_or(0))))
                    })
                    .when(run.result["metrics"]["resume"]["status"] == "restart_required", |row| {
                        row.child(caption(format!("Processed the source again because saved chunks could not be reused: {}",run.result["metrics"]["resume"]["reason"].as_str().unwrap_or("incompatible evidence"))))
                    })
                    .when_some(run.error.clone(), |row, error| {
                        row.child(
                            div()
                                .text_color(rgb(if run.status == "interrupted" {
                                    WARNING
                                } else {
                                    DANGER
                                }))
                                .child(error),
                        )
                    })
                    .when(
                        ["failed", "interrupted"].contains(&run.status.as_str())
                            && run.configuration["request"].is_object(),
                        |row| {
                            row.child(
                                button(format!("retry-{}", run.id), "Retry from retained audio")
                                    .disabled(self.pending > 0)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let service = this.processing.clone();
                                        let library = this.library.clone();
                                        let id = id.clone();
                                        this.task(
                                            "Retrying retained audio",
                                            async move {
                                                let run = service.retry(&id).await?;
                                                Ok(Update::detail(
                                                    library.show(&run.recording_id).await?,
                                                ))
                                            },
                                            cx,
                                        );
                                    })),
                            )
                        },
                    ),
            );
        }
        if detail.runs.len() > 10 {
            view = view.child(
                quiet(
                    "all-processing-runs",
                    if self.show_run_history {
                        "Show recent runs"
                    } else {
                        "Show all runs"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_run_history = !this.show_run_history;
                    cx.notify();
                })),
            );
        }
        div()
            .id("recording-processing")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .child(view)
    }
}

/// A speaker label carried over from an imported MacParakeet transcript: a claim, never a verified name.
fn imported_claim(segment: &TranscriptSegment) -> Option<&str> {
    (segment.evidence["origin"] == "imported_unverified")
        .then_some(segment.speaker_claim.as_deref())
        .flatten()
}

/// A name the saved comparisons propose for this segment's speech. It is a prediction, shown
/// apart from confirmed names, and only while no decision on that comparison is active.
fn suggested_name(
    view: &Workbench,
    detail: &reader::Document,
    segment: &TranscriptSegment,
) -> Option<String> {
    let track_id = segment.track_id.as_ref()?;
    let offset = detail
        .tracks
        .iter()
        .find(|t| &t.id == track_id)?
        .meeting_offset_ms()?;
    let (start, end) = (segment.start_ms? - offset, segment.end_ms? - offset);
    view.suggestions
        .iter()
        .filter(|s| {
            &s.track_id == track_id
                && s.resolutions.last().is_none_or(|r| r.action == "undo")
                && std::iter::once((s.start_ms, s.end_ms))
                    .chain(s.more_ranges.iter().copied())
                    .any(|(from, to)| start >= from && end <= to)
        })
        .find_map(|s| {
            let id = s.decision.suggested_profile.as_ref()?;
            view.profiles
                .iter()
                .find(|p| &p.id == id)
                .map(|p| p.name.clone())
        })
}
