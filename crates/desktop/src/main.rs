mod activity;
mod analysis;
mod job_control;
mod library;
#[cfg(feature = "performance")]
mod performance;
mod reader;
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
    Train,
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
    Candidates(String, Vec<EnrollmentCandidate>),
    Identified(
        String,
        ClusterIdentification,
        Vec<SpeakerProfile>,
        reader::Document,
    ),
    IdentificationUndone(UndoIdentificationReport),
    RefreshSelected,
    SegmentSaved(String),
    Models(Vec<ModelInfo>),
    DictationStopped(String),
    Dictations(Vec<DictationReceipt>),
    Message(String),
    Jobs(Vec<ProcessingRun>),
    StoragePreview(
        Result<
            (
                Arc<parakeet_application::storage::RetentionPreview>,
                Vec<parakeet_application::cleanup::CleanupReview>,
            ),
            String,
        >,
    ),
    CleanupReviews(
        Result<
            (
                Vec<parakeet_application::cleanup::CleanupReview>,
                Option<parakeet_application::cleanup::CleanupReview>,
            ),
            String,
        >,
    ),
    StoragePolicySaved(Result<parakeet_application::storage::RetentionPolicySnapshot, String>),
    Recovered(
        parakeet_application::jobs::ProcessingRecoveryReport,
        Vec<ProcessingRun>,
    ),
    Suggestions(String, Vec<SpeakerSuggestion>),
    SuggestionReview(SpeakerSuggestionReview),
    Settings(Vec<gestures::Shortcut>, Vec<PromptProfile>),
    Devices(serde_json::Value),
    Captured(Result<reader::Document, String>, Option<String>),
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
    cleanup: Arc<parakeet_application::cleanup::CleanupService<SqliteStore, SqliteStore>>,
    cleanup_reviews: Vec<parakeet_application::cleanup::CleanupReview>,
    cleanup_selected: Option<parakeet_application::cleanup::CleanupReview>,
    storage_preview: Option<Arc<parakeet_application::storage::RetentionPreview>>,
    storage_policy: Option<parakeet_application::storage::RetentionPolicySnapshot>,
    storage_days: Entity<InputState>,
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
    train_person: Option<String>,
    train_skipped: Vec<String>,
    train_done: Vec<ClusterIdentification>,
    train_candidates_named: Vec<(String, String)>,
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
        self.launch(label, Box::pin(future), false, cx);
    }
    fn playback_task(
        &mut self,
        label: &str,
        future: impl Future<Output = anyhow::Result<Update>> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.stop_playback(cx);
        self.playback_label = label.to_owned();
        self.launch(label, Box::pin(future), true, cx);
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
        cx: &mut Context<Self>,
    ) {
        self.status = label.into();
        self.pending += 1;
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
                this.pending = this.pending.saturating_sub(1);
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
                            Update::Identified(candidate, identification, profiles, detail) => {
                                this.train_candidates_named.push((candidate, identification.id.clone()));
                                this.status = format!("Named {} lines; {} voice examples saved", identification.segments.len(), identification.example_ids.len());
                                this.train_done.push(identification);
                                this.profiles = profiles;
                                this.train_person = None;
                                let name = this.person_name.clone();
                                let _ = cx.update_window(this.main_window, move |_, window, cx| {
                                    name.update(cx, |input, cx| input.set_value("", window, cx));
                                });
                                this.receive_detail(detail);
                                cx.notify();
                                return;
                            }
                            Update::IdentificationUndone(report) => {
                                if let Some(done) = this.train_done.iter_mut().find(|i| i.id == report.identification.id) { *done = report.identification.clone(); }
                                this.status = format!("Reverted {} lines and removed {} examples", report.reverted_segments, report.removed_examples);
                                if let Some(d)=&this.detail { let id=d.recording.id.clone(); let library=this.library.clone(); this.task("Reloading derived speaker attribution",async move{Ok(Update::detail(library.show(&id).await?))},cx); }
                                cx.notify();
                                return;
                            }
                            Update::Candidates(recording,c) => {if this.requested_recording.as_deref()==Some(recording.as_str()){this.candidates = c;}},
                            Update::StoragePreview(result) => {this.storage_busy=false;match result {Ok((preview,reviews))=>{this.storage_policy=Some(preview.saved_policy.clone());this.storage_preview=Some(preview);if let Some(selected)=&this.cleanup_selected {this.cleanup_selected=reviews.iter().find(|r|r.id==selected.id).cloned();}this.cleanup_reviews=reviews;},Err(error)=>{this.last_error=Some(error);this.status="Storage inspection failed; no files changed".into();cx.notify();return;}}},
                            Update::CleanupReviews(result) => {this.storage_busy=false; match result {Ok((reviews,selected))=> {this.cleanup_reviews=reviews;this.cleanup_selected=selected;if let Some(r)=&this.cleanup_selected {this.status=format!("{} · {} files · no space reclaimed",storage::cleanup_state_label(r.state),r.files.len());if !r.errors.is_empty() {this.last_error=Some(r.errors.join("; "));}}else{this.status="No files qualified for this bounded review; no files moved".into();}},Err(error)=>{this.last_error=Some(error);this.status="Cleanup request failed; inspect saved reviews before retrying".into();}}cx.notify();return;},
                            Update::StoragePolicySaved(result) => {this.storage_busy=false;match result {Ok(policy)=>{this.storage_policy=Some(policy);this.storage_preview=None;this.status="Preview policy saved. No files were removed; refresh to inspect it.".into();},Err(error)=>{this.last_error=Some(error);this.status="Preview policy could not be saved; refresh before retrying".into();}}cx.notify();return;},
                            Update::Jobs(jobs) => {this.pending_jobs = jobs;this.load_meeting_plans(cx);},
                            Update::MeetingPlans(plans) => this.meeting_plans = plans,
                            Update::MeetingFinalized(result, warning) => {
                                match result {Ok(d)=>this.receive_detail(d),Err(e)=>{this.status=e.clone();this.last_error=Some(e);cx.notify();return;}}
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
                                    && this.editing_segment.as_deref() == Some(id.as_str()) { this.editing_segment = None; }
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
                            Update::Captured(result, warning) => {
                                this.capture_stop = None;
                                match result {
                                    Ok(d) => {
                                        this.editing_title=false;this.editing_segment=None;this.segment_page=0;
                                        this.recording_tab=library::RecordingTab::Transcript;
                                        this.show_transcript_find=false;this.transcript_query.clear();
                                        this.cluster_review=None;this.selected_clusters.clear();this.candidates.clear();
                                        this.suggestions.clear();this.suggestion_review=None;
                                        this.reader_list.reset(0);
                                        this.library_filter=library::LibraryFilter::All;
                                        let find=this.transcript_find.clone();let search=this.search.clone();
                                        let _=cx.update_window(this.main_window,move|_,window,cx|{
                                            find.update(cx,|input,cx|input.set_value("",window,cx));
                                            search.update(cx,|input,cx|input.set_value("",window,cx));
                                        });
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
                                        this.receive_detail(d);
                                        this.area = Area::Library;
                                        this.reload(cx);
                                        if let Some(warning)=warning {this.status=warning.clone();this.last_error=Some(warning);cx.notify();return;}
                                    }
                                    Err(e) => {
                                        this.status = e;
                                        cx.notify();
                                        return;
                                    }
                                }
                            }
                            Update::CopyText(text) => {cx.write_to_clipboard(ClipboardItem::new_string(text));this.status="Transcript copied".into();cx.notify();return;},
                            Update::LibrarySearch(generation,r) => {if generation==this.search_generation {this.records=r;}},
                            Update::TitleSaved(d) => {if let Some(row)=this.records.iter_mut().find(|r|r.id==d.recording.id){*row=d.recording.clone();}
                                if this.requested_recording.as_deref()==Some(d.recording.id.as_str()){this.editing_title=false;this.receive_detail(d);}},
                            Update::Imported(report) => {this.reload(cx);this.status=format!("{} imported · {} already in library{}",report.imported,report.existing,if report.warnings.is_empty(){String::new()}else{format!(" · Warnings: {}",report.warnings.join("; "))});cx.notify();return;},
                            Update::Detail(d) => this.receive_detail(d),
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
        self.selected_track = None;
        self.segment_page = 0;
        self.editing_title = false;
        self.editing_segment = None;
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
                content = content.child(section("Speakers", "Review anonymous voices, make corrections, and manage the examples you explicitly verified."));
                content = content.child(
                    div().flex().flex_wrap().gap_2().children(
                        [
                            (SpeakerMode::Train, "Identify voices"),
                            (SpeakerMode::Profiles, "People and examples"),
                            (SpeakerMode::Clusters, "Anonymous clusters"),
                            (SpeakerMode::Comparisons, "Identity comparisons"),
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
                if self.speaker_mode == SpeakerMode::Train {
                    content = content.child(self.train_view(cx));
                } else if self.speaker_mode == SpeakerMode::Profiles {
                    content=content.child(div().flex().flex_col().gap_2().child(caption("PEOPLE & VERIFIED EXAMPLES")).child(caption("A name is confirmed only when you identify the voice. Predictions never become training examples automatically.")))
     .child(div().flex().gap_2().child(field(&self.person_name).w(px(260.))).child(button("add-person","Create person").on_click(cx.listener(|this,_,_,cx|{let name=this.person_name.read(cx).value().to_string();let service=this.speakers.clone();this.task("Creating speaker profile",async move{service.create(&name).await?;Ok(Update::Profiles(service.profiles().await?))},cx);}))));
                    content = content.child(
                        button("refresh-people", "Refresh people and examples")
                            .self_start()
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_profiles(cx))),
                    );
                    let mut people = div().flex().flex_wrap().gap_2();
                    for p in &self.profiles {
                        let id = p.id.clone();
                        let name = p.name.clone();
                        people = people.child(
                            choice(
                                p.id.clone(),
                                p.name.clone(),
                                self.selected_profile.as_ref() == Some(&p.id),
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.selected_profile = Some(id.clone());
                                    this.examples.clear();
                                    this.person_name.update(cx, |input, cx| {
                                        input.set_value(name.clone(), window, cx)
                                    });
                                    this.load_examples(id.clone(), cx);
                                    cx.notify();
                                },
                            )),
                        );
                    }
                    content = content.child(people);
                    content = content.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                button("rename-person", "Rename selected person")
                                    .disabled(self.selected_profile.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(id) = this.selected_profile.clone() {
                                            let name =
                                                this.person_name.read(cx).value().to_string();
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
                                    })),
                            )
                            .child(
                                danger("delete-person", "Delete person…")
                                    .disabled(self.selected_profile.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(id) = this.selected_profile.clone() {
                                            this.confirm_action =
                                                Some(shell::ConfirmAction::DeleteProfile(id));
                                            cx.notify();
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
                    if let Some(detail) = &self.detail {
                        let source = self
                            .selected_track
                            .as_ref()
                            .and_then(|id| detail.tracks.iter().find(|t| &t.id == id))
                            .map(|t| format!("{:?}", t.kind))
                            .unwrap_or_else(|| "No source selected".into());
                        content = content
                            .child(caption(format!(
                                "Prepared audio: {} · {} · track-local milliseconds",
                                detail.recording.title(),
                                source
                            )))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(caption("Start ms"))
                                    .child(field(&self.start).w(px(120.)))
                                    .child(caption("End ms"))
                                    .child(field(&self.end).w(px(120.))),
                            );
                    } else {
                        content = content.child(caption(
                            "Open a recording in the Library to prepare voice examples.",
                        ));
                    }
                    content = content.child(
                        button("play-enrollment", "Play prepared interval")
                            .self_start()
                            .disabled(self.detail.is_none() || self.selected_track.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.play(cx))),
                    );
                    content = content.child(
                        primary("enroll", "I verified this voice — enroll selected interval")
                            .self_start()
                            .disabled(
                                self.detail.is_none()
                                    || self.selected_track.is_none()
                                    || self.selected_profile.is_none()
                                    || self.pending > 0,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                let (Some(d), Some(track), Some(profile)) = (
                                    &this.detail,
                                    this.selected_track.clone(),
                                    this.selected_profile.clone(),
                                ) else {
                                    this.status =
                                        "Select a recording, source, and person first".into();
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
                                let profile_id = profile.clone();
                                let input = EnrollSpeakerInput {
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
                                        let examples = service.examples(Some(&profile_id)).await?;
                                        Ok(Update::Examples(profile_id, examples))
                                    },
                                    cx,
                                );
                            })),
                    );
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
                content=content.child(section("Background jobs","Review unfinished processing. Recovery preserves active work and original audio."))
                    .child(button("refresh-jobs","Refresh pending jobs").self_start().on_click(cx.listener(|this,_,_,cx|{let service=this.recovery.clone();this.task("Reading pending jobs",async move{Ok(Update::Jobs(service.pending().await?))},cx); })))
                    .child(button("recover-jobs","Recover abandoned processing jobs").self_start().on_click(cx.listener(|this,_,_,cx|{let service=this.recovery.clone();this.task("Recovering abandoned jobs",async move{let report=service.recover().await?;Ok(Update::Recovered(report,service.pending().await?))},cx); })))
                    .child(caption(if self.pending_jobs.is_empty(){"No unfinished processing jobs. Failed or interrupted runs can be retried from their recording’s Processing tab.".into()}else{format!("{} unfinished jobs. Open a recording’s Processing tab to retry a settled failure.",self.pending_jobs.len())}));
                content=content.child(section("Meeting transcripts", "Resume pending or failed final passes. Recover abandoned source jobs above before resuming an interrupted pass."));
                for plan in &self.meeting_plans {
                    let id = plan.recording_id.clone();
                    content = content.child(
                        div()
                            .p_3()
                            .bg(rgb(PANEL))
                            .flex()
                            .flex_col()
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
                    content = content.child(div().p_3().bg(rgb(PANEL)).child(format!(
                        "{} · {} · {} · started {} · {} · run {}",
                        recording,
                        run.kind,
                        run.model,
                        run.started_at,
                        if run.configuration["ownership_protocol"]
                            == parakeet_application::jobs::OWNERSHIP_PROTOCOL
                        {
                            "ownership-aware"
                        } else {
                            "older run: ownership unknown"
                        },
                        run.id
                    )));
                }
            }
            Area::Models => {
                content = content.child(section("Local models","Choose separate models for transcription, speaker detection, and verified voice examples. Downloads are always explicit."));
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
                            .bg(rgb(PANEL))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(BORDER))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(15.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(ui::model_name(&m.id).to_owned()),
                                    )
                                    .child(pill(
                                        if m.verified {
                                            "Ready offline"
                                        } else if m.installed {
                                            "Needs verification"
                                        } else {
                                            "Not downloaded"
                                        },
                                        if m.verified { SUCCESS } else { MUTED },
                                    )),
                            )
                            .child(caption(format!(
                                "{} · {} · {}",
                                m.id,
                                m.category,
                                m.bytes
                                    .map(|b| format!("{:.0} MB", b as f64 / 1_000_000.))
                                    .unwrap_or("Size not specified".into())
                            )))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .when(m.detail["selected"] == true, |row| {
                                        row.child(pill("Selected for processing", ACCENT))
                                    })
                                    .when(m.detail["selected_for_embeddings"] == true, |row| {
                                        row.child(pill("Selected for voice examples", SUCCESS))
                                    }),
                            )
                            .when(self.show_status_details, |row| {
                                row.child(caption(format!(
                                    "Revision {} · {}",
                                    m.revision, m.license
                                )))
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(
                                        primary(format!("select-{}", m.id), "Use model")
                                            .disabled(
                                                !m.verified
                                                    || m.detail["selected"] == true
                                                    || self.pending > 0,
                                            )
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
                                            .disabled(
                                                !m.verified
                                                    || m.detail["selected_for_embeddings"] == true
                                                    || self.pending > 0,
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
                                        danger(format!("remove-model-{}", m.id), "Remove…")
                                            .disabled(!m.installed || self.pending > 0)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.confirm_action =
                                                    Some(shell::ConfirmAction::RemoveModel(
                                                        remove_id.clone(),
                                                    ));
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        button(format!("install-{}", m.id), "Download")
                                            .disabled(m.installed || self.pending > 0)
                                            .on_click(cx.listener(move |this, _, _, cx| {
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
                                            })),
                                    )
                                    .child(
                                        button(format!("verify-{}", m.id), "Verify files")
                                            .disabled(!m.installed || self.pending > 0)
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
                content=content.child(section("Record a meeting","Choose your microphone and audio sources. Separate source recordings are saved for accurate review."));
                let health = self.capture.status();
                content = content.child(
                    button("refresh-audio-devices", "Refresh inputs")
                        .self_start()
                        .on_click(cx.listener(|this, _, _, cx| this.navigate(Area::Capture, cx))),
                );
                content = content.child(choice("auto-final-transcription", if self.audio_preferences.transcribe_meetings_after_stop {"After Stop: transcribe locally (on)"} else {"After Stop: save audio only"},self.audio_preferences.transcribe_meetings_after_stop)
                    .disabled(self.capture_stop.is_some()).on_click(cx.listener(|this,_,_,cx|{
                        this.audio_preferences.transcribe_meetings_after_stop = !this.audio_preferences.transcribe_meetings_after_stop;
                        if !this.audio_preferences.transcribe_meetings_after_stop { this.audio_preferences.analyze_meetings_after_stop=false; }
                        let preferences=this.audio_preferences.clone();let service=this.audio_settings.clone();
                        this.task("Saving meeting transcription preference",async move{service.save(&preferences).await?;Ok(Update::Message("Meeting transcription preference saved".into()))},cx);
                    }))).child(caption("Uses the selected installed local ASR model. Audio is saved first; failed final transcription can be resumed."));
                content=content.child(choice("auto-speaker-analysis",if self.audio_preferences.analyze_meetings_after_stop {"After Stop: prepare anonymous speakers (on)"}else{"After Stop: speaker analysis off"},self.audio_preferences.analyze_meetings_after_stop)
                    .disabled(self.capture_stop.is_some()).on_click(cx.listener(|this,_,_,cx|{
                        this.audio_preferences.analyze_meetings_after_stop = !this.audio_preferences.analyze_meetings_after_stop;
                        if this.audio_preferences.analyze_meetings_after_stop {this.audio_preferences.transcribe_meetings_after_stop=true;}
                        let preferences=this.audio_preferences.clone();let service=this.audio_settings.clone();
                        this.task("Saving speaker analysis preference",async move{service.save(&preferences).await?;Ok(Update::Message("Speaker analysis preference saved".into()))},cx);
                    }))).child(caption("Pins installed ASR and diarization models before capture. Prepares playable unknown voices; no identity comparison or enrollment."));
                if self.devices["devices"]
                    .as_array()
                    .is_none_or(|devices| devices.is_empty())
                {
                    content = content.child(caption(if self.pending > 0 { "Checking available audio inputs…" } else { "No microphone inputs are available. Connect an input and refresh. System audio can be recorded separately." }));
                }
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
                    .child(caption(format!("Selected microphone: {}",self.audio_preferences.microphone_uid.as_ref().and_then(|uid|self.devices["devices"].as_array().and_then(|devices|devices.iter().find(|d|d["uid"].as_str()==Some(uid.as_str())))).and_then(|d|d["name"].as_str()).unwrap_or("Choose an input above; a saved unavailable device will not be substituted silently"))))
                    .when(self.show_status_details,|view|view.child(caption(format!("Selected input UID: {}",self.audio_preferences.microphone_uid.as_deref().unwrap_or("None")))))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .flex_wrap()
                            .child(primary("record-both", "Microphone + system").disabled(self.capture_stop.is_some() || self.audio_preferences.microphone_uid.is_none()).on_click(
                                cx.listener(|this, _, _, cx| this.start_capture(true, true, cx)),
                            ))
                            .child(button("record-system", "System audio only").disabled(self.capture_stop.is_some()).on_click(
                                cx.listener(|this, _, _, cx| this.start_capture(false, true, cx)),
                            ))
                            .child(button("record-mic", "Microphone only").disabled(self.capture_stop.is_some() || self.audio_preferences.microphone_uid.is_none()).on_click(
                                cx.listener(|this, _, _, cx| this.start_capture(true, false, cx)),
                            ))
                            .child(
                                danger("stop-recording", "Stop & save").disabled(self.capture_stop.is_none() || health["state"] == "completed").on_click(cx.listener(
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
                if self.capture_stop.is_some() && health["state"] == "completed" {
                    content = content.child(pill("Audio saved · finishing transcription", ACCENT));
                }
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
                        let frames = h["written_frames"].as_f64().unwrap_or(0.);
                        let rate = h["sample_rate"].as_f64().unwrap_or(0.);
                        if rate > 0.
                            && frames / rate >= 10.
                            && h["session_peak_amplitude"]
                                .as_f64()
                                .is_some_and(|peak| peak < parakeet_domain::SILENT_SOURCE_PEAK)
                        {
                            content = content.child(format!(
                                "{source}: SILENT — receiving buffers but no speech-level signal; check the selected input"
                            ));
                        }
                    }
                }
            }
            Area::Dictation => {
                content=content.child(section("Dictation","Hold any active shortcut to speak. Releasing it transcribes locally for the application where you started."))
                    .child(quiet("recover-clipboard","Recover interrupted clipboard paste").self_start().on_click(cx.listener(|this,_,_,cx|{
                    let service=this.insertion_recovery.clone();this.task("Recovering only Parakeet-owned clipboard transactions",async move{let report=service.recover().await?;Ok(Update::Message(format!("Restored {} interrupted paste(s); {} older snapshots discarded after clipboard changed",report.restored,report.discarded)))},cx);
                })));
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
                    button("refresh-dictations", "Refresh retained dictations")
                        .self_start()
                        .on_click(cx.listener(|this, _, _, cx| {
                            let service = this.dictation_history.clone();
                            this.task(
                                "Reading retained dictations",
                                async move { Ok(Update::Dictations(service.list().await?)) },
                                cx,
                            );
                        })),
                );
                for (index, receipt) in self.dictations.iter().take(10).enumerate() {
                    let raw = receipt.raw.clone();
                    content = content.child(
                        div()
                            .p_2()
                            .bg(rgb(SIDEBAR))
                            .child(format!(
                                "{} · {} · {}",
                                receipt.created_at, receipt.target.bundle_id, receipt.status
                            ))
                            .child(receipt.raw.clone())
                            .when_some(receipt.insertion.clone(), |row, report| {
                                row.child(format!(
                                    "Insertion: {} · {} · clipboard {}",
                                    report.method,
                                    report.verification,
                                    report.clipboard_status.as_deref().unwrap_or("untouched")
                                ))
                            })
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
                                primary("start-listening", "Enable dictation")
                                    .disabled(self.dictation_stop.is_some())
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.start_dictation(cx)),
                                    ),
                            )
                            .child(
                                button("stop-listening", "Disable dictation")
                                    .disabled(self.dictation_stop.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(stop) = this.dictation_stop.take() {
                                            stop.store(true, Ordering::SeqCst);
                                        }
                                        cx.notify();
                                    })),
                            ),
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
                content=content.child(section("Privacy & application prompts","Transcription and speaker processing run locally. AI enhancement stays disabled until you authorize an exact route."))
                    .child(div().p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(PANEL)).child("Raw speech is always retained. A CLI may use a remote provider: authorize it only after reviewing the route and prompt."));
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
                this.editing_segment = None;
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
            microphone_uid,
            system_audio,
            title: format!("Meeting {}", now()),
            maximum_seconds: 7200,
            final_transcription: None,
        };
        let transcribe = self.audio_preferences.transcribe_meetings_after_stop;
        let analyze_speakers = self.audio_preferences.analyze_meetings_after_stop;
        let stop = Arc::new(AtomicBool::new(false));
        self.capture_stop = Some(stop.clone());
        let meeting = self.meeting.clone();
        let lib = self.library.clone();
        self.task(
            "Recording — waiting for first buffers",
            async move {
                let (id, warning) = match meeting
                    .record(
                        parakeet_application::meeting::RecordMeetingInput {
                            capture: request,
                            transcribe_after_stop: transcribe,
                            skip_unavailable_model: true,
                            analyze_speakers,
                        },
                        stop,
                    )
                    .await
                {
                    Ok(report) => (
                        Some(report.recording_id),
                        report
                            .transcription_skipped
                            .map(|why| format!("Audio saved without a transcript: {why}")),
                    ),
                    Err(error) => (
                        error.retained_recording_id().map(str::to_owned),
                        Some(error.to_string()),
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
                Ok(Update::Captured(result, warning))
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
    #[cfg(feature = "performance")]
    let performance = performance::Recorder::from_env()?;
    #[cfg(not(feature = "performance"))]
    if std::env::var_os("PARAKEET_PERFORMANCE_DIR").is_some() {
        anyhow::bail!("Performance recording requires a build with --features performance");
    }
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
    let storage = Arc::new(parakeet_application::storage::StorageService::new(
        store.clone(),
        store.clone(),
        Arc::new(parakeet_infrastructure::storage::LocalStorageInspection {
            root: root.clone(),
            database: root.join("parakeet.db"),
        }),
    ));
    let source = Arc::new(MacParakeetArtifacts);
    let models = Arc::new(ModelFiles::new(root.join("models")));
    let engine = Arc::new(LocalSpeech::new(models.clone(), model_runtime_executable()));
    let guarded_audio = Arc::new(parakeet_infrastructure::cleanup::GuardedAudioWrites {
        gate: parakeet_infrastructure::cleanup::StorageGate::for_database(
            &root.join("parakeet.db"),
        )?,
        library: store.clone(),
        locations: store.clone(),
    });
    let cleanup = Arc::new(parakeet_application::cleanup::CleanupService::new(
        store.clone(),
        store.clone(),
        Arc::new(parakeet_infrastructure::cleanup::LocalCleanupFiles::new(
            root.clone(),
            root.join("parakeet.db"),
            models.clone(),
        )?),
    ));
    let library = Arc::new(LibraryService::new(
        store.clone(),
        guarded_audio.clone(),
        source,
    ));
    let locations = Arc::new(
        parakeet_application::audio_locations::AudioLocationService::new(
            store.clone(),
            guarded_audio.clone(),
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
    let ownership = Arc::new(
        parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(
            &root.join("parakeet.db"),
        )?,
    );
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
    let capture = Arc::new(CaptureService::new(native.clone(), guarded_audio.clone()));
    let analysis = Arc::new(parakeet_application::analysis::MeetingAnalysisService::new(
        store.clone(),
        store.clone(),
        processing.clone(),
        speakers.clone(),
        models.clone(),
        Arc::new(parakeet_infrastructure::audio::LocalAudioIdentity),
        ownership.clone(),
    ));
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
            guarded_audio.clone(),
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
                        let storage_days = input(window, cx, "30", "Days, 1–3650");
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
                            cleanup,
                            cleanup_reviews: Vec::new(),
                            cleanup_selected: None,
                            storage_preview: None,
                            storage_policy: None,
                            storage_days,
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
                            train_person: None,
                            train_skipped: vec![],
                            train_done: vec![],
                            train_candidates_named: vec![],
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
