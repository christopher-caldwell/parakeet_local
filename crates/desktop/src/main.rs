use gpui::{div, prelude::*, px, rgb, size, *};
use gpui_base::input::{Input, InputState, Textarea, TextareaState};
use parakeet_application::{capture::*, dictation::*, processing::*, settings::*, speakers::*, *};
use parakeet_database::SqliteStore;
use parakeet_domain::*;
use parakeet_infrastructure::{
    artifacts::MacParakeetArtifacts, audio::LocalPlayback, models::ModelFiles, speech::LocalSpeech,
    transformation::AuthorizedCli,
};
use std::{
    future::Future,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio::runtime::Runtime;

#[derive(Clone, Copy, PartialEq)]
enum Area {
    Library,
    Speakers,
    Models,
    Dictation,
    Privacy,
    Capture,
}
enum Update {
    Library(Vec<Recording>),
    Detail(RecordingDetail),
    Profiles(Vec<SpeakerProfile>),
    Examples(Vec<VoiceExampleSummary>),
    Candidates(Vec<EnrollmentCandidate>),
    RefreshSelected,
    Models(Vec<ModelInfo>),
    DictationStopped(String),
    Dictations(Vec<DictationReceipt>),
    Message(String),
    Settings(Vec<gestures::Shortcut>, Vec<PromptProfile>),
    Devices(serde_json::Value),
    Captured(Result<RecordingDetail, String>),
}
struct Workbench {
    runtime: Arc<Runtime>,
    library: Arc<LibraryService>,
    media: Arc<MediaTranscriptionService>,
    speakers: Arc<SpeakerService>,
    models: Arc<ModelService>,
    processing: Arc<ProcessingService>,
    settings: Arc<SettingsService>,
    playback: Arc<PlaybackService>,
    capture: Arc<CaptureService>,
    audio_inspection: Arc<AudioInspectionService>,
    audio_settings: Arc<AudioSettingsService>,
    dictation_listener: Arc<DictationListener>,
    dictation_history: Arc<DictationHistoryService>,
    dictations: Vec<DictationReceipt>,
    capture_stop: Option<Arc<AtomicBool>>,
    dictation_stop: Option<Arc<AtomicBool>>,
    dictation_overlay: Option<WindowHandle<DictationOverlay>>,
    dictation_state: Arc<Mutex<serde_json::Value>>,
    devices: serde_json::Value,
    audio_preferences: AudioPreferences,
    focus: FocusHandle,
    heartbeat_last: Instant,
    heartbeat_max_ms: f64,
    area: Area,
    records: Vec<Recording>,
    detail: Option<RecordingDetail>,
    profiles: Vec<SpeakerProfile>,
    examples: Vec<VoiceExampleSummary>,
    candidates: Vec<EnrollmentCandidate>,
    recording_shortcut: bool,
    model_list: Vec<ModelInfo>,
    shortcuts: Vec<gestures::Shortcut>,
    prompts: Vec<PromptProfile>,
    selected_track: Option<String>,
    selected_profile: Option<String>,
    search: Entity<InputState>,
    title: Entity<InputState>,
    person_name: Entity<InputState>,
    start: Entity<InputState>,
    end: Entity<InputState>,
    edit_text: Entity<InputState>,
    shortcut_key: Entity<InputState>,
    import_path: Entity<InputState>,
    media_url: Entity<InputState>,
    profile_bundle: Entity<InputState>,
    profile_name: Entity<InputState>,
    profile_prompt: Entity<TextareaState>,
    profile_executable: Entity<InputState>,
    profile_arguments: Entity<InputState>,
    prompt_id: Option<String>,
    prompt_route: String,
    status: String,
    pending: usize,
    render_peak_ms: f64,
    render_count: u64,
    segment_page: usize,
    requested_recording: Option<String>,
}
fn anonymous_label(s: &str) -> String {
    let leaf = s.rsplit(':').next().unwrap_or(s);
    leaf.strip_prefix("SPEAKER_")
        .and_then(|s| s.parse::<usize>().ok())
        .map(|n| format!("Speaker {}", n + 1))
        .unwrap_or_else(|| leaf.to_owned())
}
fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    let label: SharedString = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .tab_index(0)
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(0x273645))
        .text_color(rgb(0xe5edf4))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x375369)))
        .child(label)
}
fn field(input: &Entity<InputState>) -> Div {
    div()
        .min_w(px(100.))
        .px_2()
        .py_2()
        .bg(rgb(0x18232c))
        .rounded_md()
        .child(Input::new(input))
}
impl Workbench {
    fn task(
        &mut self,
        label: &str,
        future: impl Future<Output = anyhow::Result<Update>> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.launch(label, Box::pin(future), cx);
    }
    fn launch(
        &mut self,
        label: &str,
        future: std::pin::Pin<Box<dyn Future<Output = anyhow::Result<Update>> + Send + 'static>>,
        cx: &mut Context<Self>,
    ) {
        self.status = label.into();
        self.pending += 1;
        cx.notify();
        let handle = self.runtime.spawn(future);
        cx.spawn(async move |this, cx| {
            let result = handle.await;
            let _ = this.update(cx, |this, cx| {
                this.pending = this.pending.saturating_sub(1);
                match result {
                    Ok(Ok(update)) => {
                        match update {
                            Update::Examples(e) => this.examples = e,
                            Update::Candidates(c) => this.candidates = c,
                            Update::RefreshSelected => {
                                if let Some(id) = this.requested_recording.clone() {
                                    let library = this.library.clone();
                                    this.task(
                                        "Reloading saved corrections",
                                        async move { Ok(Update::Detail(library.show(&id).await?)) },
                                        cx,
                                    );
                                }
                            }
                            Update::Devices(d) => this.devices = d,
                            Update::Captured(result) => {
                                this.capture_stop = None;
                                match result {
                                    Ok(d) => {
                                        this.requested_recording = Some(d.recording.id.clone());
                                        this.selected_track = d
                                            .tracks
                                            .iter()
                                            .find(|t| t.available)
                                            .map(|t| t.id.clone());
                                        if let Some(row) =
                                            this.records.iter_mut().find(|r| r.id == d.recording.id)
                                        {
                                            *row = d.recording.clone();
                                        }
                                        this.detail = Some(d);
                                        this.area = Area::Library;
                                        this.reload(cx);
                                    }
                                    Err(e) => {
                                        this.status = e;
                                        cx.notify();
                                        return;
                                    }
                                }
                            }
                            Update::Library(r) => this.records = r,
                            Update::Detail(d) => {
                                if this.requested_recording.as_deref()
                                    == Some(d.recording.id.as_str())
                                {
                                    this.selected_track = this
                                        .selected_track
                                        .clone()
                                        .filter(|id| d.tracks.iter().any(|t| t.id == *id))
                                        .or_else(|| {
                                            d.tracks
                                                .iter()
                                                .find(|t| {
                                                    t.available && t.kind != TrackKind::Playback
                                                })
                                                .map(|t| t.id.clone())
                                        });
                                    if let Some(row) =
                                        this.records.iter_mut().find(|r| r.id == d.recording.id)
                                    {
                                        *row = d.recording.clone();
                                    }
                                    this.detail = Some(d);
                                }
                            }
                            Update::Profiles(p) => {
                                if this
                                    .selected_profile
                                    .as_ref()
                                    .is_some_and(|id| !p.iter().any(|p| p.id == *id))
                                {
                                    this.selected_profile = None;
                                    this.examples.clear();
                                }
                                this.profiles = p;
                            }
                            Update::Models(m) => this.model_list = m,
                            Update::Dictations(receipts) => this.dictations = receipts,
                            Update::DictationStopped(message) => {
                                this.dictation_stop = None;
                                if let Ok(mut state) = this.dictation_state.lock() {
                                    *state = serde_json::json!({"state":"disabled"});
                                }
                                if let Some(overlay) = this.dictation_overlay.take() {
                                    let _ =
                                        overlay.update(cx, |_, window, _| window.remove_window());
                                }
                                this.status = message;
                                cx.notify();
                                return;
                            }
                            Update::Settings(s, p) => {
                                this.shortcuts = s;
                                this.prompts = p;
                            }
                            Update::Message(m) => {
                                this.status = m;
                                cx.notify();
                                return;
                            }
                        }
                        this.status = "Ready".into();
                    }
                    Ok(Err(e)) => this.status = format!("Failed: {e:#}"),
                    Err(e) => this.status = format!("Background task failed: {e}"),
                };
                cx.notify();
            });
        })
        .detach();
    }
    fn reload(&mut self, cx: &mut Context<Self>) {
        let lib = self.library.clone();
        let search = self.search.read(cx).value().to_string();
        self.task(
            "Loading library",
            async move {
                Ok(Update::Library(
                    lib.list(if search.is_empty() {
                        None
                    } else {
                        Some(&search)
                    })
                    .await?,
                ))
            },
            cx,
        );
    }
    fn open(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.area = Area::Library;
        self.detail = None;
        self.selected_track = None;
        self.segment_page = 0;
        self.requested_recording = Some(id.clone());
        let lib = self.library.clone();
        if let Some(r) = self.records.iter().find(|r| r.id == id) {
            let title = r.title().to_owned();
            self.title
                .update(cx, |s, cx| s.set_value(title, window, cx));
        }
        self.task(
            "Opening recording",
            async move { Ok(Update::Detail(lib.show(&id).await?)) },
            cx,
        );
    }
    fn range(&self, cx: &App) -> Result<(i64, i64), String> {
        let a = self
            .start
            .read(cx)
            .value()
            .parse::<i64>()
            .map_err(|_| "Start must be milliseconds".to_owned())?;
        let b = self
            .end
            .read(cx)
            .value()
            .parse::<i64>()
            .map_err(|_| "End must be milliseconds".to_owned())?;
        if a < 0 || b <= a || b - a > 30_000 {
            return Err("Choose a 1–30 second track-local interval".into());
        }
        Ok((a, b))
    }
    fn play(&mut self, cx: &mut Context<Self>) {
        let Some(d) = &self.detail else {
            return;
        };
        let Some(track) = self.selected_track.clone() else {
            return;
        };
        let (a, b) = match self.range(cx) {
            Ok(r) => r,
            Err(e) => {
                self.status = e;
                cx.notify();
                return;
            }
        };
        let id = d.recording.id.clone();
        let service = self.playback.clone();
        self.task(
            "Playing source interval",
            async move {
                service
                    .play(PlayAudioInput {
                        recording_id: id.to_string(),
                        track_id: track.to_string(),
                        start_ms: a,
                        duration_ms: b - a,
                    })
                    .await?;
                Ok(Update::Message("Playback completed".into()))
            },
            cx,
        );
    }
    fn process(&mut self, diarize: bool, cx: &mut Context<Self>) {
        let Some(d) = &self.detail else {
            return;
        };
        let Some(track) = self.selected_track.clone() else {
            return;
        };
        let id = d.recording.id.clone();
        let service = self.processing.clone();
        let lib = self.library.clone();
        let request = ProcessRequest {
            recording_id: id.clone(),
            track_id: track,
            model: if diarize {
                "selected-diarization"
            } else {
                "selected-asr"
            }
            .into(),
            start_ms: 0,
            duration_ms: None,
        };
        self.task(
            "Processing locally; navigation remains available",
            async move {
                service.run(request).await?;
                Ok(Update::Detail(lib.show(&id).await?))
            },
            cx,
        );
    }
    fn refresh_profiles(&mut self, cx: &mut Context<Self>) {
        let service = self.speakers.clone();
        self.task(
            "Loading speaker profiles",
            async move { Ok(Update::Profiles(service.profiles().await?)) },
            cx,
        );
    }
}
impl Render for Workbench {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let began = Instant::now();
        let sidebar = div()
            .w(px(200.))
            .h_full()
            .bg(rgb(0x17212a))
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_lg().mb_4().child("Parakeet Local"))
            .children(
                [
                    (Area::Library, "Library"),
                    (Area::Capture, "Record"),
                    (Area::Speakers, "Speakers"),
                    (Area::Models, "Local models"),
                    (Area::Dictation, "Dictation"),
                    (Area::Privacy, "Privacy & routes"),
                ]
                .into_iter()
                .enumerate()
                .map(|(i, (area, label))| {
                    button(("nav", i), label).on_click(cx.listener(move |this, _, _, cx| {
                        this.area = area;
                        match area {
                            Area::Capture => {
                                let capture = this.audio_inspection.clone();
                                this.task(
                                    "Enumerating input devices",
                                    async move { Ok(Update::Devices(capture.devices().await?)) },
                                    cx,
                                );
                            }
                            Area::Speakers => {
                                this.refresh_profiles(cx);
                                if let Some(d) = &this.detail {
                                    let id = d.recording.id.clone();
                                    let service = this.speakers.clone();
                                    this.task(
                                        "Preparing clean enrollment candidates",
                                        async move {
                                            Ok(Update::Candidates(service.candidates(&id).await?))
                                        },
                                        cx,
                                    );
                                }
                            }
                            Area::Dictation => {
                                let service = this.dictation_history.clone();
                                this.task(
                                    "Reading retained dictations",
                                    async move { Ok(Update::Dictations(service.list().await?)) },
                                    cx,
                                );
                                let settings = this.settings.clone();
                                this.task(
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
                                let service = this.models.clone();
                                this.task(
                                    "Reading local model registry",
                                    async move { Ok(Update::Models(service.list().await?)) },
                                    cx,
                                );
                            }
                            Area::Privacy => {
                                let service = this.settings.clone();
                                this.task(
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
                    }))
                }),
            );
        let mut content = div()
            .id("area-content")
            .overflow_y_scroll()
            .min_h_0()
            .flex_1()
            .h_full()
            .min_w_0()
            .p_5()
            .flex()
            .flex_col()
            .gap_3();
        match self.area {
            Area::Library => {
                content = content
                    .child(div().flex().gap_2().child(field(&self.import_path)).child(
                        button("import-local", "Import local file or folder").on_click(
                            cx.listener(|this, _, _, cx| {
                                let path =
                                    PathBuf::from(this.import_path.read(cx).value().to_string());
                                let library = this.library.clone();
                                this.task(
                                    "Importing local audio",
                                    async move {
                                        library
                                            .import_path(ImportRecordingsInput {
                                                path,
                                                dry_run: false,
                                            })
                                            .await?;
                                        Ok(Update::Library(library.list(None).await?))
                                    },
                                    cx,
                                );
                            }),
                        ),
                    ))
                    .child(div().flex().gap_2().child(field(&self.media_url)).child(
                        button("import-youtube", "Download & transcribe YouTube").on_click(
                            cx.listener(|this, _, _, cx| {
                                let url = this.media_url.read(cx).value().to_string();
                                let service = this.media.clone();
                                let library = this.library.clone();
                                this.task(
                                    "Downloading and transcribing YouTube locally",
                                    async move {
                                        let r = service
                                            .transcribe(TranscribeMediaInput {
                                                url,
                                                model: "selected-asr".into(),
                                            })
                                            .await?;
                                        if let Some(e) = r.transcription_failure {
                                            return Ok(Update::Message(format!(
                                                "Audio saved as {}. Transcription failed: {e}",
                                                r.recording.id
                                            )));
                                        }
                                        Ok(Update::Library(library.list(None).await?))
                                    },
                                    cx,
                                );
                            }),
                        ),
                    ));
                content = content.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(div().flex_1().child(field(&self.search)))
                        .child(
                            button("search", "Search")
                                .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                        )
                        .child(
                            button("refresh", "Refresh")
                                .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                        ),
                );
                let mut rows = div()
                    .w(px(280.))
                    .h_full()
                    .id("library-list")
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2();
                for r in &self.records {
                    let id = r.id.clone();
                    rows = rows.child(button(r.id.clone(), r.title().to_owned()).on_click(
                        cx.listener(move |this, _, window, cx| this.open(id.clone(), window, cx)),
                    ));
                }
                let mut reader = div().flex_1().min_w_0().h_full().flex().flex_col().gap_3();
                if let Some(d) = &self.detail {
                    let id = d.recording.id.clone();
                    reader = reader
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(div().flex_1().child(field(&self.title)))
                                .child(button("rename", "Save title").on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        let title = this.title.read(cx).value().to_string();
                                        let id = id.clone();
                                        let lib = this.library.clone();
                                        this.task(
                                            "Saving title",
                                            async move {
                                                lib.rename(RenameRecordingInput {
                                                    id: id.clone(),
                                                    title: title.clone(),
                                                })
                                                .await?;
                                                Ok(Update::Detail(lib.show(&id).await?))
                                            },
                                            cx,
                                        );
                                    },
                                ))),
                        )
                        .child(div().text_sm().text_color(rgb(0x99aebc)).child(format!(
                                "{} · {} · {} · {} processing runs",
                                d.recording.source_kind,
                                d.recording.created_at,
                                d.recording.metadata["state"]
                                    .as_str()
                                    .unwrap_or("imported evidence")
                                    .replace('_', " "),
                                d.runs.len()
                            )));
                    for run in d.runs.iter().rev().take(3) {
                        reader = reader.child(
                            div()
                                .text_sm()
                                .child(format!("{} · {} · {}", run.kind, run.model, run.status))
                                .when_some(run.error.clone(), |row, error| row.child(error)),
                        );
                    }
                    if !d.unmapped_corrected_segments.is_empty() {
                        reader=reader.child(format!("{} corrected intervals belong to earlier processing runs; preserved for review",d.unmapped_corrected_segments.len()));
                    }
                    let mut sources = div().flex().gap_2().flex_wrap();
                    for t in &d.tracks {
                        let id = t.id.clone();
                        sources = sources.child(
                            button(
                                t.id.clone(),
                                format!(
                                    "{}{:?} · {} · {}",
                                    if self.selected_track.as_ref() == Some(&t.id) {
                                        "● "
                                    } else {
                                        ""
                                    },
                                    t.kind,
                                    if t.available { "available" } else { "MISSING" },
                                    if t.meeting_offset_ms().is_some() {
                                        "aligned"
                                    } else {
                                        "timing uncertain"
                                    }
                                ),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.selected_track = Some(id.clone());
                                    cx.notify();
                                },
                            )),
                        );
                    }
                    reader = reader
                        .child(sources)
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(field(&self.start))
                                .child(field(&self.end))
                                .child(
                                    button("play", "Play interval")
                                        .on_click(cx.listener(|this, _, _, cx| this.play(cx))),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(button("transcribe", "Transcribe selected source").on_click(
                                    cx.listener(|this, _, _, cx| this.process(false, cx)),
                                ))
                                .child(button("diarize", "Diarize selected source").on_click(
                                    cx.listener(|this, _, _, cx| this.process(true, cx)),
                                )),
                        );
                    let mut transcript = div()
                        .flex_1()
                        .min_h_0()
                        .id("transcript")
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_3();
                    for s in d.segments.iter().skip(self.segment_page * 40).take(40) {
                        let correction = d.corrections.iter().rev().find(|c| c.segment_id == s.id);
                        let words = correction
                            .and_then(|c| c.replacement_text.clone())
                            .unwrap_or_else(|| s.text.clone());
                        let label = correction
                            .and_then(|c| c.profile_id.as_ref())
                            .and_then(|id| self.profiles.iter().find(|p| p.id == *id))
                            .map(|p| format!("{} (confirmed)", p.name))
                            .unwrap_or_else(|| "Unknown".into());
                        let attribution = d.attributions.iter().find(|a| a.segment_id == s.id);
                        let label = if label == "Unknown" {
                            match attribution {
                                Some(a) if a.state == "anonymous_model_attribution" => {
                                    format!(
                                        "Anonymous {}",
                                        a.anonymous_speakers
                                            .iter()
                                            .map(|s| anonymous_label(s))
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    )
                                }
                                Some(a) if a.state == "ambiguous" => {
                                    "Unknown (multiple voices / turn boundary)".into()
                                }
                                _ => label,
                            }
                        } else {
                            label
                        };
                        let time = s
                            .start_ms
                            .map(|v| format!("{:.2}s", v as f64 / 1000.))
                            .unwrap_or_else(|| "Untimed".into());
                        let overlap_note = attribution
                            .map(|a| {
                                if a.possible_cross_source_duplicate {
                                    " · possible cross-source duplication"
                                } else if a.overlapping_source_audio || a.overlapping_speech {
                                    " · overlapping audio"
                                } else {
                                    ""
                                }
                            })
                            .unwrap_or("");
                        let source = s
                            .track_id
                            .as_ref()
                            .and_then(|id| d.tracks.iter().find(|t| t.id == *id))
                            .map(|t| format!("{:?}", t.kind))
                            .unwrap_or_else(|| "source unverified".into());
                        let segment_id = s.id.clone();
                        let play_segment = s.id.clone();
                        let play_recording = d.recording.id.clone();
                        let unknown_id = s.id.clone();
                        let edit_id = s.id.clone();
                        let undo_id = s.id.clone();
                        transcript = transcript.child(
                            div()
                                .p_3()
                                .rounded_md()
                                .bg(rgb(0x1c2933))
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(div().text_sm().text_color(rgb(0x91b6c9)).child(format!(
                                    "{time} · {source} · {label}{overlap_note}{}",
                                    if s.evidence["uncertain_speech"] == true {
                                        " · uncertain speech"
                                    } else {
                                        ""
                                    }
                                )))
                                .child(words)
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(
                                            button(
                                                format!("play-segment-{}", s.id),
                                                "Play speech (up to 30s)",
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    let service = this.playback.clone();
                                                    let recording_id = play_recording.clone();
                                                    let segment_id = play_segment.clone();
                                                    this.task(
                                                        "Playing source-linked speech",
                                                        async move {
                                                            let played = service
                                                                .play_segment(PlaySegmentInput {
                                                                    recording_id,
                                                                    segment_id,
                                                                })
                                                                .await?;
                                                            Ok(Update::Message(
                                                            if played.truncated_to_30_seconds {
                                                                "Played first 30 seconds of segment"
                                                            } else {
                                                                "Playback complete"
                                                            }
                                                            .into(),
                                                        ))
                                                        },
                                                        cx,
                                                    );
                                                }),
                                            ),
                                        )
                                        .child(
                                            button(
                                                format!("assign-{}", s.id),
                                                "Assign selected person",
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    if let Some(profile) =
                                                        this.selected_profile.clone()
                                                    {
                                                        let service = this.speakers.clone();
                                                        let sid = segment_id.clone();
                                                        this.task(
                                                            "Saving confirmed attribution",
                                                            async move {
                                                                service
                                                                    .correct(CorrectSegmentInput {
                                                                        segment_id: sid.to_string(),
                                                                        profile_id: Some(profile),
                                                                        text: None,
                                                                        action: ("assign")
                                                                            .to_string(),
                                                                    })
                                                                    .await?;
                                                                Ok(Update::RefreshSelected)
                                                            },
                                                            cx,
                                                        );
                                                    }
                                                }),
                                            ),
                                        )
                                        .child(
                                            button(format!("unknown-{}", s.id), "Unknown")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    let service = this.speakers.clone();
                                                    let sid = unknown_id.clone();
                                                    this.task(
                                                        "Clearing attribution",
                                                        async move {
                                                            service
                                                                .correct(CorrectSegmentInput {
                                                                    segment_id: sid.to_string(),
                                                                    profile_id: None,
                                                                    text: None,
                                                                    action: ("unknown").to_string(),
                                                                })
                                                                .await?;
                                                            Ok(Update::RefreshSelected)
                                                        },
                                                        cx,
                                                    );
                                                })),
                                        )
                                        .child(
                                            button(format!("edit-{}", s.id), "Use correction text")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    let service = this.speakers.clone();
                                                    let sid = edit_id.clone();
                                                    let text =
                                                        this.edit_text.read(cx).value().to_string();
                                                    this.task(
                                                        "Saving text correction",
                                                        async move {
                                                            service
                                                                .correct(CorrectSegmentInput {
                                                                    segment_id: sid.to_string(),
                                                                    profile_id: None,
                                                                    text: Some(text),
                                                                    action: ("text").to_string(),
                                                                })
                                                                .await?;
                                                            Ok(Update::RefreshSelected)
                                                        },
                                                        cx,
                                                    );
                                                })),
                                        )
                                        .child(button(format!("undo-{}", s.id), "Undo").on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                let service = this.speakers.clone();
                                                let sid = undo_id.clone();
                                                this.task(
                                                    "Undoing correction",
                                                    async move {
                                                        service.undo(&sid).await?;
                                                        Ok(Update::RefreshSelected)
                                                    },
                                                    cx,
                                                );
                                            }),
                                        )),
                                ),
                        )
                    }
                    reader = reader
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(button("previous-segments", "Previous").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.segment_page = this.segment_page.saturating_sub(1);
                                        cx.notify();
                                    }),
                                ))
                                .child(format!(
                                    "{} segments · page {}",
                                    d.segments.len(),
                                    self.segment_page + 1
                                ))
                                .child(button("next-segments", "Next").on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if this.detail.as_ref().is_some_and(|d| {
                                            (this.segment_page + 1) * 40 < d.segments.len()
                                        }) {
                                            this.segment_page += 1;
                                        }
                                        cx.notify();
                                    },
                                ))),
                        )
                        .child(field(&self.edit_text))
                        .child(transcript);
                } else {
                    reader=reader.child("Choose a recording. Imported labels remain unverified; source audio never implies a named person.");
                }
                content = content.child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .gap_4()
                        .child(rows)
                        .child(reader),
                );
            }
            Area::Speakers => {
                content=content.child(div().text_xl().child("Verified speaker examples")).child("Choose a person, return to a recording, play a clean 3–30 second track-local interval, then enroll it here.")
     .child(div().flex().gap_2().child(field(&self.person_name)).child(button("add-person","Create person").on_click(cx.listener(|this,_,_,cx|{let name=this.person_name.read(cx).value().to_string();let service=this.speakers.clone();this.task("Creating speaker profile",async move{service.create(&name).await?;Ok(Update::Profiles(service.profiles().await?))},cx);}))));
                for p in &self.profiles {
                    let id = p.id.clone();
                    content = content.child(
                        button(
                            p.id.clone(),
                            format!(
                                "{}{}",
                                if self.selected_profile.as_ref() == Some(&p.id) {
                                    "● "
                                } else {
                                    ""
                                },
                                p.name
                            ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected_profile = Some(id.clone());
                            cx.notify();
                        })),
                    );
                }
                content = content.child(
                    button("enroll", "I verified this voice — enroll selected interval").on_click(
                        cx.listener(|this, _, _, cx| {
                            let (Some(d), Some(track), Some(profile)) = (
                                &this.detail,
                                this.selected_track.clone(),
                                this.selected_profile.clone(),
                            ) else {
                                this.status = "Select a recording, source, and person first".into();
                                cx.notify();
                                return;
                            };
                            let (a, b) = match this.range(cx) {
                                Ok(r) => r,
                                Err(e) => {
                                    this.status = e;
                                    cx.notify();
                                    return;
                                }
                            };
                            let service = this.speakers.clone();
                            let input = Enrollment {
                                profile_id: profile,
                                recording_id: d.recording.id.clone(),
                                track_id: track,
                                start_ms: a,
                                end_ms: b,
                                confirmed: true,
                            };
                            this.task(
                                "Extracting local verified embedding",
                                async move {
                                    service.enroll(input).await?;
                                    Ok(Update::Message("Verified example saved on-device".into()))
                                },
                                cx,
                            );
                        }),
                    ),
                );
                content = content.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(button("rename-person", "Rename selected person").on_click(
                            cx.listener(|this, _, _, cx| {
                                if let Some(id) = this.selected_profile.clone() {
                                    let name = this.person_name.read(cx).value().to_string();
                                    let service = this.speakers.clone();
                                    this.task(
                                        "Renaming person",
                                        async move {
                                            service
                                                .rename(RenameSpeakerInput {
                                                    id: id.clone(),
                                                    name: name.clone(),
                                                })
                                                .await?;
                                            Ok(Update::Profiles(service.profiles().await?))
                                        },
                                        cx,
                                    );
                                }
                            }),
                        ))
                        .child(
                            button("delete-person", "Delete selected person and examples")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(id) = this.selected_profile.clone() {
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
                                })),
                        ),
                );
                for (index, candidate) in self.candidates.iter().enumerate() {
                    let c = candidate.clone();
                    content = content.child(
                        button(
                            ("candidate", index),
                            format!(
                                "Prepare {} · {:.1}–{:.1}s",
                                anonymous_label(&candidate.anonymous_speaker),
                                candidate.local_start_ms as f64 / 1000.,
                                candidate.local_end_ms as f64 / 1000.
                            ),
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.selected_track = Some(c.track_id.clone());
                                this.start.update(cx, |s, cx| {
                                    s.set_value(c.local_start_ms.to_string(), window, cx)
                                });
                                this.end.update(cx, |s, cx| {
                                    s.set_value(c.local_end_ms.to_string(), window, cx)
                                });
                                cx.notify();
                            },
                        )),
                    );
                }
                content = content.child(
                    button("play-enrollment", "Play prepared interval")
                        .on_click(cx.listener(|this, _, _, cx| this.play(cx))),
                );
                for (index, example) in self.examples.iter().enumerate() {
                    let e = example.clone();
                    let remove = example.id.clone();
                    let profile = example.profile_id.clone();
                    content =
                        content.child(
                            div()
                                .flex()
                                .gap_2()
                                .child(format!(
                                    "{} · {} · {}–{}ms",
                                    example.model,
                                    example.capture_domain,
                                    example.start_ms,
                                    example.end_ms
                                ))
                                .child(button(("example-play", index), "Play").on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        let player = this.playback.clone();
                                        let e = e.clone();
                                        this.task(
                                            "Playing verified reference",
                                            async move {
                                                player
                                                    .play(PlayAudioInput {
                                                        recording_id: e.recording_id.to_string(),
                                                        track_id: e.track_id.to_string(),
                                                        start_ms: e.start_ms,
                                                        duration_ms: e.end_ms - e.start_ms,
                                                    })
                                                    .await?;
                                                Ok(Update::Message(
                                                    "Reference playback completed".into(),
                                                ))
                                            },
                                            cx,
                                        );
                                    }),
                                ))
                                .child(
                                    button(("example-remove", index), "Remove incorrect example")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let service = this.speakers.clone();
                                            let id = remove.clone();
                                            let profile = profile.clone();
                                            this.task(
                                                "Removing voice example",
                                                async move {
                                                    service.remove_example(&id).await?;
                                                    Ok(Update::Examples(
                                                        service.examples(Some(&profile)).await?,
                                                    ))
                                                },
                                                cx,
                                            );
                                        })),
                                ),
                        );
                }
            }
            Area::Models => {
                content = content
                    .child(div().text_xl().child("Local model registry"))
                    .child("Install deliberately. Processing never downloads weights.");
                if let Some(progress) = self.models.download_progress() {
                    content = content.child(format!(
                        "Downloading {} · file {}/{} · {:.1} / {} MB",
                        progress.model,
                        progress.file_index,
                        progress.total_files,
                        progress.downloaded_bytes as f64 / 1_000_000.,
                        progress
                            .total_bytes
                            .map(|n| format!("{:.1}", n as f64 / 1_000_000.))
                            .unwrap_or_else(|| "unknown".into())
                    ));
                }
                for m in &self.model_list {
                    let id = m.id.clone();
                    let verify_id = m.id.clone();
                    let select_id = m.id.clone();
                    let embedding_id = m.id.clone();
                    let remove_id = m.id.clone();
                    content = content.child(
                        div()
                            .p_3()
                            .bg(rgb(0x1c2933))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(format!(
                                "{} · {} · {}",
                                m.id,
                                m.category,
                                if m.verified {
                                    "verified files"
                                } else {
                                    "not installed/verified"
                                }
                            ))
                            .child(format!("Revision {} · {}", m.revision, m.license))
                            .child(format!(
                                "Selected for processing: {} · Voice embeddings: {}",
                                m.detail["selected"] == true,
                                m.detail["selected_for_embeddings"] == true
                            ))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        button(format!("select-{}", m.id), "Use for processing")
                                            .on_click(cx.listener(move |this, _, _, cx| {
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
                                            })),
                                    )
                                    .when(m.detail["provides_voice_embeddings"] == true, |row| {
                                        row.child(
                                            button(
                                                format!("embedding-{}", m.id),
                                                "Use for voice examples",
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    let service = this.models.clone();
                                                    let id = embedding_id.clone();
                                                    this.task(
                                                        "Selecting embedding model",
                                                        async move {
                                                            service.select_embedding(&id).await?;
                                                            Ok(Update::Models(
                                                                service.list().await?,
                                                            ))
                                                        },
                                                        cx,
                                                    );
                                                }),
                                            ),
                                        )
                                    })
                                    .child(
                                        button(format!("remove-model-{}", m.id), "Remove weights")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                let service = this.models.clone();
                                                let id = remove_id.clone();
                                                this.task(
                                                    "Removing downloaded model weights",
                                                    async move {
                                                        service.remove(&id).await?;
                                                        Ok(Update::Models(service.list().await?))
                                                    },
                                                    cx,
                                                );
                                            })),
                                    )
                                    .child(button(format!("install-{}", m.id), "Install").on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            let service = this.models.clone();
                                            let id = id.clone();
                                            this.task(
                                                "Downloading local model",
                                                async move {
                                                    service.install(&id).await?;
                                                    Ok(Update::Models(service.list().await?))
                                                },
                                                cx,
                                            );
                                        }),
                                    ))
                                    .child(
                                        button(format!("verify-{}", m.id), "Verify checksums")
                                            .on_click(cx.listener(move |this, _, _, cx| {
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
                                            })),
                                    ),
                            ),
                    );
                }
            }
            Area::Capture => {
                content=content.child(div().text_xl().child("Meeting capture")).child("Select a physical input explicitly. System audio is captured independently. Stop preserves separate source recordings.");
                if let Some(devices) = self.devices["devices"].as_array() {
                    for (index, device) in devices.iter().enumerate() {
                        let uid = device["uid"].as_str().unwrap_or_default().to_owned();
                        let label = format!(
                            "{}{}{}",
                            if self.audio_preferences.microphone_uid.as_ref() == Some(&uid) {
                                "● "
                            } else {
                                ""
                            },
                            device["name"].as_str().unwrap_or("Unknown input"),
                            if device["is_default"] == true {
                                " (macOS default)"
                            } else {
                                ""
                            }
                        );
                        content = content.child(button(("device", index), label).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.audio_preferences.microphone_uid = Some(uid.clone());
                                let preferences = this.audio_preferences.clone();
                                let service = this.audio_settings.clone();
                                this.task(
                                    "Saving explicit microphone",
                                    async move {
                                        service.save(&preferences).await?;
                                        Ok(Update::Message("Selected microphone saved".into()))
                                    },
                                    cx,
                                );
                            }),
                        ));
                    }
                }
                content = content
                    .child(format!(
                        "Selected input UID: {}",
                        self.audio_preferences
                            .microphone_uid
                            .as_deref()
                            .unwrap_or("None — choose a microphone")
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(button("record-both", "Record mic + system").on_click(
                                cx.listener(|this, _, _, cx| this.start_capture(true, true, cx)),
                            ))
                            .child(button("record-system", "Record system only").on_click(
                                cx.listener(|this, _, _, cx| this.start_capture(false, true, cx)),
                            ))
                            .child(button("record-mic", "Record mic only").on_click(
                                cx.listener(|this, _, _, cx| this.start_capture(true, false, cx)),
                            ))
                            .child(
                                button("stop-recording", "Stop & save").on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if let Some(stop) = &this.capture_stop {
                                            stop.store(true, Ordering::SeqCst);
                                            this.status = "Stopping and finalizing sources".into();
                                            cx.notify();
                                        }
                                    },
                                )),
                            ),
                    )
                    .child(format!(
                        "Capture state: {}",
                        self.capture.status()["state"].as_str().unwrap_or("unknown")
                    ));
                let health = self.capture.status();
                if let Some(message) = health["message"].as_str() {
                    content = content.child(format!("Capture warning: {message}"));
                }
                if let Some(uid) = health["effective_microphone_uid"].as_str() {
                    content = content.child(format!("Actual effective microphone UID: {uid}"));
                }
                for source in ["microphone", "system"] {
                    let h = &health[source];
                    if !h.is_null() {
                        content = content.child(format!(
                            "{source}: {} · {} frames · {} Hz · peak {:.3}",
                            if !matches!(
                                health["state"].as_str(),
                                Some(
                                    "starting"
                                        | "recording"
                                        | "waiting_or_stalled"
                                        | "source_failed"
                                )
                            ) {
                                "not capturing"
                            } else if (source == "microphone"
                                && health["requested_microphone_uid"].is_null())
                                || (source == "system" && health["requested_system_audio"] != true)
                            {
                                "not requested"
                            } else if h["is_receiving"] == true {
                                "receiving audio"
                            } else if h["first_buffer_ready"] == true {
                                "stalled — inspect source"
                            } else {
                                "waiting for first buffer"
                            },
                            h["written_frames"],
                            h["sample_rate"],
                            h["peak_amplitude"].as_f64().unwrap_or(0.)
                        ));
                    }
                }
            }
            Area::Dictation => {
                let live = self
                    .dictation_state
                    .lock()
                    .ok()
                    .map(|v| v.clone())
                    .unwrap_or_default();
                if let Some(message) = live["message"].as_str() {
                    content = content.child(format!("Dictation error: {message}"));
                }
                if let Some(raw) = live["receipt"]["raw"].as_str().filter(|s| !s.is_empty()) {
                    let text = raw.to_owned();
                    content = content.child(format!("Latest raw speech: {raw}")).child(
                        button("copy-live-dictation", "Copy latest raw text").on_click(
                            cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                            }),
                        ),
                    );
                }
                content = content.child(
                    button("refresh-dictations", "Refresh retained dictations").on_click(
                        cx.listener(|this, _, _, cx| {
                            let service = this.dictation_history.clone();
                            this.task(
                                "Reading retained dictations",
                                async move { Ok(Update::Dictations(service.list().await?)) },
                                cx,
                            );
                        }),
                    ),
                );
                for (index, receipt) in self.dictations.iter().take(10).enumerate() {
                    let raw = receipt.raw.clone();
                    content = content.child(
                        div()
                            .p_2()
                            .bg(rgb(0x18232c))
                            .child(format!(
                                "{} · {} · {}",
                                receipt.created_at, receipt.target.bundle_id, receipt.status
                            ))
                            .child(receipt.raw.clone())
                            .when_some(receipt.failure.clone(), |row, error| row.child(error))
                            .child(button(("copy-retained", index), "Copy raw text").on_click(
                                cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(raw.clone()))
                                }),
                            )),
                    );
                }
                content=content.child(div().text_xl().child("Simultaneous hold shortcuts")).child("Bindings persist across keyboards. Native event delivery requires Input Monitoring permission.");
                content = content
                    .child(format!(
                        "Dictation state: {}",
                        self.dictation_state
                            .lock()
                            .ok()
                            .and_then(|v| v["state"].as_str().map(str::to_owned))
                            .unwrap_or_else(|| "idle".into())
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                button("start-listening", "Enable hold dictation").on_click(
                                    cx.listener(|this, _, _, cx| this.start_dictation(cx)),
                                ),
                            )
                            .child(button("stop-listening", "Disable hold dictation").on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Some(stop) = this.dictation_stop.take() {
                                        stop.store(true, Ordering::SeqCst);
                                    }
                                    cx.notify();
                                }),
                            )),
                    );
                content = content.child(button("record-shortcut", "Record a shortcut").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.recording_shortcut = true;
                        this.status = "Press the shortcut; Escape cancels".into();
                        cx.notify();
                    }),
                ));
                for (index, binding) in self.shortcuts.iter().enumerate() {
                    let toggle = binding.id.clone();
                    let remove = binding.id.clone();
                    content = content.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(format!(
                                "{} {:?} · {}",
                                binding.key,
                                binding.modifiers,
                                if binding.enabled {
                                    "active"
                                } else {
                                    "disabled"
                                }
                            ))
                            .child(
                                button(("toggle-binding", index), "Enable / disable").on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        let mut bindings = this.shortcuts.clone();
                                        if let Some(b) =
                                            bindings.iter_mut().find(|b| b.id == toggle)
                                        {
                                            b.enabled = !b.enabled;
                                        }
                                        this.persist_shortcuts(bindings, cx);
                                    }),
                                ),
                            )
                            .child(button(("remove-binding", index), "Remove").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    let bindings = this
                                        .shortcuts
                                        .iter()
                                        .filter(|b| b.id != remove)
                                        .cloned()
                                        .collect();
                                    this.persist_shortcuts(bindings, cx);
                                }),
                            )),
                    );
                }
                content =
                    content.child(div().flex().gap_2().child(field(&self.shortcut_key)).child(
                        button("add-shortcut", "Add shortcut").on_click(cx.listener(
                            |this, _, _, cx| {
                                let key = this.shortcut_key.read(cx).value().to_string();
                                let mut bindings = this.shortcuts.clone();
                                bindings.push(gestures::Shortcut {
                                    id: new_id(),
                                    key,
                                    modifiers: vec![],
                                    enabled: true,
                                });
                                this.persist_shortcuts(bindings, cx);
                            },
                        )),
                    ));
            }
            Area::Privacy => {
                content=content.child(div().text_xl().child("Application prompts"))
                    .child("Raw transcription is always retained. Saving a draft disables its route. Only an explicit authorization enables execution for that exact application.");
                for (index, p) in self.prompts.iter().enumerate() {
                    let profile = p.clone();
                    let revoke = p.clone();
                    content = content.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                button(
                                    ("edit-prompt", index),
                                    format!(
                                        "Edit {} · {} · authorized {}",
                                        p.application_name, p.route, p.authorized
                                    ),
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.profile_prompt.update(cx, |state, cx| {
                                            state.set_value(profile.prompt.clone(), window, cx)
                                        });
                                        this.prompt_id = Some(profile.id.clone());
                                        this.prompt_route = profile.route.clone();
                                        for (entity, value) in [
                                            (&this.profile_bundle, profile.bundle_id.clone()),
                                            (&this.profile_name, profile.application_name.clone()),
                                            (&this.profile_executable, profile.executable.clone()),
                                            (
                                                &this.profile_arguments,
                                                serde_json::to_string(&profile.arguments)
                                                    .unwrap_or_default(),
                                            ),
                                        ] {
                                            entity
                                                .update(cx, |s, cx| s.set_value(value, window, cx));
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(button(("revoke-prompt", index), "Revoke").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    let mut p = revoke.clone();
                                    p.authorized = false;
                                    p.enabled = false;
                                    let service = this.settings.clone();
                                    this.task(
                                        "Revoking AI route",
                                        async move {
                                            service.save_prompt(&p).await?;
                                            Ok(Update::Settings(
                                                service.shortcuts().await?,
                                                service.prompts().await?,
                                            ))
                                        },
                                        cx,
                                    );
                                }),
                            )),
                    );
                }
                content=content.child(button("new-prompt","New application profile").on_click(cx.listener(|this,_,window,cx|{
                    this.profile_prompt.update(cx,|state,cx|state.set_value("",window,cx));
                    this.prompt_id=None;this.prompt_route="remote".into();
                    for entity in [&this.profile_bundle,&this.profile_name,&this.profile_executable] { entity.update(cx,|s,cx|s.set_value("",window,cx)); }
                    this.profile_arguments.update(cx,|s,cx|s.set_value("[]",window,cx));cx.notify();
                })))
                .child("Application bundle identifier").child(field(&self.profile_bundle))
                .child("Application name").child(field(&self.profile_name))
                .child("Your transformation prompt").child(div().p_2().bg(rgb(0x18232c)).child(Textarea::new(&self.profile_prompt)))
                .child("AI CLI executable (absolute path)").child(field(&self.profile_executable))
                .child("CLI arguments (JSON array, no shell expansion)").child(field(&self.profile_arguments))
                .child(button("prompt-route",format!("Route: {} · click to change",self.prompt_route)).on_click(cx.listener(|this,_,_,cx|{this.prompt_route=if this.prompt_route=="remote"{"local"}else{"remote"}.into();cx.notify();})))
                .child("Declare local only if you have verified that the CLI runs inference on this device. A remote route may send your transcript to its provider.")
                .child(div().flex().gap_2()
                    .child(button("save-prompt-draft","Save disabled draft").on_click(cx.listener(|this,_,_,cx|this.save_prompt(false,cx))))
                    .child(button("authorize-prompt","Authorize this route & save").on_click(cx.listener(|this,_,_,cx|this.save_prompt(true,cx)))));
            }
        }
        self.render_count += 1;
        self.render_peak_ms = self
            .render_peak_ms
            .max(began.elapsed().as_secs_f64() * 1000.);
        div()
            .id("workbench-root")
            .role(Role::Application)
            .aria_label("Parakeet Local")
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &NextFocus, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &PreviousFocus, window, cx| window.focus_prev(cx)))
            .on_action(cx.listener(|this, _: &SearchLibrary, window, cx| {
                this.area = Area::Library;
                this.search.focus_handle(cx).focus(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &Quit, _, cx| cx.quit()))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if !this.recording_shortcut {
                    return;
                }
                cx.stop_propagation();
                this.recording_shortcut = false;
                if event.keystroke.key == "escape" {
                    cx.notify();
                    return;
                }
                let mut modifiers = vec![];
                let m = event.keystroke.modifiers;
                for (active, name) in [
                    (m.control, "ctrl"),
                    (m.alt, "alt"),
                    (m.shift, "shift"),
                    (m.platform, "cmd"),
                ] {
                    if active {
                        modifiers.push(name.into());
                    }
                }
                let mut bindings = this.shortcuts.clone();
                bindings.push(gestures::Shortcut {
                    id: new_id(),
                    key: event.keystroke.key.clone(),
                    modifiers,
                    enabled: true,
                });
                this.persist_shortcuts(bindings, cx);
            }))
            .size_full()
            .tab_group()
            .bg(rgb(0x101920))
            .text_color(rgb(0xe5edf4))
            .text_base()
            .flex()
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(content)
                    .child(div().p_3().text_sm().bg(rgb(0x192832)).child(format!(
                        "{} · {} background tasks · render construction peak {:.2} ms",
                        self.status, self.pending, self.render_peak_ms
                    ))),
            )
    }
}
fn input(window: &mut Window, cx: &mut App, value: &str, placeholder: &str) -> Entity<InputState> {
    cx.new(|cx| {
        let mut s = InputState::new(window, cx);
        s.set_value(value.to_owned(), window, cx);
        s.set_placeholder(placeholder.to_owned(), window, cx);
        s
    })
}

