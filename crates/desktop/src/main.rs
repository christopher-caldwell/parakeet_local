mod activity;
mod analysis;
mod job_control;
mod library;
#[cfg(feature = "performance")]
mod performance;
mod reader;
mod screens;
mod shell;
mod storage;
mod train;
mod ui;
use gpui::{div, prelude::*, px, rgb, size, *};
use gpui_base::input::{Input, InputEvent, InputState, Textarea, TextareaState};
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
use ui::*;

#[derive(Clone, Copy, PartialEq)]
enum Area {
    Library,
    Speakers,
    Models,
    Dictation,
    Privacy,
    Capture,
    Jobs,
    Storage,
}
#[derive(Clone, Copy, PartialEq)]
enum SpeakerMode {
    Profiles,
    Clusters,
    Comparisons,
}
enum Update {
    Clusters(parakeet_application::cluster_edits::ClusterReview),
    CopyText(String),
    LibrarySearch(u64, Vec<Recording>),
    TitleSaved(reader::Document),
    Imported(ImportReport),
    Detail(reader::Document),
    Profiles(Vec<SpeakerProfile>),
    Examples(String, Vec<VoiceExampleSummary>),
    Candidates(
        String,
        u64,
        Result<(Vec<EnrollmentCandidate>, Vec<ClusterIdentification>, bool), String>,
    ),
    Permissions(Result<serde_json::Value, String>),
    DictationReady(Result<(serde_json::Value, serde_json::Value), String>),
    Identified(
        Box<(
            String,
            ClusterIdentification,
            Vec<SpeakerProfile>,
            reader::Document,
        )>,
    ),
    IdentificationUndone(UndoIdentificationReport),
    Calibration(parakeet_domain::calibration::CalibrationReport),
    Preferences(AudioPreferences, String),
    RefreshSelected,
    SegmentSaved(String),
    Models(Vec<ModelInfo>),
    DictationStopped(Result<(), String>),
    Dictations(Vec<DictationReceipt>),
    Message(String),
    Jobs(Vec<ProcessingRun>),
    StorageUsage(Result<Arc<parakeet_application::storage::StorageUsage>, String>),
    Recovered(
        parakeet_application::jobs::ProcessingRecoveryReport,
        Vec<ProcessingRun>,
    ),
    Suggestions(String, Vec<SpeakerSuggestion>),
    SuggestionReview(SpeakerSuggestionReview),
    Settings(Vec<gestures::Shortcut>, Vec<PromptProfile>),
    Devices(serde_json::Value),
    /// The meeting audio is saved; `true` when its transcription still has to be finished.
    Captured(Result<reader::Document, String>, Option<String>, bool),
    /// Transcription and speaker work after Stop ended for this recording.
    MeetingFinished(String, Result<reader::Document, String>, Option<String>),
    MeetingFinalized(Result<reader::Document, String>, Option<String>),
    MeetingPlans(Vec<parakeet_application::meeting::MeetingFinalization>),
}
struct Workbench {
    #[cfg(feature = "performance")]
    performance: Option<performance::Recorder>,
    runtime: Arc<Runtime>,
    library: Arc<LibraryService>,
    locations: Arc<parakeet_application::audio_locations::AudioLocationService>,
    relink_path: Entity<InputState>,
    clusters: Arc<parakeet_application::cluster_edits::ClusterEditingService>,
    cluster_review: Option<parakeet_application::cluster_edits::ClusterReview>,
    selected_clusters: Vec<String>,
    split_at: Entity<InputState>,
    media: Arc<MediaTranscriptionService>,
    speakers: Arc<SpeakerService>,
    models: Arc<ModelService>,
    processing: Arc<ProcessingService>,
    analysis: Arc<parakeet_application::analysis::MeetingAnalysisService>,
    recovery: Arc<parakeet_application::jobs::ProcessingRecoveryService>,
    pending_jobs: Vec<ProcessingRun>,
    storage: Arc<parakeet_application::storage::StorageService>,
    archive: Arc<parakeet_application::audio_archive::AudioArchiveService>,
    storage_usage: Option<Arc<parakeet_application::storage::StorageUsage>>,
    storage_busy: bool,
    storage_show_warnings: bool,
    job_control: Arc<parakeet_application::processing_control::ProcessingControlService>,
    job_monitor_running: bool,
    suggestions: Vec<SpeakerSuggestion>,
    suggestion_review: Option<SpeakerSuggestionReview>,
    match_score: Entity<InputState>,
    match_margin: Entity<InputState>,
    match_examples: Entity<InputState>,
    settings: Arc<SettingsService>,
    playback: Arc<PlaybackService>,
    capture: Arc<CaptureService>,
    meeting: Arc<parakeet_application::meeting::MeetingService>,
    meeting_plans: Vec<parakeet_application::meeting::MeetingFinalization>,
    audio_inspection: Arc<AudioInspectionService>,
    audio_settings: Arc<AudioSettingsService>,
    dictation_listener: Arc<DictationListener>,
    dictation_history: Arc<DictationHistoryService>,
    insertion_recovery: Arc<InsertionRecoveryService>,
    dictations: Vec<DictationReceipt>,
    capture_stop: Option<Arc<AtomicBool>>,
    dictation_stop: Option<Arc<AtomicBool>>,
    dictation_overlay: Option<WindowHandle<DictationOverlay>>,
    dictation_state: Arc<Mutex<serde_json::Value>>,
    devices: serde_json::Value,
    permissions: Option<serde_json::Value>,
    permissions_error: Option<String>,
    dictation_starting: bool,
    audio_preferences: AudioPreferences,
    focus: FocusHandle,
    main_window: AnyWindowHandle,
    heartbeat_last: Instant,
    heartbeat_max_ms: f64,
    heartbeat_running: bool,
    heartbeat_ticks: u64,
    metrics_path: Option<PathBuf>,
    area: Area,
    speaker_mode: SpeakerMode,
    library_filter: library::LibraryFilter,
    recording_tab: library::RecordingTab,
    editing_title: bool,
    editing_segment: Option<String>,
    transcript_find: Entity<InputState>,
    transcript_query: String,
    show_transcript_find: bool,
    reader_list: ListState,
    reader_editor_focus: FocusHandle,
    reader_matches: std::rc::Rc<Vec<usize>>,
    search_generation: u64,
    show_media_import: bool,
    show_import_path: bool,
    show_status_details: bool,
    show_ai_routes: bool,
    /// Grid columns for card screens, from the window width.
    columns: usize,
    last_error: Option<String>,
    confirm_action: Option<shell::ConfirmAction>,
    show_location_tools: bool,
    show_run_history: bool,
    show_word_evidence: bool,
    records: Vec<Recording>,
    detail: Option<reader::Document>,
    profiles: Vec<SpeakerProfile>,
    examples: Vec<VoiceExampleSummary>,
    candidates: Vec<EnrollmentCandidate>,
    voices_generation: u64,
    /// Unsaved text of transcript lines whose editor was closed, kept until saved or cleared.
    drafts: std::collections::HashMap<String, String>,
    /// Meetings whose transcript and speaker work after Stop is still running.
    finishing_meetings: std::collections::BTreeSet<String>,
    /// The recording and analysis state the voice list was last loaded for.
    voices_key: Option<String>,
    voices_loading: bool,
    analysis_active: bool,
    voices_error: Option<String>,
    calibration: Option<parakeet_domain::calibration::CalibrationReport>,
    train_person: Option<String>,
    train_hint: Option<String>,
    train_heard: Option<train::HeardClips>,
    train_done: Vec<ClusterIdentification>,
    recording_shortcut: bool,
    /// Why the shortcut just pressed or chosen was not added.
    shortcut_hint: Option<String>,
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
    edit_text: Entity<TextareaState>,
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
    playback_abort: Option<tokio::task::AbortHandle>,
    playback_generation: u64,
    playback_label: String,
    render_peak_ms: f64,
    render_count: u64,
    segment_page: usize,
    requested_recording: Option<String>,
}
fn anonymous_label(s: &str) -> String {
    let leaf = s.rsplit(':').next().unwrap_or(s);
    if s.starts_with("manual:") {
        return format!("Edited voice {}", leaf.chars().take(8).collect::<String>());
    }
    leaf.strip_prefix("SPEAKER_")
        .and_then(|s| s.parse::<usize>().ok())
        .map(|n| format!("Speaker {}", n + 1))
        .unwrap_or_else(|| leaf.to_owned())
}
impl Update {
    fn detail(detail: RecordingDetail) -> Self {
        Self::Detail(reader::Document::new(detail))
    }
    fn title_saved(detail: RecordingDetail) -> Self {
        Self::TitleSaved(reader::Document::new(detail))
    }
}
fn field(input: &Entity<InputState>) -> Div {
    div()
        .min_w(px(100.))
        .px_2()
        .py_2()
        .bg(rgb(SIDEBAR))
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
        self.launch(label, Box::pin(future), false, false, cx);
    }
    /// Long-lived work (dictation listener, meeting capture, archiving) that must not disable
    /// unrelated controls. Controls that truly conflict check that activity's own state.
    fn background_task(
        &mut self,
        label: &str,
        future: impl Future<Output = anyhow::Result<Update>> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.launch(label, Box::pin(future), false, true, cx);
    }
    fn playback_task(
        &mut self,
        label: &str,
        future: impl Future<Output = anyhow::Result<Update>> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.capture_stop.is_some() {
            // Playback is a separate process that screen capture would record as system audio.
            self.status = "Review playback is paused while a meeting is recording".into();
            cx.notify();
            return;
        }
        self.stop_playback(cx);
        self.playback_label = label.to_owned();
        self.launch(label, Box::pin(future), true, false, cx);
    }
    fn stop_playback(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.playback_abort.take() {
            handle.abort();
            self.playback_generation += 1;
            self.status = "Playback stopped".into();
            cx.notify();
        }
    }
    fn launch(
        &mut self,
        label: &str,
        future: std::pin::Pin<Box<dyn Future<Output = anyhow::Result<Update>> + Send + 'static>>,
        playback: bool,
        background: bool,
        cx: &mut Context<Self>,
    ) {
        self.status = label.into();
        let label = label.to_owned();
        if !background {
            self.pending += 1;
        }
        self.monitor_jobs(cx);
        self.start_heartbeat(cx);
        cx.notify();
        let handle = self.runtime.spawn(future);
        let playback_generation = self.playback_generation;
        if playback {
            self.playback_abort = Some(handle.abort_handle());
        }

        cx.spawn(async move |this, cx| {
            let result = handle.await;
            let _ = this.update(cx, |this, cx| {
                if !background {
                    this.pending = this.pending.saturating_sub(1);
                }
                if playback {
                    if playback_generation != this.playback_generation { cx.notify(); return; }
                    this.playback_abort = None;
                }

                match result {
                    Ok(Ok(update)) => {
                        match update {
                            Update::Clusters(review) => {
                                if this.requested_recording.as_deref()==Some(review.recording_id.as_str()) && this.selected_track.as_deref()==Some(review.track_id.as_str()) {
                                    let id=review.recording_id.clone();this.cluster_review=Some(review);this.selected_clusters.clear();
                                    let library=this.library.clone();this.task("Reloading derived speaker attribution",async move{Ok(Update::detail(library.show(&id).await?))},cx);
                                }
                            }
                            Update::Examples(profile_id, examples) => {
                                if this.selected_profile.as_deref() == Some(profile_id.as_str()) {
                                    this.examples = examples;
                                }
                            }
                            Update::Identified(done) => {
                                let (_, identification, profiles, detail) = *done;
                                this.status = format!("Named {} lines; {} voice examples saved", identification.segments.len(), identification.example_ids.len());
                                this.train_done.push(identification);
                                this.profiles = profiles;
                                this.train_person = None;
                                let name = this.person_name.clone();
                                let _ = cx.update_window(this.main_window, move |_, window, cx| {
                                    name.update(cx, |input, cx| input.set_value("", window, cx));
                                });
                                this.receive_detail(detail, cx);
                                cx.notify();
                                return;
                            }
                            Update::Calibration(report) => { this.calibration = Some(report); }
                            Update::Preferences(preferences, message) => { this.audio_preferences = preferences; this.status = message; }
                            Update::IdentificationUndone(report) => {
                                if let Some(done) = this.train_done.iter_mut().find(|i| i.id == report.identification.id) { *done = report.identification.clone(); }
                                this.status = format!("Reverted {} lines and removed {} examples", report.reverted_segments, report.removed_examples);
                                if let Some(d)=&this.detail { let id=d.recording.id.clone(); let library=this.library.clone(); this.task("Reloading derived speaker attribution",async move{Ok(Update::detail(library.show(&id).await?))},cx); }
                                cx.notify();
                                return;
                            }
                            Update::Candidates(recording, generation, result) => {
                                if this.requested_recording.as_deref() == Some(recording.as_str()) && generation == this.voices_generation {
                                    this.voices_loading = false;
                                    match result {
                                        Ok((candidates, named, active)) => { this.analysis_active = active; this.candidates = candidates; this.train_done = named; this.voices_error = None; }
                                        Err(error) => this.voices_error = Some(error),
                                    }
                                }
                            }
                            Update::Permissions(result) => match result {
                                Ok(value) => { this.permissions = Some(value); this.permissions_error = None; }
                                Err(error) => { this.permissions = None; this.permissions_error = Some(error); }
                            },
                            Update::DictationReady(result) => {
                                this.dictation_starting = false;
                                match result {
                                    Ok((permissions, devices)) => {
                                        this.permissions = Some(permissions); this.devices = devices;
                                        if let Some(reason) = this.dictation_blocker() {
                                            this.dictation_feedback(reason);
                                        } else { this.begin_dictation(cx); }
                                    }
                                    Err(error) => this.dictation_feedback(format!("Could not check dictation readiness: {error}")),
                                }
                            }
                            Update::StorageUsage(result) => {this.storage_busy=false;match result {Ok(usage)=>this.storage_usage=Some(usage),Err(error)=>{this.last_error=Some(error);this.status="Storage inspection failed; no files changed".into();cx.notify();return;}}},
                            Update::Jobs(jobs) => {this.pending_jobs = jobs;this.load_meeting_plans(cx);},
                            Update::MeetingPlans(plans) => this.meeting_plans = plans,
                            Update::MeetingFinalized(result, warning) => {
                                match result {Ok(d)=>this.receive_detail(d, cx),Err(e)=>{this.status=e.clone();this.last_error=Some(e);cx.notify();return;}}
                                this.load_meeting_plans(cx);
                                if let Some(warning)=warning {this.status=warning.clone();this.last_error=Some(warning);cx.notify();return;}
                            },
                            Update::Recovered(report, jobs) => {
                                this.pending_jobs = jobs;
                                this.load_meeting_plans(cx);
                                this.status = format!("Recovered {} interrupted jobs; {} active; {} older jobs need manual inspection",report.interrupted.len(),report.active.len(),report.unmanaged.len());
                                cx.notify(); return;
                            }
                            Update::Suggestions(recording, suggestions) => {
                                if this.requested_recording.as_deref() == Some(recording.as_str()) {this.suggestions=suggestions;}
                            }
                            Update::SuggestionReview(review) => {
                                if this.requested_recording.as_deref() == Some(review.suggestion.recording_id.as_str()) {
                                    let id=review.suggestion.recording_id.clone();
                                    if let Some(row)=this.suggestions.iter_mut().find(|r|r.id==review.suggestion.id){*row=review.suggestion.clone();}else{this.suggestions.push(review.suggestion.clone());}
                                    this.suggestion_review=Some(review);
                                    let library=this.library.clone();
                                    this.task("Reloading current transcript",async move {Ok(Update::detail(library.show(&id).await?))},cx);
                                }
                            }
                            Update::RefreshSelected | Update::SegmentSaved(_) => {
                                if let Update::SegmentSaved(id) = update
                                    && this.editing_segment.as_deref() == Some(id.as_str()) { this.editing_segment = None; this.drafts.remove(&id); }
                                if let Some(id) = this.requested_recording.clone() {
                                    let library = this.library.clone();
                                    this.task(
                                        "Reloading saved corrections",
                                        async move { Ok(Update::detail(library.show(&id).await?)) },
                                        cx,
                                    );
                                }
                            }
                            Update::Devices(d) => this.devices = d,
                            Update::Captured(result, warning, finalize) => {
                                // The recorder is free as soon as the audio is saved; the
                                // transcript is finished in the background.
                                this.capture_stop = None;
                                match result {
                                    Ok(d) => {
                                        let id = d.recording.id.clone();
                                        let title = d.recording.title().to_owned();
                                        if finalize {
                                            this.finish_meeting_after_stop(id, cx);
                                        }
                                        // Open it only from the Record screen, so a recording that
                                        // ends while you work elsewhere doesn't take the screen.
                                        if this.area == Area::Capture {
                                            this.show_new_recording(d, cx);
                                        } else {
                                            this.reload(cx);
                                        }
                                        this.status = if finalize {
                                            format!("“{title}” saved · transcribing in the background")
                                        } else {
                                            format!("“{title}” saved")
                                        };
                                        if let Some(warning)=warning {this.status=warning.clone();this.last_error=Some(warning);}
                                        cx.notify();
                                        return;
                                    }
                                    Err(e) => {
                                        this.last_error = Some(e.clone());
                                        this.status = e;
                                        cx.notify();
                                        return;
                                    }
                                }
                            }
                            Update::MeetingFinished(id, result, warning) => {
                                this.finishing_meetings.remove(&id);
                                let title = match result {
                                    Ok(d) => {
                                        let title = d.recording.title().to_owned();
                                        this.receive_detail(d, cx);
                                        title
                                    }
                                    Err(_) => "The meeting".into(),
                                };
                                this.reload(cx);
                                this.load_meeting_plans(cx);
                                this.status = format!("“{title}” is transcribed");
                                if let Some(warning) = warning {
                                    this.status = format!("“{title}”: {warning}");
                                    this.last_error = Some(this.status.clone());
                                }
                                cx.notify();
                                return;
                            }
                            Update::CopyText(text) => {cx.write_to_clipboard(ClipboardItem::new_string(text));this.status="Transcript copied".into();cx.notify();return;},
                            Update::LibrarySearch(generation,r) => {if generation==this.search_generation {this.records=r;}},
                            Update::TitleSaved(d) => {if let Some(row)=this.records.iter_mut().find(|r|r.id==d.recording.id){*row=d.recording.clone();}
                                if this.requested_recording.as_deref()==Some(d.recording.id.as_str()){this.editing_title=false;this.receive_detail(d, cx);}},
                            Update::Imported(report) => {this.reload(cx);this.status=format!("{} imported · {} already in library{}",report.imported,report.existing,if report.warnings.is_empty(){String::new()}else{format!(" · Warnings: {}",report.warnings.join("; "))});cx.notify();return;},
                            Update::Detail(d) => this.receive_detail(d, cx),
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
                                if let Some(id) = this.selected_profile.clone() {
                                    this.load_examples(id, cx);
                                }
                            }
                            Update::Models(m) => this.model_list = m,
                            Update::Dictations(receipts) => this.dictations = receipts,
                            Update::DictationStopped(result) => {
                                this.dictation_stop = None;
                                if let Some(overlay) = this.dictation_overlay.take() {
                                    let _ = overlay.update(cx, |_, window, _| window.remove_window());
                                }
                                match result {
                                    Ok(()) => {
                                        if let Ok(mut state) = this.dictation_state.lock() { state["state"] = serde_json::json!("disabled"); state.as_object_mut().map(|o| o.remove("message")); }
                                        this.status = "Dictation is off".into();
                                    }
                                    Err(error) => this.dictation_feedback(error),
                                }
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
                        // A message an update just set, or another task's, stays visible.
                        if this.status == label {
                            this.status = "Ready".into();
                        }
                    }
                    Ok(Err(e)) => {if e.chain().any(|cause|matches!(cause.downcast_ref::<ProcessingError>(),Some(ProcessingError::Cancelled(_)))){this.status="Processing cancelled. Source audio and saved chunks are retained.".into();}else{this.status=format!("Failed: {e:#}");this.last_error=Some(this.status.clone());}},
                    Err(e) => {this.status=format!("Background task failed: {e}");this.last_error=Some(this.status.clone());},
                };
                cx.notify();
            });
        })
        .detach();
    }
    fn reload(&mut self, cx: &mut Context<Self>) {
        let lib = self.library.clone();
        self.search_generation += 1;
        let generation = self.search_generation;
        let search = self.search.read(cx).value().to_string();
        self.task(
            "Loading library",
            async move {
                Ok(Update::LibrarySearch(
                    generation,
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
        self.reader_matches = Default::default();
        self.reader_list.reset(0);
        self.cluster_review = None;
        self.selected_clusters.clear();
        self.suggestions.clear();
        self.suggestion_review = None;
        self.candidates.clear();
        self.voices_key = None;
        self.analysis_active = false;
        self.train_done.clear();
        self.train_person = None;
        self.selected_track = None;
        self.segment_page = 0;
        self.editing_title = false;
        self.close_editor(cx);
        self.recording_tab = library::RecordingTab::Transcript;
        self.transcript_query.clear();
        self.transcript_find
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.reset_reader_rows(true);
        self.requested_recording = Some(id.clone());
        let lib = self.library.clone();
        if let Some(r) = self.records.iter().find(|r| r.id == id) {
            let title = r.title().to_owned();
            self.title
                .update(cx, |s, cx| s.set_value(title, window, cx));
        }
        self.task(
            "Opening recording",
            async move { Ok(Update::detail(lib.show(&id).await?)) },
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
        self.playback_task(
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
    fn process(&mut self, diarize: bool, word_timestamps: bool, cx: &mut Context<Self>) {
        let Some(d) = &self.detail else {
            return;
        };
        let Some(track) = self.selected_track.clone() else {
            return;
        };
        let id = d.recording.id.clone();
        let service = self.processing.clone();
        let lib = self.library.clone();
        let request = RunProcessingInput {
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
            word_timestamps,
        };
        self.task(
            "Processing locally; navigation remains available",
            async move {
                service.run(request).await?;
                Ok(Update::detail(lib.show(&id).await?))
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
    fn load_examples(&mut self, profile_id: String, cx: &mut Context<Self>) {
        let service = self.speakers.clone();
        self.task(
            "Loading verified examples",
            async move {
                let examples = service.examples(Some(&profile_id)).await?;
                Ok(Update::Examples(profile_id, examples))
            },
            cx,
        );
    }
}
impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let began = Instant::now();
        self.columns = ui::column_count(f32::from(window.viewport_size().width) - 236.);
        let sidebar = self.sidebar(cx);
        let mut content = div()
            .id("area-content")
            .min_w_0()
            .p_5()
            .flex()
            .flex_col()
            .gap_3();
        match self.area {
            Area::Library => {
                content = self.library_view(window, cx).id("library-area");
            }
            Area::Speakers => {
                content = content.child(section("People", "Everyone you have named, and how well their voices are recognized. To name the voices in a recording, open it in the Library and choose Name voices."));
                content = content.child(
                    div().flex().flex_wrap().gap_2().children(
                        [
                            (SpeakerMode::Profiles, "People"),
                            (SpeakerMode::Clusters, "Voice groups"),
                            (SpeakerMode::Comparisons, "Match checks"),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(index, (mode, label))| {
                            choice(("speaker-mode", index), label, self.speaker_mode == mode)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.speaker_mode = mode;
                                    cx.notify();
                                }))
                        }),
                    ),
                );
                if self.speaker_mode == SpeakerMode::Profiles {
                    content = content.child(group(
                        "People",
                        "A person is created when you name a voice in a recording, or here. Computer guesses are never saved as names on their own.",
                    ));
                    let tiles = self
                        .profiles
                        .iter()
                        .map(|p| {
                            let id = p.id.clone();
                            let name = p.name.clone();
                            tile(
                                p.id.clone(),
                                p.name.clone(),
                                None,
                                self.selected_profile.as_ref() == Some(&p.id),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.selected_profile = Some(id.clone());
                                this.examples.clear();
                                this.person_name.update(cx, |input, cx| {
                                    input.set_value(name.clone(), window, cx)
                                });
                                this.load_examples(id.clone(), cx);
                                cx.notify();
                            }))
                            .into_any_element()
                        })
                        .collect();
                    content = content.child(grid(self.columns * 2, tiles));
                    if self.profiles.is_empty() {
                        content = content.child(caption("Nobody named yet."));
                    }
                    content = content.child(
                        panel()
                            .child(caption(if self.selected_profile.is_some() {
                                "Change the selected person's name, or add someone new."
                            } else {
                                "Add someone new."
                            }))
                            .child(field(&self.person_name).w(px(280.)))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(primary("add-person", "Add as new person").on_click(
                                        cx.listener(|this, _, _, cx| {
                                            let name =
                                                this.person_name.read(cx).value().to_string();
                                            let service = this.speakers.clone();
                                            this.task(
                                                "Creating speaker profile",
                                                async move {
                                                    service.create(&name).await?;
                                                    Ok(Update::Profiles(service.profiles().await?))
                                                },
                                                cx,
                                            );
                                        }),
                                    ))
                                    .child(
                                        button("rename-person", "Rename selected")
                                            .disabled(self.selected_profile.is_none())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if let Some(id) = this.selected_profile.clone() {
                                                    let name = this
                                                        .person_name
                                                        .read(cx)
                                                        .value()
                                                        .to_string();
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
                                                            Ok(Update::Profiles(
                                                                service.profiles().await?,
                                                            ))
                                                        },
                                                        cx,
                                                    );
                                                }
                                            })),
                                    )
                                    .child(
                                        danger("delete-person", "Delete selected…")
                                            .disabled(self.selected_profile.is_none())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if let Some(id) = this.selected_profile.clone() {
                                                    this.confirm_action = Some(
                                                        shell::ConfirmAction::DeleteProfile(id),
                                                    );
                                                    cx.notify();
                                                }
                                            })),
                                    ),
                            ),
                    );
                    content = content.child(group(
                        "Recognition check",
                        "Tests whether the app can tell your named people apart, using voices you have confirmed.",
                    ));
                    content = content.child(self.calibration_view(cx));
                    if self.selected_profile.is_some() {
                        content = content.child(caption(format!(
                            "{} verified voice {}",
                            self.examples.len(),
                            if self.examples.len() == 1 {
                                "example"
                            } else {
                                "examples"
                            }
                        )));
                    }
                    for (index, example) in self.examples.iter().enumerate() {
                        let e = example.clone();
                        let remove = example.id.clone();
                        let profile = example.profile_id.clone();
                        content = content.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(format!(
                                    "{} · {} · {:.1}–{:.1}s",
                                    example.model,
                                    example.capture_domain,
                                    example.start_ms as f64 / 1000.,
                                    example.end_ms as f64 / 1000.
                                ))
                                .child(button(("example-play", index), "Play").on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        let player = this.playback.clone();
                                        let e = e.clone();
                                        this.playback_task(
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
                                            this.confirm_action =
                                                Some(shell::ConfirmAction::RemoveExample {
                                                    id: remove.clone(),
                                                    profile: profile.clone(),
                                                });
                                            cx.notify();
                                        })),
                                ),
                        );
                    }
                } else if self.speaker_mode == SpeakerMode::Clusters {
                    content = content.child(self.cluster_panel(cx));
                } else {
                    content = content.child(self.suggestion_panel(cx));
                }
            }
            Area::Storage => {
                content = content.child(self.storage_view(cx));
            }
            Area::Jobs => {
                content = content.child(section(
                    "Background jobs",
                    "Transcription and speaker work that has not finished. Your original audio is never touched.",
                ));
                content = content.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(button("refresh-jobs", "Refresh").on_click(cx.listener(
                            |this, _, _, cx| {
                                let service = this.recovery.clone();
                                this.task(
                                    "Reading pending jobs",
                                    async move { Ok(Update::Jobs(service.pending().await?)) },
                                    cx,
                                );
                            },
                        )))
                        .child(button("recover-jobs", "Recover abandoned jobs").on_click(
                            cx.listener(|this, _, _, cx| {
                                let service = this.recovery.clone();
                                this.task(
                                    "Recovering abandoned jobs",
                                    async move {
                                        let report = service.recover().await?;
                                        Ok(Update::Recovered(report, service.pending().await?))
                                    },
                                    cx,
                                );
                            }),
                        )),
                );
                content = content.child(caption(if self.pending_jobs.is_empty() {
                    "Nothing is running or waiting. A failed or interrupted run can be retried from its recording's Processing tab.".into()
                } else {
                    format!("{} unfinished jobs. Open a recording's Processing tab to retry a settled failure.", self.pending_jobs.len())
                }));
                if !self.meeting_plans.is_empty() {
                    content = content.child(group(
                        "Meeting transcripts",
                        "Resume pending or failed final passes. Recover abandoned jobs first if one was interrupted.",
                    ));
                }
                for plan in &self.meeting_plans {
                    let id = plan.recording_id.clone();
                    content = content.child(
                        panel()
                            .p_3()
                            .gap_2()
                            .child(format!(
                                "{} · {} · {}/{} sources",
                                self.records
                                    .iter()
                                    .find(|r| r.id == id)
                                    .map(|r| r.title())
                                    .unwrap_or(&id),
                                plan.status,
                                plan.completed.len(),
                                plan.track_ids.len()
                            ))
                            .child(
                                button(format!("open-finalization-{}", plan.id), "Open recording")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open(id.clone(), window, cx);
                                        this.recording_tab = library::RecordingTab::Processing;
                                        cx.notify();
                                    })),
                            ),
                    );
                }
                for run in &self.pending_jobs {
                    content = content.child(self.processing_job(run, cx));
                    let recording = self
                        .records
                        .iter()
                        .find(|r| r.id == run.recording_id)
                        .map(|r| r.title())
                        .unwrap_or(run.recording_id.as_str());
                    content = content.child(
                        div()
                            .p_3()
                            .rounded_lg()
                            .bg(rgb(PANEL))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(format!(
                                "{} · {} · started {}",
                                recording, run.kind, run.started_at
                            ))
                            .when(self.show_status_details, |card| {
                                card.child(caption(format!(
                                    "{} · {} · run {}",
                                    run.model,
                                    if run.configuration["ownership_protocol"]
                                        == parakeet_application::jobs::OWNERSHIP_PROTOCOL
                                    {
                                        "ownership-aware"
                                    } else {
                                        "older run: ownership unknown"
                                    },
                                    run.id
                                )))
                            }),
                    );
                }
            }
            Area::Models => {
                content = content.child(self.models_view(cx));
            }
            Area::Capture => {
                content = content.child(self.capture_view(cx));
            }
            Area::Dictation => {
                content = content.child(self.dictation_view(cx));
            }
            Area::Privacy => {
                content = content.child(self.privacy_view(cx));
                if self.show_ai_routes {
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
                                                (
                                                    &this.profile_name,
                                                    profile.application_name.clone(),
                                                ),
                                                (
                                                    &this.profile_executable,
                                                    profile.executable.clone(),
                                                ),
                                                (
                                                    &this.profile_arguments,
                                                    serde_json::to_string(&profile.arguments)
                                                        .unwrap_or_default(),
                                                ),
                                            ] {
                                                entity.update(cx, |s, cx| {
                                                    s.set_value(value, window, cx)
                                                });
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
                .child("Your transformation prompt").child(div().p_2().bg(rgb(SIDEBAR)).child(Textarea::new(&self.profile_prompt)))
                .child("AI CLI executable (absolute path)").child(field(&self.profile_executable))
                .child("CLI arguments (JSON array, no shell expansion)").child(field(&self.profile_arguments))
                .child(button("prompt-route",format!("Route: {} · click to change",self.prompt_route)).on_click(cx.listener(|this,_,_,cx|{this.prompt_route=if this.prompt_route=="remote"{"local"}else{"remote"}.into();cx.notify();})))
                .child("Declare local only if you have verified that the CLI runs inference on this device. A remote route may send your transcript to its provider.")
                .child(div().flex().gap_2()
                    .child(button("save-prompt-draft","Save disabled draft").on_click(cx.listener(|this,_,_,cx|this.save_prompt(false,cx))))
                    .child(button("authorize-prompt","Authorize this route & save").on_click(cx.listener(|this,_,_,cx|this.save_prompt(true,cx)))));
                }
            }
        }
        if self.area != Area::Library {
            content = div()
                .id("area-scroll")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_y_scroll()
                .child(content);
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
            .on_action(cx.listener(|this, _: &SaveSegmentText, _, cx| this.save_segment_text(cx)))
            .on_action(cx.listener(|this, _: &FindText, window, cx| {
                if this.area == Area::Library && this.detail.is_some() {
                    this.show_transcript_find = true;
                    this.transcript_find.focus_handle(cx).focus(window, cx);
                } else {
                    this.area = Area::Library;
                    this.search.focus_handle(cx).focus(window, cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenAudio, _, cx| this.choose_audio(cx)))
            .on_action(cx.listener(|this, _: &ShowLibrary, _, cx| this.navigate(Area::Library, cx)))
            .on_action(cx.listener(|this, _: &ShowCapture, _, cx| this.navigate(Area::Capture, cx)))
            .on_action(
                cx.listener(|this, _: &ShowDictation, _, cx| this.navigate(Area::Dictation, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ShowSpeakers, _, cx| this.navigate(Area::Speakers, cx)),
            )
            .on_action(cx.listener(|this, _: &DismissPanels, window, cx| {
                this.confirm_action = None;
                if this.show_transcript_find {
                    this.show_transcript_find = false;
                    this.transcript_query.clear();
                    this.refresh_reader_matches();
                    this.transcript_find
                        .update(cx, |input, cx| input.set_value("", window, cx));
                }
                this.editing_title = false;
                this.close_editor(cx);
                this.reset_reader_rows(false);
                this.show_media_import = false;
                this.show_import_path = false;
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
                this.add_shortcut(event.keystroke.key.clone(), modifiers, cx);
            }))
            .size_full()
            .tab_group()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .font_family(".AppleSystemUIFont")
            .text_size(px(13.))
            .flex()
            .child(gpui_base::TextSelectionLayer)
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .when(self.confirm_action.is_some(), |view| {
                        view.child(self.confirmation(cx))
                    })
                    .when(self.last_error.is_some(), |view| {
                        view.child(self.error_banner(cx))
                    })
                    .child(content)
                    .child(self.status_bar(cx)),
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
    [
        NextFocus,
        PreviousFocus,
        SearchLibrary,
        FindText,
        OpenAudio,
        ShowLibrary,
        ShowCapture,
        ShowDictation,
        ShowSpeakers,
        DismissPanels,
        SaveSegmentText,
        Quit
    ]
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
            .bg(rgb(SIDEBAR))
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
    /// Adds a dictation shortcut after checking it here, so a key that would take over typing or
    /// clash with another shortcut is explained instead of failing the save.
    fn add_shortcut(&mut self, key: String, modifiers: Vec<String>, cx: &mut Context<Self>) {
        let binding = gestures::Shortcut {
            id: new_id(),
            key: key.trim().to_lowercase(),
            modifiers,
            enabled: true,
        };
        self.shortcut_hint = if let Err(error) = binding.validate() {
            Some(error.0)
        } else if self.shortcuts.iter().any(|b| b.conflicts(&binding)) {
            Some("You already have this shortcut.".into())
        } else {
            None
        };
        if self.shortcut_hint.is_some() {
            cx.notify();
            return;
        }
        let mut bindings = self.shortcuts.clone();
        bindings.push(binding);
        self.persist_shortcuts(bindings, cx);
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
        let request = CaptureRecordingInput {
            dictation: false,
            microphone_uid,
            system_audio,
            title: chrono::Local::now()
                .format("Meeting %b %-d, %Y at %-I:%M %p")
                .to_string(),
            maximum_seconds: 7200,
            final_transcription: None,
        };
        self.last_error = None;
        let transcribe = self.audio_preferences.transcribe_meetings_after_stop;
        let analyze_speakers = self.audio_preferences.analyze_meetings_after_stop;
        let matching_policy = self.audio_preferences.after_stop_policy();
        let stop = Arc::new(AtomicBool::new(false));
        self.capture_stop = Some(stop.clone());
        let meeting = self.meeting.clone();
        let lib = self.library.clone();
        self.background_task(
            "Meeting capture",
            async move {
                let (id, warning, finalize) = match meeting
                    .record_audio(
                        parakeet_application::meeting::RecordMeetingInput {
                            capture: request,
                            transcribe_after_stop: transcribe,
                            skip_unavailable_model: true,
                            analyze_speakers,
                            matching_policy,
                        },
                        stop,
                    )
                    .await
                {
                    Ok(report) => (
                        Some(report.recording_id),
                        report
                            .transcription_skipped
                            .map(|why| format!("Audio saved without a transcript: {why}"))
                            .or(report.analysis_skipped.map(|why| {
                                format!("Audio saved; speakers won't be detected: {why}")
                            })),
                        report.transcription_planned,
                    ),
                    Err(error) => (
                        error.retained_recording_id().map(str::to_owned),
                        Some(error.to_string()),
                        false,
                    ),
                };
                let result = match id {
                    Some(id) => lib
                        .show(&id)
                        .await
                        .map(reader::Document::new)
                        .map_err(|e| e.to_string()),
                    None => Err(warning.clone().unwrap_or_else(|| "Recording failed".into())),
                };
                Ok(Update::Captured(result, warning, finalize))
            },
            cx,
        );
    }
    /// Applies a preference change on screen now and saves it against the stored preferences,
    /// so a change made elsewhere meanwhile is not overwritten by this screen's copy.
    fn change_preferences(
        &mut self,
        label: &str,
        saved: &str,
        change: impl Fn(&mut AudioPreferences) + Clone + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        change(&mut self.audio_preferences);
        let service = self.audio_settings.clone();
        let message = saved.to_owned();
        self.task(
            label,
            async move { Ok(Update::Preferences(service.update(change).await?, message)) },
            cx,
        );
    }
    fn dictation_feedback(&mut self, message: String) {
        self.status = message.clone();
        if let Ok(mut state) = self.dictation_state.lock() {
            state["state"] = serde_json::json!("disabled");
            state["message"] = serde_json::json!(message);
        }
    }
    fn start_dictation(&mut self, cx: &mut Context<Self>) {
        if self.dictation_stop.is_some() || self.dictation_starting {
            return;
        }
        if !self.shortcuts.iter().any(|s| s.enabled) {
            self.dictation_feedback(
                "Add and enable a shortcut below before turning on dictation.".into(),
            );
            cx.notify();
            return;
        }
        self.dictation_starting = true;
        let service = self.audio_inspection.clone();
        self.task(
            "Checking dictation permissions and microphone",
            async move {
                let result = async {
                    let permissions = service.permissions().await?;
                    let devices = service.devices().await?;
                    Ok::<_, anyhow::Error>((permissions, devices))
                }
                .await
                .map_err(|e| e.to_string());
                Ok(Update::DictationReady(result))
            },
            cx,
        );
    }
    fn begin_dictation(&mut self, cx: &mut Context<Self>) {
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
        self.background_task(
            "Listening for configured hold shortcuts",
            async move {
                let result = listener.run(uid, stop).await;
                Ok(Update::DictationStopped(
                    result.map_err(|e| format!("Dictation could not continue: {e}")),
                ))
            },
            cx,
        );
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
/// Apps opened from Finder or the Dock get a minimal PATH without Homebrew, so FFmpeg, FFprobe
/// and yt-dlp would not be found. Adds the standard tool folders that exist.
fn extend_tool_path() {
    let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for dir in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let dir = PathBuf::from(dir);
        if dir.is_dir() && !paths.contains(&dir) {
            paths.push(dir);
        }
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        // SAFETY: called first in `main`, before any other thread exists.
        unsafe { std::env::set_var("PATH", joined) };
    }
}
fn support_directory() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/ParakeetLocal")
}
/// The library the app was last explicitly started with, so opening the app from Finder finds
/// the same library. `PARAKEET_DATA_ROOT` always wins; nothing is created or migrated here.
fn library_root() -> (PathBuf, bool) {
    if let Some(root) = std::env::var_os("PARAKEET_DATA_ROOT") {
        return (PathBuf::from(root), true);
    }
    let remembered = std::fs::read_to_string(support_directory().join("library-location"))
        .ok()
        .map(|text| PathBuf::from(text.trim()))
        .filter(|path| path.is_absolute());
    (remembered.unwrap_or_else(support_directory), false)
}
fn remember_library(root: &std::path::Path) {
    let Ok(root) = root.canonicalize() else {
        return;
    };
    let support = support_directory();
    if std::fs::create_dir_all(&support).is_ok() {
        let _ = std::fs::write(
            support.join("library-location"),
            root.to_string_lossy().as_bytes(),
        );
    }
}
/// A Finder launch has no terminal, so a startup failure must be shown, not only printed.
fn show_startup_failure(error: &anyhow::Error) {
    let message = format!("{error:#}")
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            &format!(
                "display alert \"Parakeet Local could not start\" message \"{message}\" as critical"
            ),
        ])
        .status();
}
fn main() {
    extend_tool_path();
    if let Err(error) = run() {
        eprintln!("{error:#}");
        show_startup_failure(&error);
        std::process::exit(1);
    }
}
fn run() -> anyhow::Result<()> {
    #[cfg(feature = "performance")]
    let performance = performance::Recorder::from_env()?;
    #[cfg(not(feature = "performance"))]
    if std::env::var_os("PARAKEET_PERFORMANCE_DIR").is_some() {
        anyhow::bail!("Performance recording requires a build with --features performance");
    }
    let (root, explicit) = library_root();
    if !root.join("parakeet.db").exists() {
        anyhow::bail!(
            "No library at {}. Create it with `parakeet --data-root <folder> db create` and `db migrate`, then start the app once with `just run` so it remembers the folder.",
            root.display()
        );
    }
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?,
    );
    let pool = runtime.block_on(parakeet_database::open_existing(&root.join("parakeet.db")))?;
    if explicit {
        remember_library(&root);
    }
    let store = Arc::new(SqliteStore::new(pool));
    let storage = Arc::new(parakeet_application::storage::StorageService::new(
        store.clone(),
        Arc::new(parakeet_infrastructure::storage::LocalStorageInspection {
            root: root.clone(),
            database: root.join("parakeet.db"),
        }),
    ));
    let source = Arc::new(MacParakeetArtifacts);
    let models = Arc::new(ModelFiles::new(root.join("models")));
    let engine = Arc::new(LocalSpeech::new(models.clone(), model_runtime_executable()));
    let validated_audio = Arc::new(
        parakeet_infrastructure::audio_writes::ValidatedAudioWrites {
            library: store.clone(),
            locations: store.clone(),
        },
    );
    let ownership = Arc::new(
        parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
            &root.join("parakeet.db"),
        )?,
    );
    let archive = Arc::new(
        parakeet_application::audio_archive::AudioArchiveService::new(
            store.clone(),
            store.clone(),
            store.clone(),
            validated_audio.clone(),
            Arc::new(parakeet_infrastructure::audio::LocalAudioIdentity),
            Arc::new(parakeet_infrastructure::archive::FfmpegArchiver),
            ownership.clone(),
        ),
    );
    let library = Arc::new(LibraryService::new(
        store.clone(),
        validated_audio.clone(),
        source,
    ));
    let locations = Arc::new(
        parakeet_application::audio_locations::AudioLocationService::new(
            store.clone(),
            validated_audio.clone(),
            Arc::new(parakeet_infrastructure::audio::LocalAudioIdentity),
        ),
    );
    let job_control = Arc::new(
        parakeet_application::processing_control::ProcessingControlService::new(
            store.clone(),
            store.clone(),
        ),
    );
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
        store.clone(),
        ownership.clone(),
        store.clone(),
    ));
    let clusters = Arc::new(
        parakeet_application::cluster_edits::ClusterEditingService::new(
            store.clone(),
            store.clone(),
            ownership.clone(),
        ),
    );
    let recovery = Arc::new(parakeet_application::jobs::ProcessingRecoveryService::new(
        store.clone(),
        store.clone(),
        ownership.clone(),
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
    let insertion_recovery = Arc::new(InsertionRecoveryService::new(native.clone()));
    let startup_status = match runtime.block_on(insertion_recovery.recover()) {
        Ok(report) if report.restored > 0 => format!(
            "Recovered {} interrupted clipboard paste(s)",
            report.restored
        ),
        Ok(_) => "Ready".into(),
        Err(e) => format!("Clipboard recovery needs attention: {e}"),
    };
    let audio_inspection = Arc::new(AudioInspectionService::new(native.clone()));
    let capture = Arc::new(CaptureService::new(native.clone(), validated_audio.clone()));
    let analysis = Arc::new(
        parakeet_application::analysis::MeetingAnalysisService::new(
            store.clone(),
            store.clone(),
            processing.clone(),
            speakers.clone(),
            models.clone(),
            Arc::new(parakeet_infrastructure::audio::LocalAudioIdentity),
            ownership.clone(),
        )
        .with_bleed_detection(Arc::new(
            parakeet_infrastructure::bleed::LocalBleedDetection,
        )),
    );
    let meeting = Arc::new(
        parakeet_application::meeting::MeetingService::new(
            capture.clone(),
            store.clone(),
            store.clone(),
            models.clone(),
            processing.clone(),
            ownership,
        )
        .with_analysis(analysis.clone()),
    );
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
            validated_audio.clone(),
            store.clone(),
        )),
        processing.clone(),
    ));
    let models = Arc::new(ModelService::new(models));
    let records = runtime.block_on(library.list(None))?;
    let profiles = runtime.block_on(speakers.profiles())?;
    gpui_platform::application()
        .with_assets(ui::Icons)
        .run(move |cx: &mut App| {
            gpui_base::init(cx);
            gpui_base::Theme::global_mut(cx).appearance = gpui_base::ThemeAppearance::Dark;
            gpui_base::Theme::global_mut(cx).tokens.colors = gpui_base::ColorTokens::dark();
            cx.bind_keys([
                KeyBinding::new("tab", NextFocus, None),
                KeyBinding::new("shift-tab", PreviousFocus, None),
                KeyBinding::new(
                    "cmd-enter",
                    SaveSegmentText,
                    Some("TranscriptEditor > Input"),
                ),
                KeyBinding::new("cmd-f", FindText, None),
                KeyBinding::new("cmd-shift-f", SearchLibrary, None),
                KeyBinding::new("cmd-o", OpenAudio, None),
                KeyBinding::new("cmd-1", ShowLibrary, None),
                KeyBinding::new("cmd-2", ShowCapture, None),
                KeyBinding::new("cmd-3", ShowDictation, None),
                KeyBinding::new("cmd-4", ShowSpeakers, None),
                KeyBinding::new("escape", DismissPanels, None),
                KeyBinding::new("escape", DismissPanels, Some("Input")),
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
                        let search = input(window, cx, "", "Search library");
                        let transcript_find = input(window, cx, "", "Find in this transcript");
                        let title = input(window, cx, "", "Recording title");
                        let person_name = input(window, cx, "", "Person's name");
                        let start = input(window, cx, "0", "Start ms");
                        let end = input(window, cx, "10000", "End ms");
                        let edit_text = cx.new(|cx| {
                            let mut state = TextareaState::new(window, cx).auto_grow(2, 6);
                            state.set_placeholder("Correct this transcript segment", window, cx);
                            state
                        });
                        let reader_editor_focus = edit_text.focus_handle(cx);
                        let shortcut_key = input(window, cx, "", "Home, F6, etc.");
                        let import_path =
                            input(window, cx, "", "Local audio file or meeting folder");
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
                        let relink_path = input(window, cx, "", "Replacement audio file path");
                        let split_at = input(window, cx, "", "Meeting-time split in ms");
                        let match_score = input(window, cx, "", "Minimum similarity (-1 to 1)");
                        let match_margin = input(window, cx, "", "Minimum separation");
                        let match_examples = input(window, cx, "", "Verified reference count");
                        let focus = cx.focus_handle();
                        window.focus(&focus, cx);
                        let view = cx.new(|_| Workbench {
                            #[cfg(feature = "performance")]
                            performance,
                            runtime,
                            library,
                            locations,
                            relink_path,
                            clusters,
                            cluster_review: None,
                            selected_clusters: vec![],
                            split_at,
                            media,
                            speakers,
                            models,
                            processing,
                            analysis,
                            recovery,
                            pending_jobs: vec![],
                            storage,
                            archive,
                            storage_usage: None,
                            storage_busy: false,
                            storage_show_warnings: false,
                            job_control,
                            job_monitor_running: false,
                            suggestions: vec![],
                            suggestion_review: None,
                            match_score,
                            match_margin,
                            match_examples,
                            settings,
                            playback,
                            capture,
                            meeting,
                            meeting_plans: vec![],
                            audio_inspection,
                            audio_settings,
                            dictation_listener,
                            dictation_history,
                            insertion_recovery,
                            dictations: vec![],
                            capture_stop: None,
                            dictation_stop: None,
                            dictation_overlay: None,
                            dictation_state,
                            devices: serde_json::Value::Null,
                            permissions: None,
                            permissions_error: None,
                            dictation_starting: false,
                            audio_preferences,
                            focus,
                            main_window: window.window_handle(),
                            heartbeat_last: Instant::now(),
                            heartbeat_max_ms: 0.,
                            heartbeat_running: false,
                            heartbeat_ticks: 0,
                            metrics_path: std::env::var_os("PARAKEET_UI_METRICS")
                                .map(PathBuf::from),
                            area: Area::Library,
                            speaker_mode: SpeakerMode::Profiles,
                            library_filter: library::LibraryFilter::All,
                            recording_tab: library::RecordingTab::Transcript,
                            editing_title: false,
                            editing_segment: None,
                            transcript_find,
                            transcript_query: String::new(),
                            show_transcript_find: false,
                            reader_list: ListState::new(0, ListAlignment::Top, px(300.))
                                .measure_all(),
                            reader_editor_focus,
                            reader_matches: Default::default(),
                            search_generation: 0,
                            show_media_import: false,
                            show_import_path: false,
                            show_status_details: false,
                            show_ai_routes: false,
                            columns: 1,
                            last_error: None,
                            confirm_action: None,
                            show_location_tools: false,
                            show_run_history: false,
                            show_word_evidence: false,
                            records,
                            detail: None,
                            profiles,
                            examples: vec![],
                            candidates: vec![],
                            voices_generation: 0,
                            drafts: Default::default(),
                            finishing_meetings: Default::default(),
                            voices_key: None,
                            voices_loading: false,
                            analysis_active: false,
                            voices_error: None,
                            calibration: None,
                            train_person: None,
                            train_hint: None,
                            train_heard: None,
                            train_done: vec![],
                            recording_shortcut: false,
                            shortcut_hint: None,
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
                            status: startup_status,
                            pending: 0,
                            playback_abort: None,
                            playback_generation: 0,
                            playback_label: String::new(),
                            render_peak_ms: 0.,
                            render_count: 0,
                            segment_page: 0,
                            requested_recording: None,
                        });
                        view.update(cx, |this, cx| {
                            this.start_heartbeat(cx);
                            this.archive_old_captures(cx);
                            #[cfg(feature = "performance")]
                            this.start_performance_observer(window, cx);
                            this.subscribe_search(cx);
                            cx.on_focus_lost(window, |this, window, cx| {
                                this.focus.focus(window, cx)
                            })
                            .detach();
                        });
                        let quitting = view.downgrade();
                        cx.on_app_quit(move |cx| {
                            let state = quitting
                                .update(cx, |this, _| {
                                    #[cfg(feature = "performance")]
                                    if let Some(recorder) = this.performance.take() {
                                        recorder.finish();
                                    }
                                    if let Some(handle) = this.playback_abort.take() {
                                        handle.abort();
                                    }
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
                                            while began.elapsed()
                                                < std::time::Duration::from_secs(12)
                                            {
                                                if capture.status()["event"] == "completed"
                                                    || capture.status()["event"] == "failed"
                                                {
                                                    break;
                                                }
                                                tokio::time::sleep(
                                                    std::time::Duration::from_millis(100),
                                                )
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

impl Workbench {
    fn compare_interval(&mut self, cx: &mut Context<Self>) {
        let Some(recording_id) = self.detail.as_ref().map(|d| d.recording.id.clone()) else {
            self.status = "Open a recording first".into();
            cx.notify();
            return;
        };
        let Some(track_id) = self.selected_track.clone() else {
            self.status = "Choose a source first".into();
            cx.notify();
            return;
        };
        let (start_ms, end_ms) = match self.range(cx) {
            Ok(r) => r,
            Err(e) => {
                self.status = e;
                cx.notify();
                return;
            }
        };
        let policy = match (
            self.match_score.read(cx).value().parse::<f64>(),
            self.match_margin.read(cx).value().parse::<f64>(),
            self.match_examples.read(cx).value().parse::<usize>(),
        ) {
            (Ok(minimum_score), Ok(minimum_margin), Ok(minimum_examples)) => {
                parakeet_domain::recognition::MatchPolicy {
                    minimum_score,
                    minimum_margin,
                    minimum_examples,
                }
            }
            _ => {
                self.status="Enter an explicit similarity, separation, and reference count; no personal threshold has been approved".into();
                cx.notify();
                return;
            }
        };
        let service = self.speakers.clone();
        self.task(
            "Comparing voices locally; no enrollment",
            async move {
                let suggestion = service
                    .suggest(SuggestSpeakerInput {
                        recording_id,
                        track_id,
                        start_ms,
                        end_ms,
                        more_ranges: vec![],
                        policy,
                    })
                    .await?;
                Ok(Update::SuggestionReview(
                    service.review_suggestion(&suggestion.id).await?,
                ))
            },
            cx,
        );
    }
    fn resolve_review(&mut self, action: &str, segment_id: Option<String>, cx: &mut Context<Self>) {
        let Some(review) = &self.suggestion_review else {
            return;
        };
        let input = ResolveSuggestionInput {
            id: review.suggestion.id.clone(),
            action: action.into(),
            segment_id,
            profile_id: self.selected_profile.clone(),
        };
        let service = self.speakers.clone();
        self.task(
            "Saving reversible identity decision; no enrollment",
            async move {
                service.resolve_suggestion(input.clone()).await?;
                Ok(Update::SuggestionReview(
                    service.review_suggestion(&input.id).await?,
                ))
            },
            cx,
        );
    }
    fn suggestion_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut panel=div().flex().flex_col().gap_2().mt_4()
            .child(div().text_xl().child("Identity comparisons"))
            .child("Comparisons are predictions. Listen before confirming a transcript identity. Confirmation never adds a voice reference.");
        for person in &self.profiles {
            let id = person.id.clone();
            panel = panel.child(
                button(
                    format!("comparison-person-{}", person.id),
                    format!(
                        "{}Replacement identity: {}",
                        if self.selected_profile.as_ref() == Some(&person.id) {
                            "✓ "
                        } else {
                            ""
                        },
                        person.name
                    ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected_profile = Some(id.clone());
                    cx.notify();
                })),
            );
        }
        let Some(detail) = &self.detail else {
            return panel.child("Open a recording in Library to compare or review its voices.");
        };
        let recording_id = detail.recording.id.clone();
        panel=panel.child(format!("Recording: {} · prepared source: {}",detail.recording.title(),self.selected_track.as_deref().and_then(|id|detail.tracks.iter().find(|t|t.id==id)).map(|t|format!("{:?}",t.kind)).unwrap_or_else(||"none".into())))
            .child("Track-local start/end in milliseconds")
            .child(div().flex().flex_wrap().gap_2().child(field(&self.start)).child(field(&self.end)))
            .child("Explicit comparison policy: similarity, separation from next person, verified reference count")
            .child(div().flex().flex_wrap().gap_2()
                .child(div().flex().flex_col().child("Similarity").child(field(&self.match_score)))
                .child(div().flex().flex_col().child("Separation").child(field(&self.match_margin)))
                .child(div().flex().flex_col().child("Reference count").child(field(&self.match_examples))))
            .child(button("compare-voice","Compare prepared interval").on_click(cx.listener(|this,_,_,cx|this.compare_interval(cx))))
            .child(button("load-suggestions","Load saved comparisons for this recording").on_click(cx.listener(move |this,_,_,cx|{
                let service=this.speakers.clone();let id=recording_id.clone();
                this.task("Reading saved comparison evidence",async move{Ok(Update::Suggestions(id.clone(),service.suggestions(Some(&id)).await?))},cx);
            })));
        for suggestion in self.suggestions.iter().rev().take(20) {
            let id = suggestion.id.clone();
            let name = suggestion
                .decision
                .suggested_profile
                .as_ref()
                .and_then(|id| self.profiles.iter().find(|p| &p.id == id))
                .map(|p| p.name.as_str())
                .unwrap_or("Unknown");
            panel = panel.child(
                button(
                    format!("review-{}", suggestion.id),
                    format!(
                        "Review {name} · {:.1}–{:.1}s · {}",
                        suggestion.start_ms as f64 / 1000.,
                        suggestion.end_ms as f64 / 1000.,
                        suggestion.created_at
                    ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let service = this.speakers.clone();
                    let id = id.clone();
                    this.task(
                        "Reading comparison and covered speech",
                        async move {
                            Ok(Update::SuggestionReview(
                                service.review_suggestion(&id).await?,
                            ))
                        },
                        cx,
                    );
                })),
            );
        }
        if self.suggestions.len() > 20 {
            panel = panel.child("Showing the 20 latest comparisons; CLI can inspect all receipts.");
        }
        let Some(review) = &self.suggestion_review else {
            return panel;
        };
        let suggestion = &review.suggestion;
        let resolved = suggestion
            .resolutions
            .last()
            .is_some_and(|r| r.action != "undo");
        panel = panel
            .child(div().text_lg().child(format!(
                "Selected comparison · {}",
                suggestion.decision.reason.replace('_', " ")
            )))
            .child(format!(
                "Policy {:.3} similarity / {:.3} separation / {} references · {}",
                suggestion.policy.minimum_score,
                suggestion.policy.minimum_margin,
                suggestion.policy.minimum_examples,
                if resolved { "resolved" } else { "unresolved" }
            ));
        for candidate in &suggestion.decision.candidates {
            let name = self
                .profiles
                .iter()
                .find(|p| p.id == candidate.profile_id)
                .map(|p| p.name.as_str())
                .unwrap_or("Deleted profile");
            panel=panel.child(format!("Candidate {name} · similarity {:.3} · {} compatible verified references (similarity is not a probability)",candidate.score,candidate.examples));
        }
        let play = PlayAudioInput {
            recording_id: suggestion.recording_id.clone(),
            track_id: suggestion.track_id.clone(),
            start_ms: suggestion.start_ms,
            duration_ms: suggestion.end_ms - suggestion.start_ms,
        };
        panel = panel.child(
            button("play-comparison", "Play comparison evidence").on_click(cx.listener(
                move |this, _, _, cx| {
                    let service = this.playback.clone();
                    let input = play.clone();
                    this.playback_task(
                        "Playing compared source interval",
                        async move {
                            service.play(input).await?;
                            Ok(Update::Message("Comparison playback complete".into()))
                        },
                        cx,
                    );
                },
            )),
        );
        if resolved {
            return panel.child(
                button("undo-comparison", "Undo comparison decision")
                    .on_click(cx.listener(|this, _, _, cx| this.resolve_review("undo", None, cx))),
            );
        }
        panel = panel.child(
            button("reject-comparison", "Reject this comparison")
                .on_click(cx.listener(|this, _, _, cx| this.resolve_review("reject", None, cx))),
        );
        if review.eligible_segments.is_empty() {
            panel=panel.child("No current transcript segment is fully covered by this evidence. Keep the comparison or choose another interval; individual manual corrections remain in the reader.");
        }
        for segment in &review.eligible_segments {
            let accept = segment.id.clone();
            let replace = segment.id.clone();
            let mut row = div()
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .bg(rgb(PANEL))
                .child(segment.text.clone())
                .child(if segment.evidence["uncertain_speech"] == true {
                    "ASR flagged uncertain speech. Listen before attributing these words."
                } else {
                    "Model transcript; identity confirmation does not verify every word."
                });
            if suggestion.decision.suggested_profile.is_some() {
                row = row.child(
                    button(
                        format!("accept-comparison-{}", segment.id),
                        "I listened — confirm proposed identity for this speech",
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.resolve_review("accept", Some(accept.clone()), cx)
                    })),
                );
            }
            if self.selected_profile.is_some() {
                row = row.child(
                    button(
                        format!("replace-comparison-{}", segment.id),
                        format!(
                            "I listened — assign {} to this speech",
                            self.selected_profile
                                .as_ref()
                                .and_then(|id| self.profiles.iter().find(|p| &p.id == id))
                                .map(|p| p.name.as_str())
                                .unwrap_or("selected person")
                        ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.resolve_review("replace", Some(replace.clone()), cx)
                    })),
                );
            }
            panel = panel.child(row);
        }
        panel
    }
}

impl Workbench {
    fn inspect_clusters(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else {
            self.status = "Open a recording first".into();
            cx.notify();
            return;
        };
        let Some(track) = self.selected_track.clone() else {
            return;
        };
        let service = self.clusters.clone();
        let input = parakeet_application::cluster_edits::InspectClustersInput {
            recording_id: detail.recording.id.clone(),
            track_id: track,
            run_id: None,
        };
        self.task(
            "Inspecting source-scoped anonymous speakers",
            async move { Ok(Update::Clusters(service.inspect(input).await?)) },
            cx,
        );
    }
    fn edit_clusters(
        &mut self,
        edit: parakeet_application::cluster_edits::ClusterEdit,
        cx: &mut Context<Self>,
    ) {
        if self.pending > 0 {
            self.status =
                "Wait for the current background operation before editing clusters".into();
            cx.notify();
            return;
        }
        let Some(review) = &self.cluster_review else {
            return;
        };
        if self.selected_track.as_deref() != Some(review.track_id.as_str()) {
            self.status = "Inspect clusters for the selected source first".into();
            cx.notify();
            return;
        }
        let input = parakeet_application::cluster_edits::EditClustersInput {
            recording_id: review.recording_id.clone(),
            run_id: review.run_id.clone(),
            edit,
        };
        let service = self.clusters.clone();
        let track = review.track_id.clone();
        self.task(
            "Saving derived cluster edit; raw result preserved",
            async move {
                let run = service.edit(input).await?;
                Ok(Update::Clusters(
                    service
                        .inspect(parakeet_application::cluster_edits::InspectClustersInput {
                            recording_id: run.recording_id,
                            track_id: track,
                            run_id: Some(run.id),
                        })
                        .await?,
                ))
            },
            cx,
        );
    }
    fn cluster_panel(&self, cx: &mut Context<Self>) -> Div {
        use parakeet_application::cluster_edits::ClusterEdit;
        let mut panel=div().flex().flex_col().gap_2().mt_4().child(div().text_xl().child("Anonymous speaker clusters"))
            .child("Split or merge voices within one source. This creates a derived result; it does not name a person or save voice examples.")
            .child(button("inspect-clusters","Inspect latest clusters for selected source").on_click(cx.listener(|this,_,_,cx|this.inspect_clusters(cx))));
        let Some(review) = self
            .cluster_review
            .as_ref()
            .filter(|r| self.selected_track.as_deref() == Some(r.track_id.as_str()))
        else {
            return panel;
        };
        if let Some(detail) = self
            .detail
            .as_ref()
            .filter(|d| d.recording.id == review.recording_id)
        {
            let source = detail
                .tracks
                .iter()
                .find(|t| t.id == review.track_id)
                .map(|t| format!("{:?}", t.kind))
                .unwrap_or_else(|| "unavailable".into());
            panel = panel.child(format!(
                "Recording: {} · source: {}",
                detail.recording.title(),
                source
            ));
        }
        panel = panel.child(format!(
            "Model {} · run {} · {} clusters",
            review.model,
            review.run_id,
            review.clusters.len()
        ));
        for cluster in &review.clusters {
            let id = cluster.id.clone();
            let mut row = div().flex().flex_col().gap_2().p_3().bg(rgb(PANEL)).child(
                button(
                    format!("choose-cluster-{}", cluster.id),
                    format!(
                        "{}{} · {} intervals · {:.1}s speech · {:.1}–{:.1}s meeting time",
                        if self.selected_clusters.contains(&cluster.id) {
                            "✓ "
                        } else {
                            ""
                        },
                        anonymous_label(&cluster.id),
                        cluster.interval_count,
                        cluster.speech_ms as f64 / 1000.,
                        cluster.start_ms as f64 / 1000.,
                        cluster.end_ms as f64 / 1000.
                    ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.selected_clusters.contains(&id) {
                        this.selected_clusters.retain(|v| v != &id)
                    } else {
                        this.selected_clusters.push(id.clone())
                    }
                    cx.notify();
                })),
            );
            if let Some(preview) = &cluster.preview {
                let preview = preview.clone();
                row = row.child(
                    button(
                        format!("preview-cluster-{}", cluster.id),
                        "Play representative nonoverlapping speech",
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let service = this.playback.clone();
                        let input = preview.clone();
                        this.playback_task(
                            "Playing anonymous cluster evidence",
                            async move {
                                service.play(input).await?;
                                Ok(Update::Message("Cluster playback complete".into()))
                            },
                            cx,
                        );
                    })),
                );
            } else {
                row = row.child(
                    "No safely aligned, nonoverlapping preview; inspect the original track.",
                );
            }
            panel = panel.child(row);
        }
        panel=panel.child("Select one cluster to split at an explicit meeting time; select two or more to merge.")
            .child(field(&self.split_at))
            .child(button("split-cluster","Split selected anonymous cluster").on_click(cx.listener(|this,_,_,cx|{
                let Some(speaker)=this.selected_clusters.first().filter(|_|this.selected_clusters.len()==1).cloned() else{this.status="Select exactly one cluster".into();cx.notify();return};
                let Ok(at_ms)=this.split_at.read(cx).value().parse::<i64>() else{this.status="Enter a meeting-time split in milliseconds".into();cx.notify();return};
                this.edit_clusters(ClusterEdit::Split{speaker,at_ms},cx);
            })))
            .child(button("merge-clusters","Merge selected anonymous clusters").on_click(cx.listener(|this,_,_,cx|{this.edit_clusters(ClusterEdit::Merge{speakers:this.selected_clusters.clone()},cx)})));
        if review.can_undo {
            panel = panel.child(
                button("undo-cluster-edit", "Undo latest cluster edit").on_click(
                    cx.listener(|this, _, _, cx| this.edit_clusters(ClusterEdit::Undo, cx)),
                ),
            );
        }
        panel
    }
}

