//! The Models, Record and Dictation screens: one obvious primary action each, plain-language
//! status, and diagnostics behind the status bar's Details toggle.
use super::*;

fn model_blurb(id: &str) -> &'static str {
    match id {
        "parakeet-tdt-0.6b-v2" => {
            "Most accurate in our tests and much faster than the others. English only."
        }
        "whisper-large-v3" => {
            "Slower and a little less accurate on meetings. Handles other languages."
        }
        "whisper-large-v3-turbo" => "Faster than Large v3, slightly less accurate.",
        "whisper-medium.en" => "Mid-sized English model. Less accurate than the recommended one.",
        "whisper-small.en" => "Small and quick. Noticeably less accurate.",
        "whisper-base.en" => "Smallest and quickest. Least accurate.",
        "nemotron3-fast128" => "Finds who is speaking when. Fast; up to 8 voices.",
        "nemotron3-offline" => "Finds who is speaking when. Slower; up to 8 voices.",
        "speakrs-community-1-coreml" => {
            "Finds who is speaking and recognizes people you have named."
        }
        "speakrs-community-1-cpu" => "Same as above, without the Mac's neural hardware. Slower.",
        _ => "",
    }
}

fn capture_state_label(state: &str) -> &str {
    match state {
        "idle" => "Not recording",
        "starting" => "Checking audio…",
        "recording" => "Recording",
        "waiting_or_stalled" => "Waiting for audio…",
        "source_failed" => "A source stopped working",
        "source_interrupted" => "A source was interrupted",
        "recording_with_warnings" => "Recording · check audio",
        "finalizing" => "Checking saved audio…",
        "completed_with_warnings" => "Audio saved with problems",
        "failed" => "Recording failed",
        "start_failed" => "Recording not started",
        "completed" => "Saved",
        other => other,
    }
}

/// Modifier keys held on their own, as offered on the Dictation screen.
const HOLD_KEYS: [(&str, &str); 4] = [
    ("rightcommand", "Right ⌘"),
    ("rightoption", "Right ⌥"),
    ("rightcontrol", "Right ⌃"),
    ("fn", "Fn"),
];
fn shortcut_label(binding: &gestures::Shortcut) -> String {
    let key = binding.key.to_lowercase();
    let hold = [
        ("leftcommand", "Left ⌘"),
        ("leftoption", "Left ⌥"),
        ("leftcontrol", "Left ⌃"),
        ("rightshift", "Right ⇧"),
        ("leftshift", "Left ⇧"),
    ];
    if let Some((_, name)) = HOLD_KEYS.iter().chain(&hold).find(|(id, _)| *id == key) {
        return format!("Hold {name}");
    }
    let mut label: String = ["ctrl", "alt", "shift", "cmd"]
        .iter()
        .filter(|m| binding.modifiers.iter().any(|b| b == *m))
        .map(|m| match *m {
            "ctrl" => "⌃",
            "alt" => "⌥",
            "shift" => "⇧",
            _ => "⌘",
        })
        .collect();
    label.push_str(&match key.as_str() {
        "space" => "Space".to_owned(),
        "pageup" => "Page Up".to_owned(),
        "pagedown" => "Page Down".to_owned(),
        "home" | "end" => format!("{}{}", key[..1].to_uppercase(), &key[1..]),
        _ => key.to_uppercase(),
    });
    label
}