actions!(
    parakeet_local,
    [NextFocus, PreviousFocus, SearchLibrary, Quit]
);
struct DictationOverlay {
    state: Arc<Mutex<serde_json::Value>>,
}
impl DictationOverlay {
    fn watch(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                if view.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
    }
}
impl Render for DictationOverlay {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let state = self
            .state
            .lock()
            .ok()
            .map(|s| s.clone())
            .unwrap_or_default();
        let name = state["state"].as_str().unwrap_or("waiting");
        let (label, color) = match name {
            "starting" => ("Starting microphone", 0xdfbc72),
            "listening" => ("Listening", 0x7ed6ad),
            "transcribing" => ("Transcribing locally", 0xdfbc72),
            "inserted" => ("Text inserted", 0x7ed6ad),
            "cancelled" => ("Cancelled", 0xa3b2bf),
            "failed" => ("Dictation failed — open Dictation", 0xef9a91),
            "retained_after_failure" => ("Needs review — open Dictation history", 0xef9a91),
            _ => ("Hold a dictation shortcut", 0xa3b2bf),
        };
        div()
            .id("dictation-feedback")
            .role(Role::Status)
            .aria_label(label)
            .size_full()
            .bg(rgb(0x18232c))
            .text_color(rgb(color))
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .child(label)
    }
}
struct UiDictationObserver(Arc<Mutex<serde_json::Value>>);
impl DictationObserver for UiDictationObserver {
    fn event(&self, event: serde_json::Value) {
        if let Ok(mut state) = self.0.lock() {
            *state = event;
        }
    }
}
impl Workbench {
    fn save_prompt(&mut self, authorized: bool, cx: &mut Context<Self>) {
        let arguments =
            match serde_json::from_str::<Vec<String>>(&self.profile_arguments.read(cx).value()) {
                Ok(v) => v,
                Err(e) => {
                    self.status = format!("Arguments must be a JSON array of strings: {e}");
                    cx.notify();
                    return;
                }
            };
        let id = self.prompt_id.clone().unwrap_or_else(new_id);
        self.prompt_id = Some(id.clone());
        let profile = PromptProfile {
            id,
            bundle_id: self.profile_bundle.read(cx).value().to_string(),
            application_name: self.profile_name.read(cx).value().to_string(),
            prompt: self.profile_prompt.read(cx).value().to_string(),
            executable: self.profile_executable.read(cx).value().to_string(),
            arguments,
            route: self.prompt_route.clone(),
            enabled: authorized,
            authorized,
            timeout_seconds: 60,
        };
        let service = self.settings.clone();
        self.task(
            "Saving application prompt",
            async move {
                service.save_prompt(&profile).await?;
                Ok(Update::Settings(
                    service.shortcuts().await?,
                    service.prompts().await?,
                ))
            },
            cx,
        );
    }
    fn persist_shortcuts(&mut self, bindings: Vec<gestures::Shortcut>, cx: &mut Context<Self>) {
        let service = self.settings.clone();
        if let Some(stop) = self.dictation_stop.take() {
            stop.store(true, Ordering::SeqCst);
        }
        self.task(
            "Saving shortcut bindings",
            async move {
                service.save_shortcuts(&bindings).await?;
                Ok(Update::Settings(
                    service.shortcuts().await?,
                    service.prompts().await?,
                ))
            },
            cx,
        );
    }
    fn start_capture(&mut self, microphone: bool, system_audio: bool, cx: &mut Context<Self>) {
        if self.capture_stop.is_some() {
            self.status = "A capture session is already active".into();
            cx.notify();
            return;
        }
        let microphone_uid = if microphone {
            match self.audio_preferences.microphone_uid.clone() {
                Some(uid) => Some(uid),
                None => {
                    self.status = "Choose a microphone first".into();
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let request = CaptureRequest {
            microphone_uid,
            system_audio,
            title: format!("Meeting {}", now()),
            maximum_seconds: 7200,
        };
        let stop = Arc::new(AtomicBool::new(false));
        self.capture_stop = Some(stop.clone());
        let capture = self.capture.clone();
        let lib = self.library.clone();
        self.task(
            "Recording — waiting for first buffers",
            async move {
                let result = async {
                    let bundle = capture.record(request, stop).await?;
                    Ok::<_, anyhow::Error>(lib.show(&bundle.recording.id).await?)
                }
                .await;
                Ok(Update::Captured(result.map_err(|e| e.to_string())))
            },
            cx,
        );
    }
    fn start_dictation(&mut self, cx: &mut Context<Self>) {
        if self.dictation_stop.is_some() {
            return;
        }
        let Some(uid) = self.audio_preferences.microphone_uid.clone() else {
            self.status = "Select a microphone in Record first".into();
            cx.notify();
            return;
        };
        let listener = self.dictation_listener.clone();
        let stop = Arc::new(AtomicBool::new(false));
        self.dictation_stop = Some(stop.clone());
        if let Ok(mut state) = self.dictation_state.lock() {
            *state = serde_json::json!({"state":"waiting"});
        }
        let state = self.dictation_state.clone();
        let mut bounds = Bounds::centered(None, size(px(360.), px(52.)), cx);
        if let Some(display) = cx.primary_display() {
            let visible = display.visible_bounds();
            bounds.origin.y = visible.origin.y + visible.size.height - px(90.);
        }
        match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                focus: false,
                kind: WindowKind::PopUp,
                is_resizable: false,
                is_minimizable: false,
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("Parakeet dictation status");
                let view = cx.new(|_| DictationOverlay { state });
                view.update(cx, |view, cx| view.watch(cx));
                view
            },
        ) {
            Ok(window) => self.dictation_overlay = Some(window),
            Err(error) => {
                self.status = format!("Cannot show dictation feedback: {error}");
                cx.notify();
            }
        }
        self.task(
            "Listening for configured hold shortcuts",
            async move {
                let result = listener.run(uid, stop).await;
                Ok(Update::DictationStopped(match result {
                    Ok(()) => "Hold dictation disabled".into(),
                    Err(e) => format!("Hold dictation stopped: {e}"),
                }))
            },
            cx,
        );
    }
    fn start_heartbeat(&mut self, cx: &mut Context<Self>) {
        let metrics_path = std::env::var_os("PARAKEET_UI_METRICS").map(PathBuf::from);
        cx.spawn(async move|this,cx|{loop{cx.background_executor().timer(std::time::Duration::from_millis(100)).await;let result=this.update(cx,|this,cx|{let elapsed=this.heartbeat_last.elapsed().as_secs_f64()*1000.;this.heartbeat_last=Instant::now();this.heartbeat_max_ms=this.heartbeat_max_ms.max(elapsed);if this.capture_stop.is_some()||this.dictation_stop.is_some()||(this.area==Area::Models && this.pending>0){cx.notify();}let metrics=serde_json::json!({"max_100ms_heartbeat_interval_ms":this.heartbeat_max_ms,"render_construction_peak_ms":this.render_peak_ms,"render_count":this.render_count,"background_jobs":this.pending});(this.runtime.clone(),metrics)});match result{Ok((runtime,metrics))=>{if let Some(path)=metrics_path.clone(){runtime.spawn(async move{let _=tokio::fs::write(path,metrics.to_string()).await;});}},Err(_)=>break,}}}).detach();
    }
}

fn model_runtime_executable() -> PathBuf {
    std::env::var_os("PARAKEET_MODEL_RUNTIME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let exe = std::env::current_exe().expect("current executable");
            let sibling = exe.parent().unwrap().join("parakeet-model-runtime");
            if sibling.exists() {
                sibling
            } else {
                exe.parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("native/parakeet-model-runtime")
            }
        })
}
fn main() -> anyhow::Result<()> {
    let root = std::env::var_os("PARAKEET_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join("Library/Application Support/ParakeetLocal")
        });
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?,
    );
    let pool = runtime.block_on(parakeet_database::open_existing(&root.join("parakeet.db")))?;
    let store = Arc::new(SqliteStore::new(pool));
    let source = Arc::new(MacParakeetArtifacts);
    let models = Arc::new(ModelFiles::new(root.join("models")));
    let engine = Arc::new(LocalSpeech::new(models.clone(), model_runtime_executable()));
    let library = Arc::new(LibraryService::new(store.clone(), store.clone(), source));
    let speakers = Arc::new(SpeakerService::new(
        store.clone(),
        store.clone(),
        store.clone(),
        engine.clone(),
        models.clone(),
    ));
    let processing = Arc::new(ProcessingService::new(
        store.clone(),
        store.clone(),
        models.clone(),
        engine,
    ));
    let settings = Arc::new(SettingsService::new(
        store.clone(),
        store.clone(),
        Arc::new(AuthorizedCli),
    ));
    let playback = Arc::new(PlaybackService::new(
        store.clone(),
        Arc::new(LocalPlayback {
            root: root.join("playback"),
        }),
    ));
    let bridge_executable = std::env::var_os("PARAKEET_NATIVE_BRIDGE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let exe = std::env::current_exe().unwrap();
            let sibling = exe.parent().unwrap().join("parakeet-bridge");
            if sibling.exists() {
                sibling
            } else {
                exe.parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("native/parakeet-bridge")
            }
        });
    let native = Arc::new(parakeet_infrastructure::native::NativeBridge::new(
        bridge_executable,
        root.clone(),
    ));
    let audio_inspection = Arc::new(AudioInspectionService::new(native.clone()));
    let capture = Arc::new(CaptureService::new(native.clone(), store.clone()));
    let audio_settings = Arc::new(AudioSettingsService::new(store.clone(), store.clone()));
    let audio_preferences = runtime.block_on(audio_settings.get())?;
    let dictation_state = Arc::new(Mutex::new(serde_json::json!({"state":"idle"})));
    let dictation = Arc::new(DictationService::new(
        capture.clone(),
        processing.clone(),
        settings.clone(),
        native.clone(),
        store.clone(),
    ));
    let dictation_history = Arc::new(DictationHistoryService::new(store.clone()));
    let dictation_listener = Arc::new(DictationListener::new(
        native,
        settings.clone(),
        dictation,
        Arc::new(UiDictationObserver(dictation_state.clone())),
    ));
    let media = Arc::new(MediaTranscriptionService::new(
        Arc::new(MediaService::new(
            Arc::new(parakeet_infrastructure::media::YouTubeMedia {
                executable: std::env::var_os("PARAKEET_YT_DLP")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        let bundled = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                            .join("Library/Application Support/MacParakeet/bin/yt-dlp");
                        if bundled.exists() {
                            bundled
                        } else {
                            PathBuf::from("yt-dlp")
                        }
                    }),
                root: root.join("media"),
            }),
            store.clone(),
            store.clone(),
        )),
        processing.clone(),
    ));
    let models = Arc::new(ModelService::new(models));
    let records = runtime.block_on(library.list(None))?;
    let profiles = runtime.block_on(speakers.profiles())?;
    gpui_platform::application().run(move |cx: &mut App| {
        gpui_base::init(cx);
        cx.bind_keys([
            KeyBinding::new("tab", NextFocus, None),
            KeyBinding::new("shift-tab", PreviousFocus, None),
            KeyBinding::new("cmd-f", SearchLibrary, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        let bounds = Bounds::centered(None, size(px(1240.), px(850.)), cx);
        let main_window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(920.), px(640.))),
                    ..Default::default()
                },
                move |window, cx| {
                    window.set_window_title("Parakeet Local");
                    let search = input(window, cx, "", "Search titles and transcript");
                    let title = input(window, cx, "", "Recording title");
                    let person_name = input(window, cx, "", "Person's name");
                    let start = input(window, cx, "0", "Start ms");
                    let end = input(window, cx, "10000", "End ms");
                    let edit_text = input(window, cx, "", "Correction text for chosen segment");
                    let shortcut_key = input(window, cx, "", "Home, F6, etc.");
                    let import_path = input(window, cx, "", "Local audio file or meeting folder");
                    let media_url = input(window, cx, "", "YouTube video URL");
                    let profile_bundle = input(window, cx, "", "com.example.application");
                    let profile_name = input(window, cx, "", "Application name");
                    let profile_prompt = cx.new(|cx| {
                        let mut state = TextareaState::new(window, cx).auto_grow(3, 8);
                        state.set_placeholder("Transformation prompt", window, cx);
                        state
                    });
                    let profile_executable = input(window, cx, "", "/absolute/path/to/cli");
                    let profile_arguments = input(window, cx, "[]", "Arguments");
                    let focus = cx.focus_handle();
                    let view = cx.new(|_| Workbench {
                        runtime,
                        library,
                        media,
                        speakers,
                        models,
                        processing,
                        settings,
                        playback,
                        capture,
                        audio_inspection,
                        audio_settings,
                        dictation_listener,
                        dictation_history,
                        dictations: vec![],
                        capture_stop: None,
                        dictation_stop: None,
                        dictation_overlay: None,
                        dictation_state,
                        devices: serde_json::Value::Null,
                        audio_preferences,
                        focus,
                        heartbeat_last: Instant::now(),
                        heartbeat_max_ms: 0.,
                        area: Area::Library,
                        records,
                        detail: None,
                        profiles,
                        examples: vec![],
                        candidates: vec![],
                        recording_shortcut: false,
                        model_list: vec![],
                        shortcuts: vec![],
                        prompts: vec![],
                        selected_track: None,
                        selected_profile: None,
                        search,
                        title,
                        person_name,
                        start,
                        end,
                        edit_text,
                        shortcut_key,
                        import_path,
                        media_url,
                        profile_bundle,
                        profile_name,
                        profile_prompt,
                        profile_executable,
                        profile_arguments,
                        prompt_id: None,
                        prompt_route: "remote".into(),
                        status: "Ready".into(),
                        pending: 0,
                        render_peak_ms: 0.,
                        render_count: 0,
                        segment_page: 0,
                        requested_recording: None,
                    });
                    view.update(cx, |this, cx| this.start_heartbeat(cx));
                    let quitting = view.downgrade();
                    cx.on_app_quit(move |cx| {
                        let state = quitting
                            .update(cx, |this, _| {
                                let active = this.capture_stop.is_some();
                                if let Some(stop) = &this.capture_stop {
                                    stop.store(true, Ordering::SeqCst);
                                }
                                if let Some(stop) = &this.dictation_stop {
                                    stop.store(true, Ordering::SeqCst);
                                }
                                (active, this.runtime.clone(), this.capture.clone())
                            })
                            .ok();
                        async move {
                            if let Some((true, runtime, capture)) = state {
                                let _ = runtime
                                    .spawn(async move {
                                        let began = Instant::now();
                                        while began.elapsed() < std::time::Duration::from_secs(12) {
                                            if capture.status()["event"] == "completed"
                                                || capture.status()["event"] == "failed"
                                            {
                                                break;
                                            }
                                            tokio::time::sleep(std::time::Duration::from_millis(
                                                100,
                                            ))
                                            .await;
                                        }
                                    })
                                    .await;
                            }
                        }
                    })
                    .detach();
                    cx.new(|cx| gpui_base::Root::new(view, window, cx))
                },
            )
            .expect("open Parakeet Local window");
        let main_id = AnyWindowHandle::from(main_window).window_id();
        cx.on_window_closed(move |cx, id| {
            if id == main_id {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
    Ok(())
}