impl Workbench {
    fn fingerprint_audio(&mut self, cx: &mut Context<Self>) {
        let (Some(detail), Some(track)) = (&self.detail, self.selected_track.clone()) else {
            return;
        };
        let input = parakeet_application::audio_locations::FingerprintAudioInput {
            recording_id: detail.recording.id.clone(),
            track_id: track,
        };
        let service = self.locations.clone();
        let library = self.library.clone();
        self.task(
            "Hashing selected source without modifying audio",
            async move {
                let track = service.fingerprint(input).await?;
                Ok(Update::detail(library.show(&track.recording_id).await?))
            },
            cx,
        );
    }
    fn relink_audio(&mut self, dry_run: bool, cx: &mut Context<Self>) {
        let (Some(detail), Some(track)) = (&self.detail, self.selected_track.clone()) else {
            return;
        };
        let input = parakeet_application::audio_locations::RelinkAudioInput {
            recording_id: detail.recording.id.clone(),
            track_id: track,
            path: PathBuf::from(self.relink_path.read(cx).value().to_string()),
            dry_run,
        };
        let service = self.locations.clone();
        let library = self.library.clone();
        self.task(
            "Verifying replacement audio content",
            async move {
                let report = service.relink(input).await?;
                if report.dry_run {
                    Ok(Update::Message(
                        "Replacement is byte-identical. Locator was not changed.".into(),
                    ))
                } else {
                    Ok(Update::detail(
                        library.show(&report.track.recording_id).await?,
                    ))
                }
            },
            cx,
        );
    }
}