impl Workbench {
    pub(super) fn refresh_permissions(&mut self, cx: &mut Context<Self>) {
        let service = self.audio_inspection.clone();
        self.permissions_error = None;
        self.task(
            "Checking macOS permissions",
            async move {
                Ok(Update::Permissions(
                    service.permissions().await.map_err(|e| e.to_string()),
                ))
            },
            cx,
        );
    }
    fn permission_granted(&self, kind: &str) -> Option<bool> {
        let p = self.permissions.as_ref()?;
        if kind == "microphone" {
            p["microphone_status"].as_u64().map(|s| s == 3)
        } else {
            p[if kind == "screen_recording" {
                "screen_capture"
            } else {
                kind
            }]
            .as_bool()
        }
    }
    pub(super) fn dictation_blocker(&self) -> Option<String> {
        if self.capture_stop.is_some() {
            return Some("Finish the meeting recording before using dictation.".into());
        }
        if !self.shortcuts.iter().any(|s| s.enabled) {
            return Some("Add and enable a shortcut below to use dictation.".into());
        }
        let selected = self.audio_preferences.microphone_uid.as_deref();
        if selected.is_none()
            || !self.devices["devices"]
                .as_array()
                .is_some_and(|ds| ds.iter().any(|d| d["uid"].as_str() == selected))
        {
            return Some("Choose an available microphone below.".into());
        }
        if self.permissions_error.is_some() {
            return Some(
                "Permissions could not be checked. Open permissions and refresh their status."
                    .into(),
            );
        }
        let missing: Vec<_> = [
            ("microphone", "Microphone"),
            ("accessibility", "Pasting text"),
            ("input_monitoring", "Shortcuts"),
        ]
        .into_iter()
        .filter(|(kind, _)| self.permission_granted(kind) != Some(true))
        .map(|(_, label)| label)
        .collect();
        if !missing.is_empty() {
            return Some(if self.permissions.is_none() {
                "Checking macOS permissions…".into()
            } else {
                format!(
                    "Allow {} in Privacy & permissions before turning on dictation.",
                    missing.join(", ")
                )
            });
        }
        None
    }

    pub(super) fn models_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div().flex().flex_col().gap_3().child(section(
            "Local models",
            "Everything runs on this Mac. A model is downloaded once, then works offline.",
        ));
        if let Some(progress) = self.models.download_progress() {
            view = view.child(panel().child(format!(
                    "Downloading {} · file {}/{} · {:.0} of {} MB",
                    ui::model_name(&progress.model),
                    progress.file_index,
                    progress.total_files,
                    progress.downloaded_bytes as f64 / 1_000_000.,
                    progress
                        .total_bytes
                        .map(|n| format!("{:.0}", n as f64 / 1_000_000.))
                        .unwrap_or_else(|| "?".into())
                )));
        }
        for (category, title, hint) in [
            (
                "asr",
                "Transcription",
                "Turns speech into text. The one marked In use transcribes everything.",
            ),
            (
                "diarization",
                "Speakers",
                "Finds the separate voices in a recording and compares them with people you have named.",
            ),
        ] {
            view = view.child(group(title, hint));
            let cards = self
                .model_list
                .iter()
                .filter(|m| m.category == category)
                .map(|m| self.model_card(m, cx).into_any_element())
                .collect();
            view = view.child(grid(self.columns, cards));
        }
        view
    }

    fn model_card(&self, m: &ModelInfo, cx: &mut Context<Self>) -> Div {
        let in_use = m.detail["selected"] == true;
        let for_voices = m.detail["selected_for_embeddings"] == true;
        let busy = self.pending > 0;
        let (install_id, verify_id, select_id, embedding_id, remove_id) = (
            m.id.clone(),
            m.id.clone(),
            m.id.clone(),
            m.id.clone(),
            m.id.clone(),
        );
        let install = move |this: &mut Self, cx: &mut Context<Self>| {
            let service = this.models.clone();
            let id = install_id.clone();
            this.task(
                "Downloading local model",
                async move {
                    service.install(&id).await?;
                    Ok(Update::Models(service.list().await?))
                },
                cx,
            );
        };
        let verify = move |this: &mut Self, cx: &mut Context<Self>| {
            let service = this.models.clone();
            let id = verify_id.clone();
            this.task(
                "Verifying model files",
                async move {
                    service.verify(&id).await?;
                    Ok(Update::Models(service.list().await?))
                },
                cx,
            );
        };
        let select = move |this: &mut Self, cx: &mut Context<Self>| {
            let service = this.models.clone();
            let id = select_id.clone();
            this.task(
                "Selecting model",
                async move {
                    service.select(&id).await?;
                    Ok(Update::Models(service.list().await?))
                },
                cx,
            );
        };
        let (status, color) = if in_use {
            ("In use", SUCCESS)
        } else if m.verified {
            ("Ready", SUCCESS)
        } else if m.installed {
            ("Needs a file check", WARNING)
        } else {
            ("Not downloaded", MUTED)
        };
        let mut actions = div().flex().flex_wrap().items_center().gap_2();
        if !m.installed {
            actions = actions.child(
                primary(format!("install-{}", m.id), "Download")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| install(this, cx))),
            );
        } else if !m.verified {
            actions = actions.child(
                primary(format!("verify-{}", m.id), "Check files")
                    .disabled(busy)
                    .on_click(cx.listener({
                        let verify = verify.clone();
                        move |this, _, _, cx| verify(this, cx)
                    })),
            );
        } else if !in_use {
            actions = actions.child(
                primary(format!("select-{}", m.id), "Use this model")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| select(this, cx))),
            );
        }
        if m.verified && m.detail["provides_voice_embeddings"] == true && !for_voices {
            actions = actions.child(
                button(
                    format!("embedding-{}", m.id),
                    "Also use for voice recognition",
                )
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let service = this.models.clone();
                    let id = embedding_id.clone();
                    this.task(
                        "Selecting embedding model",
                        async move {
                            service.select_embedding(&id).await?;
                            Ok(Update::Models(service.list().await?))
                        },
                        cx,
                    );
                })),
            );
        }
        if m.installed && m.verified {
            actions = actions.child(
                quiet(format!("verify-{}", m.id), "Check files again")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| verify(this, cx))),
            );
        }
        if m.installed {
            actions = actions.child(
                quiet(format!("remove-model-{}", m.id), "Remove…")
                    .text_color(rgb(DANGER))
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.confirm_action =
                            Some(shell::ConfirmAction::RemoveModel(remove_id.clone()));
                        cx.notify();
                    })),
            );
        }
        let mut card = panel()
            .size_full()
            .gap_2()
            .when(in_use || for_voices, |c| {
                c.border_color(rgba((SUCCESS << 8) | 0x80))
            })
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(ui::model_name(&m.id).to_owned()),
                            )
                            .child(caption(
                                m.bytes
                                    .map(|b| format!("{:.0} MB download", b as f64 / 1_000_000.))
                                    .unwrap_or_default(),
                            )),
                    )
                    .child(pill(status, color)),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(MUTED))
                    .child(model_blurb(&m.id)),
            );
        if m.id == "parakeet-tdt-0.6b-v2" || for_voices {
            let mut tags = div().flex().flex_wrap().gap_2();
            if m.id == "parakeet-tdt-0.6b-v2" {
                tags = tags.child(pill("Recommended", MUTED));
            }
            if for_voices {
                tags = tags.child(pill("Used for voice recognition", SUCCESS));
            }
            card = card.child(tags);
        }
        if self.show_status_details {
            card = card.child(caption(format!(
                "{} · revision {} · {}",
                m.id, m.revision, m.license
            )));
        }
        card.child(div().flex_1()).child(actions)
    }

    /// The microphones to choose from, shared by the Record and Dictation screens.
    fn microphone_picker(&self, cx: &mut Context<Self>) -> Div {
        let recording = self.capture_stop.is_some();
        let mut view = div().flex().flex_col().gap_2();
        let devices = self.devices["devices"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if devices.is_empty() {
            view = view.child(caption(if self.pending > 0 {
                "Checking available audio inputs…"
            } else {
                "No microphones found. Connect one and press Refresh. You can still record meeting audio alone."
            }));
        }
        if recording || self.dictation_stop.is_some() {
            view = view.child(caption(if recording {
                "Stop the recording to change the microphone."
            } else {
                "Turn off dictation to change the microphone."
            }));
        }
        let tiles = devices
            .iter()
            .enumerate()
            .map(|(index, device)| {
                let uid = device["uid"].as_str().unwrap_or_default().to_owned();
                let selected = self.audio_preferences.microphone_uid.as_ref() == Some(&uid);
                tile(
                    ("device", index),
                    device["name"]
                        .as_str()
                        .unwrap_or("Unknown input")
                        .to_owned(),
                    (device["is_default"] == true).then(|| "Mac default".to_owned()),
                    selected,
                )
                .disabled(recording || self.dictation_stop.is_some())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let uid = uid.clone();
                    this.change_preferences(
                        "Saving explicit microphone",
                        "Selected microphone saved",
                        move |p| p.microphone_uid = Some(uid.clone()),
                        cx,
                    );
                }))
                .into_any_element()
            })
            .collect();
        view.child(grid(self.columns, tiles))
    }
    pub(super) fn capture_view(&self, cx: &mut Context<Self>) -> Div {
        let health = self.capture.status();
        let recording = self.capture_stop.is_some();
        let state = health["state"].as_str().unwrap_or("idle");
        let microphone = self
            .audio_preferences
            .microphone_uid
            .as_ref()
            .and_then(|uid| {
                self.devices["devices"].as_array().and_then(|devices| {
                    devices
                        .iter()
                        .find(|d| d["uid"].as_str() == Some(uid.as_str()))
                })
            })
            .and_then(|d| d["name"].as_str());
        let no_mic = microphone.is_none();
        let mut view = div().w_full().max_w(px(1080.)) .flex().flex_col().gap_3().child(section(
            "Record a meeting",
            "Record your microphone and meeting audio together. Transcription starts after you stop.",
        ));
        let mut card = panel()
            .max_w(px(760.))
            .child(div().flex().child(pill(
                capture_state_label(state).to_owned(),
                match state {
                    "completed" => SUCCESS,
                    "recording" => DANGER,
                    "source_failed"
                    | "source_interrupted"
                    | "waiting_or_stalled"
                    | "starting"
                    | "recording_with_warnings"
                    | "completed_with_warnings"
                    | "finalizing" => WARNING,
                    "failed" | "start_failed" => DANGER,
                    _ => MUTED,
                },
            )))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        primary("record-both", "Record meeting")
                            .disabled(recording || no_mic)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.start_capture(true, true, cx)),
                            ),
                    )
                    .child(
                        danger("stop-recording", "Stop & save")
                            .disabled(
                                !recording
                                    || matches!(
                                        state,
                                        "completed" | "completed_with_warnings" | "finalizing"
                                    ),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(stop) = &this.capture_stop {
                                    stop.store(true, Ordering::SeqCst);
                                    this.status = "Stopping and finalizing sources".into();
                                    cx.notify();
                                }
                            })),
                    )
                    // Without a microphone, meeting audio alone is the way to record.
                    .when(self.show_status_details || no_mic, |row| {
                        row.child(
                            quiet("record-system", "Record meeting audio only")
                                .disabled(recording)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_capture(false, true, cx)
                                })),
                        )
                    })
                    .when(self.show_status_details, |row| {
                        row.child(
                            quiet("record-mic", "Microphone only")
                                .disabled(recording || no_mic)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_capture(true, false, cx)
                                })),
                        )
                    }),
            )
            .child(caption(match microphone {
                Some(name) => format!("Microphone: {name}. Change it below."),
                None if self.audio_preferences.microphone_uid.is_some() => {
                    "Your selected microphone is unavailable. Reconnect it and press Refresh."
                        .into()
                }
                None => "Choose a microphone below to start recording.".into(),
            }));
        if let Some(started) = self.capture_started.filter(|_| recording) {
            card = card.child(
                div()
                    .text_size(px(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(library::time_label(Some(
                        started.elapsed().as_millis() as i64
                    ))),
            );
        }
        if !self.finishing_meetings.is_empty() {
            card = card.child(caption(
                "Transcribing your last meeting in the background. You can record again now.",
            ));
        }
        let transcriber_ready = self
            .model_list
            .iter()
            .any(|m| m.category == "asr" && m.verified && m.detail["selected"] == true);
        if !recording
            && !self.model_list.is_empty()
            && !transcriber_ready
            && self.audio_preferences.transcribe_meetings_after_stop
        {
            card = card.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        caption("No transcription model is ready, so a meeting will be saved as audio only.")
                            .text_color(rgb(WARNING)),
                    )
                    .child(
                        quiet("capture-models", "Local models")
                            .on_click(cx.listener(|this, _, _, cx| this.navigate(Area::Models, cx))),
                    ),
            );
        }
        if let Some(message) = health["message"].as_str() {
            card = card.child(div().text_color(rgb(WARNING)).child(message.to_owned()));
        }
        for (source, name) in [("microphone", "Microphone"), ("system", "Meeting audio")] {
            let h = &health[source];
            if h.is_null() {
                continue;
            }
            let active = matches!(
                state,
                "starting"
                    | "recording"
                    | "waiting_or_stalled"
                    | "source_failed"
                    | "source_interrupted"
                    | "recording_with_warnings"
            );
            let wanted = !((source == "microphone"
                && health["requested_microphone_uid"].is_null())
                || (source == "system" && health["requested_system_audio"] != true));
            let text = if !active {
                "not recording"
            } else if !wanted {
                "not requested"
            } else if h["is_receiving"] == true {
                "receiving audio"
            } else if h["first_buffer_ready"] == true {
                "stalled — check the source"
            } else {
                "waiting for audio"
            };
            let frames = h["written_frames"].as_f64().unwrap_or(0.);
            let rate = h["sample_rate"].as_f64().unwrap_or(0.);
            let silent = rate > 0.
                && frames / rate >= 10.
                && h["session_peak_amplitude"]
                    .as_f64()
                    .is_some_and(|peak| peak < parakeet_domain::SILENT_SOURCE_PEAK);
            if active && wanted || self.show_status_details {
                card = card.child(caption(format!("{name}: {text}")));
            }
            if silent {
                card = card.child(
                    div()
                        .text_color(rgb(WARNING))
                        .child(format!("{name} is silent: buffers arrive but there is no speech-level sound. Check the selected input.")),
                );
            }
            if self.show_status_details {
                card = card.child(caption(format!(
                    "{frames} frames · {rate} Hz · peak {:.3}",
                    h["peak_amplitude"].as_f64().unwrap_or(0.)
                )));
            }
        }
        if self.show_status_details {
            card = card
                .child(caption(format!("Capture state: {state}")))
                .child(caption(format!(
                    "Selected input UID: {}",
                    self.audio_preferences
                        .microphone_uid
                        .as_deref()
                        .unwrap_or("none")
                )));
            if let Some(uid) = health["effective_microphone_uid"].as_str() {
                card = card.child(caption(format!("Effective microphone UID: {uid}")));
            }
        }
        view = view.child(card);
        view = view.child(group_action(
            "Microphone",
            "Choose your microphone. Recording checks that it works before starting.",
            button("refresh-audio-devices", "Refresh")
                .on_click(cx.listener(|this, _, _, cx| this.navigate(Area::Capture, cx))),
        ));
        view = view.child(self.microphone_picker(cx));
        view = view.child(group(
            "After you stop",
            "The audio is always saved first. Switched-on steps then run by themselves.",
        ));
        let busy = recording;
        let transcribe = self.audio_preferences.transcribe_meetings_after_stop;
        let analyze = self.audio_preferences.analyze_meetings_after_stop;
        let suggest = self.audio_preferences.suggest_speakers_after_stop;
        view.child(
            panel()
                .max_w(px(760.))
                .p_1()
                .gap_0()
                .child(
                    toggle_row(
                        "auto-final-transcription",
                        "Transcribe the meeting",
                        "Turn the saved audio into a transcript.",
                        transcribe,
                    )
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let on = !this.audio_preferences.transcribe_meetings_after_stop;
                        this.change_preferences(
                            "Saving meeting transcription preference",
                            "Meeting transcription preference saved",
                            move |p| {
                                p.transcribe_meetings_after_stop = on;
                                if !on {
                                    p.analyze_meetings_after_stop = false;
                                    p.suggest_speakers_after_stop = false;
                                }
                            },
                            cx,
                        );
                    })),
                )
                .child(
                    toggle_row(
                        "auto-speaker-analysis",
                        "Detect the speakers",
                        "Prepare voice clips for you to listen to and name. Includes transcription.",
                        analyze,
                    )
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let on = !this.audio_preferences.analyze_meetings_after_stop;
                        this.change_preferences(
                            "Saving speaker analysis preference",
                            "Speaker analysis preference saved",
                            move |p| {
                                p.analyze_meetings_after_stop = on;
                                if on {
                                    p.transcribe_meetings_after_stop = true;
                                } else {
                                    p.suggest_speakers_after_stop = false;
                                }
                            },
                            cx,
                        );
                    })),
                )
                .when(self.show_status_details || self.audio_preferences.matching_policy.is_some(), |panel| panel                .child(
                    toggle_row(
                        "auto-speaker-suggest",
                        "Suggest names for voices",
                        "Needs speaker detection and a recognition check (People → Check how well my verified voices are recognized). Suggestions are only shown; they never name anyone.",
                        suggest,
                    )
                    .disabled(busy || self.audio_preferences.matching_policy.is_none() || !analyze)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let on = !this.audio_preferences.suggest_speakers_after_stop;
                        this.change_preferences(
                            "Saving suggestion preference",
                            "Suggestion preference saved",
                            move |p| p.suggest_speakers_after_stop = on,
                            cx,
                        );
                    })),
                ),),
        )
    }

    pub(super) fn dictation_view(&self, cx: &mut Context<Self>) -> Div {
        let live = self
            .dictation_state
            .lock()
            .ok()
            .map(|v| v.clone())
            .unwrap_or_default();
        let on = self.dictation_stop.is_some();
        let blocker = self.dictation_blocker();
        let mut view = div().w_full().max_w(px(760.)) .flex().flex_col().gap_3().child(section(
            "Dictation",
            "Hold a shortcut, speak, then let go. Your words are transcribed on this Mac and pasted where you were typing.",
        ));
        let mut card = panel()
            .child(div().flex().child(pill(
                if on { "Dictation is on" } else { "Dictation is off" },
                if on { SUCCESS } else { MUTED },
            )))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        primary("start-listening", if self.dictation_starting { "Checking…" } else { "Turn on dictation" })
                            .disabled(on || self.dictation_starting || blocker.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.start_dictation(cx))),
                    )
                    .child(
                        button("stop-listening", "Turn off")
                            .disabled(!on)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(stop) = this.dictation_stop.take() {
                                    stop.store(true, Ordering::SeqCst);
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(caption(if ["microphone", "accessibility", "input_monitoring"].iter().all(|kind| self.permission_granted(kind) == Some(true)) {
                        "Microphone, shortcuts and pasting are allowed."
                    } else { "Dictation needs access to your microphone, shortcuts and text insertion." }).flex_1().min_w_0())
                    .child(quiet("dictation-permissions", "Permissions").on_click(
                        cx.listener(|this, _, _, cx| this.navigate(Area::Privacy, cx)),
                    )),
            );
        if !on && let Some(reason) = blocker {
            card = card.child(caption(reason).text_color(rgb(WARNING)));
        }
        if let Some(message) = live["message"].as_str() {
            card = card.child(div().text_color(rgb(WARNING)).child(message.to_owned()));
        }
        if self.show_status_details {
            card = card.child(caption(format!(
                "Dictation state: {}",
                live["state"].as_str().unwrap_or("idle")
            )));
        }
        view = view.child(card);
        view = view.child(group_action(
            "Microphone",
            "Dictation uses the same microphone as meetings.",
            button("refresh-dictation-devices", "Refresh")
                .on_click(cx.listener(|this, _, _, cx| this.navigate(Area::Dictation, cx))),
        ));
        view = view.child(self.microphone_picker(cx));
        if let Some(raw) = live["receipt"]["raw"].as_str().filter(|s| !s.is_empty()) {
            let text = raw.to_owned();
            view = view.child(
                div()
                    .p_4()
                    .rounded_lg()
                    .bg(rgb(PANEL))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(caption("LATEST"))
                    .child(raw.to_owned())
                    .child(
                        button("copy-live-dictation", "Copy text")
                            .self_start()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                            })),
                    ),
            );
        }
        view = view.child(group(
            "Shortcuts",
            "Hold any active shortcut to dictate. Several can be active at once.",
        ));
        let mut shortcuts = panel().gap_0().p_1();
        if self.shortcuts.is_empty() {
            shortcuts = shortcuts.child(div().p_3().child(caption(
                "No shortcut yet. Press Record a shortcut, then press the keys you want.",
            )));
        }
        let last = self.shortcuts.len().saturating_sub(1);
        for (index, binding) in self.shortcuts.iter().enumerate() {
            let toggle = binding.id.clone();
            let remove = binding.id.clone();
            shortcuts = shortcuts.child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(index < last, |row| {
                        row.border_b_1().border_color(rgb(BORDER))
                    })
                    .child(
                        div().flex_1().child(
                            div()
                                .self_start()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(BORDER))
                                .bg(rgb(ELEVATED))
                                .text_size(px(13.))
                                .child(shortcut_label(binding)),
                        ),
                    )
                    .child(pill(
                        if binding.enabled { "Active" } else { "Off" },
                        if binding.enabled { SUCCESS } else { MUTED },
                    ))
                    .child(
                        quiet(
                            ("toggle-binding", index),
                            if binding.enabled { "Disable" } else { "Enable" },
                        )
                        .disabled(on || self.dictation_starting)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let mut bindings = this.shortcuts.clone();
                            if let Some(b) = bindings.iter_mut().find(|b| b.id == toggle) {
                                b.enabled = !b.enabled;
                            }
                            this.persist_shortcuts(bindings, cx);
                        })),
                    )
                    .child(
                        quiet(("remove-binding", index), "Remove")
                            .disabled(on || self.dictation_starting)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let bindings = this
                                    .shortcuts
                                    .iter()
                                    .filter(|b| b.id != remove)
                                    .cloned()
                                    .collect();
                                this.persist_shortcuts(bindings, cx);
                            })),
                    ),
            );
        }
        view = view.child(shortcuts);
        if on {
            view = view.child(caption("Turn off dictation to change shortcuts."));
        }
        if self.recording_shortcut {
            view = view.child(caption(
                "Press the keys you want to hold while speaking. Escape cancels.",
            ));
        }
        if let Some(hint) = &self.shortcut_hint {
            view = view.child(caption(hint.clone()).text_color(rgb(WARNING)));
        }
        view = view.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    primary(
                        "record-shortcut",
                        if self.recording_shortcut {
                            "Press your shortcut…"
                        } else {
                            "Record a shortcut"
                        },
                    )
                    .disabled(on || self.dictation_starting)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.recording_shortcut = true;
                        this.shortcut_hint = None;
                        this.status = "Press the shortcut; Escape cancels".into();
                        cx.notify();
                    })),
                )
                .child(caption("or hold one key:"))
                .children(HOLD_KEYS.iter().enumerate().map(|(index, (key, name))| {
                    quiet(("hold-key", index), *name)
                        .disabled(on || self.dictation_starting)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.add_shortcut(key.to_string(), vec![], cx)
                        }))
                }))
                .when(self.show_status_details, |row| {
                    row.child(field(&self.shortcut_key).w(px(160.))).child(
                        quiet("add-shortcut", "Add by key name")
                            .disabled(on || self.dictation_starting)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let key = this.shortcut_key.read(cx).value().to_string();
                                this.add_shortcut(key, vec![], cx);
                            })),
                    )
                }),
        );
        view = view.child(group_action(
            "Recent dictations",
            "The raw words are always kept, so nothing is lost if pasting fails.",
            button("refresh-dictations", "Refresh").on_click(cx.listener(|this, _, _, cx| {
                let service = this.dictation_history.clone();
                this.task(
                    "Reading retained dictations",
                    async move { Ok(Update::Dictations(service.list().await?)) },
                    cx,
                );
            })),
        ));
        if self.dictations.is_empty() {
            view = view.child(caption("Nothing dictated yet."));
        }
        for (index, receipt) in self.dictations.iter().take(10).enumerate() {
            let raw = receipt.raw.clone();
            view = view.child(
                panel()
                    .p_3()
                    .gap_1()
                    .child(caption(format!(
                        "{} · {}",
                        receipt.created_at, receipt.status
                    )))
                    .child(receipt.raw.clone())
                    .when(self.show_status_details, |row| {
                        row.child(caption(format!("Pasted into {}", receipt.target.bundle_id)))
                            .when_some(receipt.insertion.clone(), |row, report| {
                                row.child(caption(format!(
                                    "Insertion: {} · {} · clipboard {}",
                                    report.method,
                                    report.verification,
                                    report.clipboard_status.as_deref().unwrap_or("untouched")
                                )))
                            })
                    })
                    .when_some(receipt.failure.clone(), |row, error| {
                        row.child(div().text_color(rgb(WARNING)).child(error))
                    })
                    .child(
                        quiet(("copy-retained", index), "Copy text")
                            .self_start()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(raw.clone()))
                            })),
                    ),
            );
        }
        view.when(self.show_status_details, |view| view.child(group(
            "Troubleshooting",
            "If dictation was interrupted while it held your clipboard, this puts your clipboard back.",
        ))
        .child(
            button("recover-clipboard", "Restore my clipboard")
                .self_start()
                .on_click(cx.listener(|this, _, _, cx| {
                    let service = this.insertion_recovery.clone();
                    this.task(
                        "Restoring interrupted clipboard pastes",
                        async move {
                            let report = service.recover().await?;
                            Ok(Update::Message(format!(
                                "Restored {} interrupted paste(s); {} older snapshots discarded after clipboard changed",
                                report.restored, report.discarded
                            )))
                        },
                        cx,
                    );
                })),
        ))
    }

    pub(super) fn privacy_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div()
            .w_full()
            .max_w(px(760.))
            .flex()
            .flex_col()
            .gap_3()
            .child(section(
                "Privacy & permissions",
                "What stays on this Mac, and what macOS lets the app do.",
            ));
        let mut facts = panel().gap_2();
        for fact in [
            "Transcription and speaker detection run on this Mac. Nothing is uploaded.",
            "Your raw speech is always kept, even if cleanup or pasting fails.",
            "Nothing is sent anywhere unless you set up and authorize an AI route under Advanced.",
        ] {
            facts = facts.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(icon("check").text_color(rgb(SUCCESS)))
                    .child(div().flex_1().child(fact)),
            );
        }
        view = view.child(facts);
        view = view.child(group(
            "macOS permissions",
            "Allow access for the features you use. After changing a permission in System Settings, refresh below.",
        ));
        let permissions = [
            (
                "microphone",
                "Microphone",
                "Record your voice in meetings and for dictation.",
            ),
            (
                "screen_recording",
                "System audio",
                "Record the other people in a meeting. macOS calls this Screen Recording.",
            ),
            (
                "accessibility",
                "Pasting text",
                "Paste dictated words where you were typing. macOS calls this Accessibility.",
            ),
            (
                "input_monitoring",
                "Shortcuts",
                "Hear your dictation shortcut. macOS calls this Input Monitoring.",
            ),
        ];
        let mut rows = panel().gap_0().p_1();
        view = view.child(
            button("refresh-permissions", "Refresh permission status")
                .on_click(cx.listener(|this, _, _, cx| this.refresh_permissions(cx))),
        );
        if let Some(error) = &self.permissions_error {
            view = view.child(
                caption(format!("Could not check permissions: {error}")).text_color(rgb(WARNING)),
            );
        }
        let last = permissions.len() - 1;
        for (index, (kind, title, purpose)) in permissions.into_iter().enumerate() {
            let granted = self.permission_granted(kind);
            let denied = kind == "microphone"
                && self
                    .permissions
                    .as_ref()
                    .is_some_and(|p| p["microphone_status"] == 2 || p["microphone_status"] == 1);
            rows = rows.child(
                div()
                    .px_3()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_4()
                    .when(index < last, |row| {
                        row.border_b_1().border_color(rgb(BORDER))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            )
                            .child(caption(purpose)),
                    )
                    .child(pill(
                        match granted {
                            Some(true) => "Granted",
                            Some(false) => "Not granted",
                            None => "Not checked",
                        },
                        if granted == Some(true) {
                            SUCCESS
                        } else {
                            MUTED
                        },
                    ))
                    .when(granted != Some(true), |row| {
                        row.child(
                            button(
                                ("request-permission", index),
                                if denied { "Open Settings" } else { "Allow" },
                            )
                            .self_center()
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let service = this.audio_inspection.clone();
                                    this.task(
                                        "Requesting macOS permission",
                                        async move {
                                            let result = async {
                                                service.request_permission(kind).await?;
                                                Ok::<_, anyhow::Error>(service.permissions().await?)
                                            }
                                            .await
                                            .map_err(|e| e.to_string());
                                            Ok(Update::Permissions(result))
                                        },
                                        cx,
                                    );
                                },
                            )),
                        )
                    }),
            );
        }
        view = view.child(rows);
        view.child(
            button(
                "toggle-ai-routes",
                if self.show_ai_routes {
                    "Hide advanced: AI cleanup of dictated text"
                } else {
                    "Advanced: AI cleanup of dictated text (off)"
                },
            )
            .self_start()
            .mt_3()
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_ai_routes = !this.show_ai_routes;
                cx.notify();
            })),
        )
    }
}
