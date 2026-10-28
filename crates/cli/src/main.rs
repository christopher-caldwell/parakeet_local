use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};
use parakeet_application::{capture::*, processing::*, settings::*, speakers::*, *};
use parakeet_database::SqliteStore;
use parakeet_domain::{gestures::Shortcut, recognition::MatchPolicy, *};
use parakeet_infrastructure::{
    artifacts::MacParakeetArtifacts, media::YouTubeMedia, models::ModelFiles, speech::LocalSpeech,
    transformation::AuthorizedCli,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

#[derive(Parser)]
#[command(
    name = "parakeet",
    version,
    about = "Local speech workbench. JSON stdout; progress stderr. Database setup is explicit."
)]
struct Cli {
    #[arg(long, global = true, help = "Independent application storage root")]
    data_root: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        help = "Override SQLite file; never created by ordinary commands"
    )]
    database: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        help = "JSON is already the default output format"
    )]
    json: bool,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Storage {
        #[command(subcommand)]
        command: Storage,
    },
    Jobs {
        #[command(subcommand)]
        command: Jobs,
    },
    Doctor,
    Db {
        #[command(subcommand)]
        command: Db,
    },
    Models {
        #[command(subcommand)]
        command: Models,
    },
    Meetings {
        #[command(subcommand)]
        command: Meetings,
    },
    Transcribe(ProcessArgs),
    TranscribeAll {
        meeting_id: String,
        #[arg(long, default_value = "selected-asr")]
        model: String,
    },
    Capture {
        #[command(subcommand)]
        command: Capture,
    },
    Dictation {
        #[command(subcommand)]
        command: Dictation,
    },
    Speakers {
        #[command(subcommand)]
        command: Speakers,
    },
    Prompts {
        #[command(subcommand)]
        command: Prompts,
    },
    Shortcuts {
        #[command(subcommand)]
        command: Shortcuts,
    },
    Media {
        #[command(subcommand)]
        command: Media,
    },
    Benchmark {
        #[command(subcommand)]
        command: Benchmark,
    },
}
#[derive(Subcommand)]
enum Storage {
    /// Review, reversibly stage, or restore completed processing scratch.
    Cleanup {
        #[command(subcommand)]
        command: Cleanup,
    },
    /// Read-only file accounting, including referenced audio outside this root.
    Usage,
    /// Read the saved preview policy; defaults to keep all.
    Policy,
    /// Preview candidates without changing policy or removing files.
    Preview {
        #[arg(long, conflicts_with = "keep_all")]
        completed_scratch_days: Option<u16>,
        #[arg(long)]
        keep_all: bool,
    },
    /// Save a preview policy only. It never starts cleanup.
    #[command(group(clap::ArgGroup::new("choice").required(true).args(["completed_scratch_days","keep_all"])))]
    SetPolicy {
        #[arg(long, conflicts_with = "keep_all")]
        completed_scratch_days: Option<u16>,
        #[arg(long)]
        keep_all: bool,
        #[arg(long)]
        expected_revision: u64,
    },
}
#[derive(Subcommand)]
enum Cleanup {
    /// Persist a bounded concrete review. No audio files are moved.
    Review {
        #[arg(long)]
        completed_scratch_days: Option<u16>,
        #[arg(long, default_value_t = 128)]
        limit: usize,
    },
    List,
    Show {
        id: String,
    },
    /// Move exactly the reviewed files to recovery. Reclaims zero bytes.
    Apply {
        id: String,
        #[arg(long)]
        review_digest: String,
        #[arg(long)]
        confirm: bool,
    },
    /// Restore retained files without overwriting anything.
    Restore {
        id: String,
    },
}
#[derive(Subcommand)]
enum Jobs {
    Show { id: String },
    Cancel { id: String },
    Pending,
    Recover,
    Retry { id: String },
}
#[derive(Subcommand)]
enum Dictation {
    RecoverClipboard,
    History,
    Show {
        id: String,
    },
    Listen {
        #[arg(long)]
        microphone_uid: String,
    },
    AudioSettings {
        #[arg(long, conflicts_with = "clear_microphone")]
        microphone_uid: Option<String>,
        #[arg(long)]
        clear_microphone: bool,
        #[arg(long)]
        system: Option<bool>,
    },
}
#[derive(Subcommand)]
enum Capture {
    /// Inspect or explicitly change desktop automatic-final-transcription behavior.
    Settings {
        #[arg(long)]
        transcribe_after_stop: Option<bool>,
    },
    Devices,
    Permissions,
    Recoverable,
    Recover {
        directory: PathBuf,
    },
    Record {
        #[arg(long)]
        microphone_uid: Option<String>,
        #[arg(long)]
        system: bool,
        #[arg(long, default_value = "Meeting")]
        title: String,
        #[arg(long, default_value_t = 60)]
        seconds: u64,
        /// Run pinned local ASR on every saved source after stopping; audio is retained on failure.
        #[arg(long)]
        transcribe: bool,
    },
}
#[derive(Subcommand)]
enum Db {
    Create,
    Migrate,
    Verify,
}
#[derive(Subcommand)]
enum Models {
    SelectEmbedding { model: String },
    Selected,
    Select { model: String },
    List,
    Install { model: String },
    Verify { model: String },
    Remove { model: String },
}
#[derive(Subcommand)]
enum Meetings {
    /// Resume durable final transcription; completed source work is reused.
    Finalize {
        id: String,
        #[arg(long)]
        model: Option<String>,
    },
    PendingTranscriptions,
    Fingerprint {
        id: String,
        #[arg(long)]
        track: String,
    },
    Relink {
        id: String,
        #[arg(long)]
        track: String,
        path: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    PlaySegment {
        id: String,
        segment: String,
    },
    Play {
        id: String,
        #[arg(long)]
        track: String,
        #[arg(long, default_value_t = 0)]
        start_ms: i64,
        #[arg(long, default_value_t = 10000)]
        duration_ms: i64,
    },
    Discover {
        path: Option<PathBuf>,
        #[arg(long)]
        details: bool,
    },
    Import {
        path: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    ImportSourceDb {
        path: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    List {
        #[arg(long)]
        search: Option<String>,
    },
    Show {
        id: String,
    },
    Tracks {
        id: String,
    },
    Rename {
        id: String,
        title: String,
    },
    Export {
        id: String,
        #[arg(long, default_value = "json")]
        format: String,
    },
}
#[derive(Args, Clone)]
struct ProcessArgs {
    meeting_id: String,
    #[arg(long)]
    track: String,
    #[arg(long, default_value = "selected-asr")]
    model: String,
    #[arg(long, default_value_t = 0)]
    start_ms: i64,
    #[arg(long)]
    duration_ms: Option<i64>,
    /// Preserve experimental model-estimated word timing; does not force speaker identity.
    #[arg(long)]
    word_timestamps: bool,
}
impl ProcessArgs {
    fn request(self) -> ProcessRequest {
        ProcessRequest {
            recording_id: self.meeting_id,
            track_id: self.track,
            model: self.model,
            start_ms: self.start_ms,
            duration_ms: self.duration_ms,
            word_timestamps: self.word_timestamps,
        }
    }
}
#[derive(Subcommand)]
enum Speakers {
    Clusters {
        recording: String,
        #[arg(long)]
        track: String,
        #[arg(long)]
        run: Option<String>,
    },
    ReviewSuggestion {
        id: String,
    },
    SplitCluster {
        recording: String,
        run: String,
        speaker: String,
        at_ms: i64,
    },
    MergeClusters {
        recording: String,
        run: String,
        #[arg(required=true,num_args=2..)]
        speakers: Vec<String>,
    },
    UndoClusterEdit {
        recording: String,
        run: String,
    },
    Suggestions {
        #[arg(long)]
        recording: Option<String>,
    },
    ResolveSuggestion {
        id: String,
        #[arg(long)]
        action: String,
        #[arg(long)]
        segment: Option<String>,
        #[arg(long)]
        profile: Option<String>,
    },
    Candidates {
        meeting: String,
    },
    List,
    Create {
        name: String,
    },
    Rename {
        id: String,
        name: String,
    },
    Delete {
        id: String,
    },
    Examples {
        profile: Option<String>,
    },
    RemoveExample {
        id: String,
    },
    Diarize {
        meeting_id: String,
        #[arg(long)]
        track: String,
        #[arg(long, default_value_t = 0)]
        start_ms: i64,
        #[arg(long)]
        duration_ms: Option<i64>,
    },
    Enroll {
        profile: String,
        meeting: String,
        #[arg(long)]
        track: String,
        #[arg(long)]
        start_ms: i64,
        #[arg(long)]
        end_ms: i64,
        #[arg(
            long,
            help = "Confirm that a human listened and verified this identity"
        )]
        confirmed: bool,
    },
    Suggest {
        meeting: String,
        #[arg(long)]
        track: String,
        #[arg(long)]
        start_ms: i64,
        #[arg(long)]
        end_ms: i64,
        #[arg(
            long,
            help = "Explicit tuning threshold; no uncalibrated permanent default"
        )]
        minimum_score: f64,
        #[arg(long)]
        minimum_margin: f64,
        #[arg(long, default_value_t = 2)]
        minimum_examples: usize,
    },
    Assign {
        segment: String,
        profile: String,
    },
    Reject {
        segment: String,
    },
    Unknown {
        segment: String,
    },
    Text {
        segment: String,
        text: String,
    },
    Undo {
        segment: String,
    },
}
#[derive(Subcommand)]
enum Prompts {
    List,
    Put {
        file: PathBuf,
        #[arg(long, help = "Explicitly authorize this declared local/remote route")]
        authorize: bool,
    },
    Revoke {
        id: String,
    },
    Test {
        bundle_id: String,
        text: String,
        #[arg(long, help = "Only resolve route; never start a subprocess")]
        dry_run: bool,
    },
}
#[derive(Subcommand)]
enum Shortcuts {
    List,
    Set { file: PathBuf },
}
#[derive(Subcommand)]
enum Media {
    Inspect {
        url: String,
    },
    Import {
        url: String,
    },
    Transcribe {
        url: String,
        #[arg(long, default_value = "selected-asr")]
        model: String,
    },
}
#[derive(Subcommand)]
enum Benchmark {
    ScoreSpeakers {
        reference: PathBuf,
        prediction: PathBuf,
        #[arg(long, default_value = "english_case_punctuation")]
        normalization: String,
    },
    Run {
        suite: PathBuf,
    },
    Score {
        reference: PathBuf,
        prediction: PathBuf,
        #[arg(long, default_value_t = 0)]
        collar_ms: i64,
        #[arg(long)]
        ignore_overlap: bool,
    },
    Report {
        path: PathBuf,
    },
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
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
fn data_root() -> PathBuf {
    home().join("Library/Application Support/ParakeetLocal")
}
fn source_root() -> PathBuf {
    home().join("Library/Application Support/MacParakeet/meeting-recordings")
}
fn yt_dlp() -> PathBuf {
    std::env::var_os("PARAKEET_YT_DLP")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("yt-dlp"))
}
fn writes(command: &Command) -> bool {
    if let Command::Capture {
        command: Capture::Settings {
            transcribe_after_stop,
        },
    } = command
    {
        return transcribe_after_stop.is_some();
    }
    if let Command::Dictation {
        command:
            Dictation::AudioSettings {
                microphone_uid,
                clear_microphone,
                system,
            },
    } = command
    {
        return microphone_uid.is_some() || *clear_microphone || system.is_some();
    }
    matches!(
        command,
        Command::Storage {
            command: Storage::SetPolicy { .. }
                | Storage::Cleanup {
                    command: Cleanup::Review { .. }
                        | Cleanup::Apply { .. }
                        | Cleanup::Restore { .. }
                }
        } | Command::Jobs {
            command: Jobs::Recover | Jobs::Retry { .. } | Jobs::Cancel { .. }
        } | Command::Meetings {
            command: Meetings::Import { dry_run: false, .. }
                | Meetings::ImportSourceDb { dry_run: false, .. }
                | Meetings::Finalize { .. }
                | Meetings::Rename { .. }
                | Meetings::Fingerprint { .. }
                | Meetings::Relink { dry_run: false, .. }
        } | Command::Transcribe(_)
            | Command::TranscribeAll { .. }
            | Command::Capture {
                command: Capture::Record { .. } | Capture::Recover { .. }
            }
            | Command::Dictation {
                command: Dictation::Listen { .. } | Dictation::RecoverClipboard
            }
            | Command::Speakers {
                command: Speakers::Create { .. }
                    | Speakers::Rename { .. }
                    | Speakers::Delete { .. }
                    | Speakers::RemoveExample { .. }
                    | Speakers::Diarize { .. }
                    | Speakers::Enroll { .. }
                    | Speakers::Assign { .. }
                    | Speakers::Reject { .. }
                    | Speakers::Unknown { .. }
                    | Speakers::Text { .. }
                    | Speakers::Undo { .. }
                    | Speakers::Suggest { .. }
                    | Speakers::ResolveSuggestion { .. }
                    | Speakers::SplitCluster { .. }
                    | Speakers::MergeClusters { .. }
                    | Speakers::UndoClusterEdit { .. }
            }
            | Command::Prompts {
                command: Prompts::Put { .. } | Prompts::Revoke { .. }
            }
            | Command::Shortcuts {
                command: Shortcuts::Set { .. }
            }
            | Command::Media {
                command: Media::Import { .. } | Media::Transcribe { .. }
            }
            | Command::Benchmark {
                command: Benchmark::Run { .. }
            }
    )
}
fn value<T: serde::Serialize>(v: T) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(v)?)
}
async fn execute(cli: Cli) -> anyhow::Result<Value> {
    let root = cli.data_root.unwrap_or_else(data_root);
    let database = cli.database.unwrap_or_else(|| root.join("parakeet.db"));
    let models = Arc::new(ModelFiles::new(root.join("models")));
    let source = Arc::new(MacParakeetArtifacts);
    let native = Arc::new(parakeet_infrastructure::native::NativeBridge::new(
        std::env::var_os("PARAKEET_NATIVE_BRIDGE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("native/parakeet-bridge")
            }),
        root.clone(),
    ));
    let media = Arc::new(YouTubeMedia {
        executable: yt_dlp(),
        root: root.join("media"),
    });
    match &cli.command {
        Command::Dictation {
            command: Dictation::RecoverClipboard,
        } => {
            return value(
                parakeet_application::dictation::InsertionRecoveryService::new(native.clone())
                    .recover()
                    .await?,
            );
        }
        Command::Capture {
            command: Capture::Devices,
        } => {
            return Ok(AudioInspectionService::new(native.clone())
                .devices()
                .await?);
        }
        Command::Capture {
            command: Capture::Permissions,
        } => {
            return Ok(AudioInspectionService::new(native.clone())
                .permissions()
                .await?);
        }
        Command::Capture {
            command: Capture::Recoverable,
        } => {
            return Ok(AudioInspectionService::new(native.clone())
                .recoverable()
                .await?);
        }
        Command::Doctor => {
            let database_status = if database.exists() {
                match parakeet_database::verify(&database).await {
                    Ok(v) => v,
                    Err(e) => json!({"error":e.to_string()}),
                }
            } else {
                json!({"initialized":false})
            };
            let ffmpeg = tokio::process::Command::new("ffmpeg")
                .arg("-version")
                .output()
                .await
                .ok()
                .map(|o| o.status.success())
                .unwrap_or(false);
            return Ok(
                json!({"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"data_root":root,"database":database_status,"ffmpeg":ffmpeg,"models":models.list().await?,"ai_disabled_without_authorized_profile":true,"does_not_request_hardware_permissions":true}),
            );
        }
        Command::Db { command } => {
            match command {
                Db::Create => parakeet_database::create_empty(&database)
                    .await
                    .map_err(anyhow::Error::from_boxed)?,
                Db::Migrate => parakeet_database::migrate(&database)
                    .await
                    .map_err(anyhow::Error::from_boxed)?,
                Db::Verify => {
                    return parakeet_database::verify(&database)
                        .await
                        .map_err(anyhow::Error::from_boxed);
                }
            }
            return Ok(
                json!({"database":database,"operation":if matches!(command,Db::Create){"created_empty"}else{"migrated"}}),
            );
        }
        Command::Models { command } => {
            let service = ModelService::new(models.clone());
            return match command {
                Models::List => value(service.list().await?),
                Models::SelectEmbedding { model } => value(service.select_embedding(model).await?),
                Models::Selected => value(service.selection().await?),
                Models::Select { model } => value(service.select(model).await?),
                Models::Install { model } => value(service.install(model).await?),
                Models::Verify { model } => value(service.verify(model).await?),
                Models::Remove { model } => {
                    service.remove(model).await?;
                    Ok(json!({"removed":model}))
                }
            };
        }
        Command::Meetings {
            command:
                Meetings::Import {
                    path,
                    dry_run: true,
                },
        } => {
            return value(
                DiscoveryService::new(source.clone())
                    .plan_import(path)
                    .await?,
            );
        }
        Command::Meetings {
            command: Meetings::Discover { path, details },
        } => {
            let discovery = DiscoveryService::new(source.clone())
                .discover(&path.clone().unwrap_or_else(source_root))
                .await?;
            return if *details {
                value(discovery)
            } else {
                value(discovery.audit())
            };
        }
        Command::Media {
            command: Media::Inspect { url },
        } => return Ok(media.inspect(url).await?),
        Command::Benchmark {
            command:
                Benchmark::Score { .. } | Benchmark::ScoreSpeakers { .. } | Benchmark::Report { .. },
        } => {
            return match &cli.command {
                Command::Benchmark { command } => match command {
                    Benchmark::ScoreSpeakers {
                        reference,
                        prediction,
                        normalization,
                    } => value(
                        parakeet_application::scoring::ScoringService::new(Arc::new(
                            parakeet_infrastructure::evaluation::EvaluationFiles {
                                root: root.join("evaluations"),
                            },
                        ))
                        .speaker_words(parakeet_application::scoring::ScoreSpeakerWordsInput {
                            reference: reference.clone(),
                            prediction: prediction.clone(),
                            normalization: normalization.clone(),
                        })
                        .await?,
                    ),
                    Benchmark::Score {
                        reference,
                        prediction,
                        collar_ms,
                        ignore_overlap,
                    } => value(
                        parakeet_application::scoring::ScoringService::new(Arc::new(
                            parakeet_infrastructure::evaluation::EvaluationFiles {
                                root: root.join("evaluations"),
                            },
                        ))
                        .diarization(parakeet_application::scoring::ScoreDiarizationInput {
                            reference: reference.clone(),
                            prediction: prediction.clone(),
                            collar_ms: *collar_ms,
                            score_overlap: !ignore_overlap,
                        })
                        .await?,
                    ),
                    Benchmark::Report { path } => {
                        Ok(serde_json::from_slice(&tokio::fs::read(path).await?)?)
                    }
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
        }
        _ => {}
    }
    // Connections are opened only for actual database consumers. Read commands use mode=ro.
    let pool = if writes(&cli.command) {
        parakeet_database::open_existing(&database).await
    } else {
        parakeet_database::open_read_only(&database).await
    }
    .with_context(|| {
        format!(
            "open {}; run db create and db migrate explicitly",
            database.display()
        )
    })?;
    let store = Arc::new(SqliteStore::new(pool.clone()));
    let guarded_audio = Arc::new(parakeet_infrastructure::cleanup::GuardedAudioWrites {
        gate: parakeet_infrastructure::cleanup::StorageGate::for_database(&database)?,
        library: store.clone(),
        locations: store.clone(),
    });
    let cleanup = parakeet_application::cleanup::CleanupService::new(
        store.clone(),
        store.clone(),
        Arc::new(parakeet_infrastructure::cleanup::LocalCleanupFiles::new(
            root.clone(),
            database.clone(),
            models.clone(),
        )?),
    );
    let library = LibraryService::new(store.clone(), guarded_audio.clone(), source.clone());
    let engine = Arc::new(LocalSpeech::new(models.clone(), model_runtime_executable()));
    let ownership =
        Arc::new(parakeet_infrastructure::jobs::FileProcessingOwnership::for_database(&database)?);
    let recovery = parakeet_application::jobs::ProcessingRecoveryService::new(
        store.clone(),
        store.clone(),
        ownership.clone(),
    );
    let processing = Arc::new(ProcessingService::new(
        store.clone(),
        store.clone(),
        models.clone(),
        engine.clone(),
        store.clone(),
        ownership.clone(),
        store.clone(),
    ));
    let speakers = SpeakerService::new(
        store.clone(),
        store.clone(),
        store.clone(),
        engine.clone(),
        models.clone(),
    );
    let settings = Arc::new(SettingsService::new(
        store.clone(),
        store.clone(),
        Arc::new(AuthorizedCli),
    ));
    let locations = parakeet_application::audio_locations::AudioLocationService::new(
        store.clone(),
        guarded_audio.clone(),
        Arc::new(parakeet_infrastructure::audio::LocalAudioIdentity),
    );
    let meeting = parakeet_application::meeting::MeetingService::new(
        Arc::new(parakeet_application::capture::CaptureService::new(
            native.clone(),
            guarded_audio.clone(),
        )),
        store.clone(),
        store.clone(),
        models.clone(),
        processing.clone(),
        ownership.clone(),
    );
    let storage = parakeet_application::storage::StorageService::new(
        store.clone(),
        store.clone(),
        Arc::new(parakeet_infrastructure::storage::LocalStorageInspection {
            root: root.clone(),
            database: database.clone(),
        }),
    );
    let result = match cli.command {
        Command::Storage { command } => match command {
            Storage::Cleanup { command } => match command {
                Cleanup::Review {
                    completed_scratch_days,
                    limit,
                } => value(
                    serde_json::json!({"review":cleanup.review(completed_scratch_days.map(|days| parakeet_domain::storage::RetentionPolicy {completed_scratch_after_days:Some(days)}),limit).await?,"reclaimed_bytes":0}),
                ),
                Cleanup::List => value(cleanup.list().await?),
                Cleanup::Show { id } => value(cleanup.show(&id).await?),
                Cleanup::Apply {
                    id,
                    review_digest,
                    confirm,
                } => cleanup_value(cleanup.apply(&id, &review_digest, confirm).await?),
                Cleanup::Restore { id } => cleanup_value(cleanup.restore(&id).await?),
            },
            Storage::Usage => value(storage.usage().await?),
            Storage::Policy => value(storage.policy().await?),
            Storage::Preview {
                completed_scratch_days,
                keep_all,
            } => value(
                storage
                    .preview(if keep_all || completed_scratch_days.is_some() {
                        Some(parakeet_domain::storage::RetentionPolicy {
                            completed_scratch_after_days: completed_scratch_days,
                        })
                    } else {
                        None
                    })
                    .await?,
            ),
            Storage::SetPolicy {
                completed_scratch_days,
                keep_all: _,
                expected_revision,
            } => value(
                storage
                    .save_policy(
                        parakeet_domain::storage::RetentionPolicy {
                            completed_scratch_after_days: completed_scratch_days,
                        },
                        expected_revision,
                    )
                    .await?,
            ),
        },
        Command::Jobs { command } => match command {
            Jobs::Show { id } => value(
                parakeet_application::processing_control::ProcessingControlService::new(
                    store.clone(),
                    store.clone(),
                )
                .show(&id)
                .await?,
            ),
            Jobs::Cancel { id } => value(
                parakeet_application::processing_control::ProcessingControlService::new(
                    store.clone(),
                    store.clone(),
                )
                .cancel(&id)
                .await?,
            ),
            Jobs::Pending => value(recovery.pending().await?),
            Jobs::Recover => value(recovery.recover().await?),
            Jobs::Retry { id } => value(processing.retry(&id).await?),
        },
        Command::Meetings { command } => match command {
            Meetings::Finalize { id, model } => {
                value(meeting.finalize(&id, model.as_deref()).await?)
            }
            Meetings::PendingTranscriptions => value(meeting.pending().await?),
            Meetings::Fingerprint { id, track } => value(
                locations
                    .fingerprint(
                        parakeet_application::audio_locations::FingerprintAudioInput {
                            recording_id: id,
                            track_id: track,
                        },
                    )
                    .await?,
            ),
            Meetings::Relink {
                id,
                track,
                path,
                dry_run,
            } => value(
                locations
                    .relink(parakeet_application::audio_locations::RelinkAudioInput {
                        recording_id: id,
                        track_id: track,
                        path,
                        dry_run,
                    })
                    .await?,
            ),
            Meetings::ImportSourceDb { path, dry_run } => value(
                LegacyImportService::new(
                    Arc::new(parakeet_database::MacParakeetCatalog),
                    source.clone(),
                    guarded_audio.clone(),
                )
                .import(ImportLegacyInput {
                    path: path.to_path_buf(),
                    dry_run,
                })
                .await?,
            ),
            Meetings::Import { path, dry_run } => value(
                library
                    .import_path(ImportRecordingsInput {
                        path: path.to_path_buf(),
                        dry_run,
                    })
                    .await?,
            ),
            Meetings::List { search } => value(library.list(search.as_deref()).await?),
            Meetings::PlaySegment { id, segment } => value(
                PlaybackService::new(
                    store.clone(),
                    Arc::new(parakeet_infrastructure::audio::LocalPlayback {
                        root: root.join("playback"),
                    }),
                )
                .play_segment(PlaySegmentInput {
                    recording_id: id,
                    segment_id: segment,
                })
                .await?,
            ),
            Meetings::Play {
                id,
                track,
                start_ms,
                duration_ms,
            } => {
                PlaybackService::new(
                    store.clone(),
                    Arc::new(parakeet_infrastructure::audio::LocalPlayback {
                        root: root.join("playback"),
                    }),
                )
                .play(PlayAudioInput {
                    recording_id: id,
                    track_id: track.clone(),
                    start_ms,
                    duration_ms,
                })
                .await?;
                Ok(
                    json!({"played_track":track,"start_ms":start_ms,"requested_duration_ms":duration_ms}),
                )
            }
            Meetings::Show { id } => value(library.show(&id).await?),
            Meetings::Tracks { id } => value(library.tracks(&id).await?),
            Meetings::Rename { id, title } => {
                library
                    .rename(RenameRecordingInput {
                        id: id.clone(),
                        title: title.clone(),
                    })
                    .await?;
                Ok(json!({"renamed":id,"title":title}))
            }
            Meetings::Export { id, format } => match format.as_str() {
                "json" => value(library.show(&id).await?),
                "markdown" => Ok(json!({"markdown":library.export_markdown(&id).await?})),
                "text" => Ok(json!({"text":library.export_text(&id).await?})),
                _ => anyhow::bail!("format must be json, markdown or text"),
            },
            Meetings::Discover { .. } => unreachable!(),
        },
        Command::Dictation {
            command: Dictation::History,
        } => value(
            parakeet_application::dictation::DictationHistoryService::new(store.clone())
                .list()
                .await?,
        ),
        Command::Dictation {
            command: Dictation::Show { id },
        } => value(
            parakeet_application::dictation::DictationHistoryService::new(store.clone())
                .show(&id)
                .await?,
        ),
        Command::Dictation {
            command:
                Dictation::AudioSettings {
                    microphone_uid,
                    clear_microphone,
                    system,
                },
        } => {
            let service = AudioSettingsService::new(store.clone(), store.clone());
            let mut preferences = service.get().await?;
            if microphone_uid.is_some() || clear_microphone || system.is_some() {
                if clear_microphone {
                    preferences.microphone_uid = None;
                } else if let Some(uid) = microphone_uid {
                    preferences.microphone_uid = Some(uid);
                }
                if let Some(enabled) = system {
                    preferences.system_audio = enabled;
                }
                service.save(&preferences).await?;
            }
            value(preferences)
        }
        Command::Dictation {
            command: Dictation::Listen { microphone_uid },
        } => {
            let capture = Arc::new(parakeet_application::capture::CaptureService::new(
                native.clone(),
                guarded_audio.clone(),
            ));
            let dictation = Arc::new(parakeet_application::dictation::DictationService::new(
                capture,
                processing.clone(),
                settings.clone(),
                native.clone(),
                store.clone(),
            ));
            let listener = parakeet_application::dictation::DictationListener::new(
                native,
                settings.clone(),
                dictation,
                Arc::new(parakeet_infrastructure::native::StderrDictationObserver),
            );
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let signal = stop.clone();
            let task = tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                signal.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            listener.run(microphone_uid, stop).await?;
            task.abort();
            Ok(json!({"stopped":true}))
        }
        Command::TranscribeAll { meeting_id, model } => value(
            processing
                .recording(TranscribeRecordingInput {
                    id: meeting_id.to_string(),
                    model: model.to_string(),
                })
                .await?,
        ),
        Command::Capture {
            command: Capture::Settings {
                transcribe_after_stop,
            },
        } => {
            let service = AudioSettingsService::new(store.clone(), store.clone());
            let mut preferences = service.get().await?;
            if let Some(enabled) = transcribe_after_stop {
                preferences.transcribe_meetings_after_stop = enabled;
                service.save(&preferences).await?;
            }
            value(preferences)
        }
        Command::Capture {
            command: Capture::Recover { directory },
        } => value(
            parakeet_application::capture::CaptureService::new(native, guarded_audio.clone())
                .recover(&directory)
                .await?,
        ),
        Command::Capture {
            command:
                Capture::Record {
                    microphone_uid,
                    system,
                    title,
                    seconds,
                    transcribe,
                },
        } => value(
            meeting
                .record(
                    parakeet_application::capture::CaptureRequest {
                        microphone_uid,
                        system_audio: system,
                        title,
                        maximum_seconds: seconds,
                        final_transcription: None,
                    },
                    transcribe,
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                )
                .await?,
        ),
        Command::Transcribe(args) => value(processing.run(args.request()).await?),
        Command::Speakers { command } => match command {
            Speakers::Clusters {
                recording,
                track,
                run,
            } => value(
                parakeet_application::cluster_edits::ClusterEditingService::new(
                    store.clone(),
                    store.clone(),
                    ownership.clone(),
                )
                .inspect(parakeet_application::cluster_edits::InspectClustersInput {
                    recording_id: recording,
                    track_id: track,
                    run_id: run,
                })
                .await?,
            ),
            Speakers::ReviewSuggestion { id } => value(speakers.review_suggestion(&id).await?),
            Speakers::Candidates { meeting } => value(speakers.candidates(&meeting).await?),
            Speakers::List => value(speakers.profiles().await?),
            Speakers::Create { name } => value(speakers.create(&name).await?),
            Speakers::Rename { id, name } => {
                speakers
                    .rename(RenameSpeakerInput {
                        id: id.clone(),
                        name: name.clone(),
                    })
                    .await?;
                Ok(json!({"renamed":id}))
            }
            Speakers::Delete { id } => {
                speakers.delete(&id).await?;
                Ok(json!({"deleted":id}))
            }
            Speakers::Examples { profile } => value(speakers.examples(profile.as_deref()).await?),
            Speakers::RemoveExample { id } => {
                speakers.remove_example(&id).await?;
                Ok(json!({"removed":id}))
            }
            Speakers::Diarize {
                meeting_id,
                track,
                start_ms,
                duration_ms,
            } => value(
                processing
                    .run(ProcessRequest {
                        recording_id: meeting_id,
                        track_id: track,
                        model: "selected-diarization".into(),
                        start_ms,
                        duration_ms,
                        word_timestamps: false,
                    })
                    .await?,
            ),
            Speakers::Enroll {
                profile,
                meeting,
                track,
                start_ms,
                end_ms,
                confirmed,
            } => value(
                speakers
                    .enroll(Enrollment {
                        profile_id: profile,
                        recording_id: meeting,
                        track_id: track,
                        start_ms,
                        end_ms,
                        confirmed,
                    })
                    .await?,
            ),
            Speakers::Suggest {
                meeting,
                track,
                start_ms,
                end_ms,
                minimum_score,
                minimum_margin,
                minimum_examples,
            } => value(
                speakers
                    .suggest(SuggestSpeakerInput {
                        recording_id: meeting.to_string(),
                        track_id: track.to_string(),
                        start_ms,
                        end_ms,
                        policy: MatchPolicy {
                            minimum_score,
                            minimum_margin,
                            minimum_examples,
                        },
                    })
                    .await?,
            ),
            Speakers::Assign { segment, profile } => value(
                speakers
                    .correct(CorrectSegmentInput {
                        segment_id: segment.to_string(),
                        profile_id: Some(profile),
                        text: None,
                        action: ("assign").to_string(),
                    })
                    .await?,
            ),
            Speakers::Reject { segment } => value(
                speakers
                    .correct(CorrectSegmentInput {
                        segment_id: segment.to_string(),
                        profile_id: None,
                        text: None,
                        action: ("reject").to_string(),
                    })
                    .await?,
            ),
            Speakers::Unknown { segment } => value(
                speakers
                    .correct(CorrectSegmentInput {
                        segment_id: segment.to_string(),
                        profile_id: None,
                        text: None,
                        action: ("unknown").to_string(),
                    })
                    .await?,
            ),
            Speakers::Text { segment, text } => value(
                speakers
                    .correct(CorrectSegmentInput {
                        segment_id: segment.to_string(),
                        profile_id: None,
                        text: Some(text),
                        action: ("text").to_string(),
                    })
                    .await?,
            ),
            Speakers::SplitCluster {
                recording,
                run,
                speaker,
                at_ms,
            } => value(
                parakeet_application::cluster_edits::ClusterEditingService::new(
                    store.clone(),
                    store.clone(),
                    ownership.clone(),
                )
                .edit(parakeet_application::cluster_edits::EditClustersInput {
                    recording_id: recording,
                    run_id: run,
                    edit: parakeet_application::cluster_edits::ClusterEdit::Split {
                        speaker,
                        at_ms,
                    },
                })
                .await?,
            ),
            Speakers::MergeClusters {
                recording,
                run,
                speakers,
            } => value(
                parakeet_application::cluster_edits::ClusterEditingService::new(
                    store.clone(),
                    store.clone(),
                    ownership.clone(),
                )
                .edit(parakeet_application::cluster_edits::EditClustersInput {
                    recording_id: recording,
                    run_id: run,
                    edit: parakeet_application::cluster_edits::ClusterEdit::Merge { speakers },
                })
                .await?,
            ),
            Speakers::UndoClusterEdit { recording, run } => value(
                parakeet_application::cluster_edits::ClusterEditingService::new(
                    store.clone(),
                    store.clone(),
                    ownership.clone(),
                )
                .edit(parakeet_application::cluster_edits::EditClustersInput {
                    recording_id: recording,
                    run_id: run,
                    edit: parakeet_application::cluster_edits::ClusterEdit::Undo,
                })
                .await?,
            ),
            Speakers::Suggestions { recording } => {
                value(speakers.suggestions(recording.as_deref()).await?)
            }
            Speakers::ResolveSuggestion {
                id,
                action,
                segment,
                profile,
            } => value(
                speakers
                    .resolve_suggestion(ResolveSuggestionInput {
                        id,
                        action,
                        segment_id: segment,
                        profile_id: profile,
                    })
                    .await?,
            ),
            Speakers::Undo { segment } => value(speakers.undo(&segment).await?),
        },
        Command::Prompts { command } => match command {
            Prompts::List => value(settings.prompts().await?),
            Prompts::Put { file, authorize } => {
                let mut p: PromptProfile = serde_json::from_slice(&tokio::fs::read(file).await?)?;
                p.authorized = authorize;
                settings.save_prompt(&p).await?;
                value(p)
            }
            Prompts::Revoke { id } => value(settings.revoke(&id).await?),
            Prompts::Test {
                bundle_id,
                text,
                dry_run,
            } => value(
                settings
                    .transform(TransformTextInput {
                        bundle_id: bundle_id.to_string(),
                        raw: text.to_string(),
                        dry_run,
                    })
                    .await?,
            ),
        },
        Command::Shortcuts { command } => match command {
            Shortcuts::List => value(settings.shortcuts().await?),
            Shortcuts::Set { file } => {
                let bindings: Vec<Shortcut> =
                    serde_json::from_slice(&tokio::fs::read(file).await?)?;
                settings.save_shortcuts(&bindings).await?;
                value(bindings)
            }
        },
        Command::Media {
            command: Media::Import { url },
        } => value(
            MediaService::new(media, guarded_audio.clone(), store)
                .import(&url)
                .await?,
        ),
        Command::Media {
            command: Media::Transcribe { url, model },
        } => {
            let receipt = MediaTranscriptionService::new(
                Arc::new(MediaService::new(
                    media,
                    guarded_audio.clone(),
                    store.clone(),
                )),
                processing.clone(),
            )
            .transcribe(TranscribeMediaInput { url, model })
            .await?;
            if receipt.transcription_failure.is_some() {
                return Err(IncompleteMedia(receipt).into());
            }
            value(receipt)
        }
        Command::Benchmark {
            command: Benchmark::Run { suite },
        } => Ok(parakeet_application::evaluation::EvaluationService::new(
            Arc::new(parakeet_infrastructure::evaluation::EvaluationFiles {
                root: root.join("evaluations"),
            }),
            source,
            store,
            processing,
        )
        .run(&suite)
        .await?),
        _ => unreachable!(),
    };
    pool.close().await;
    result
}
#[derive(Debug)]
struct IncompleteMedia(MediaTranscription);
impl std::fmt::Display for IncompleteMedia {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Audio retained as {}; transcription failed: {}",
            self.0.recording.id,
            self.0
                .transcription_failure
                .as_deref()
                .unwrap_or("incomplete")
        )
    }
}
impl std::error::Error for IncompleteMedia {}
fn kind(error: &anyhow::Error) -> &'static str {
    if error.downcast_ref::<IncompleteCleanup>().is_some() {
        return "storage";
    }
    if error.downcast_ref::<IncompleteMedia>().is_some() {
        return "processing";
    }
    for cause in error.chain() {
        macro_rules! classify {($($type:ty),*)=>{$(if let Some(error)=cause.downcast_ref::<$type>(){let kind=error.kind();if kind!="nested"{return kind;}})*};}
        classify!(
            parakeet_application::cleanup::CleanupError,
            parakeet_application::storage::InspectStorageError,
            parakeet_application::storage::ReadRetentionPolicyError,
            parakeet_application::storage::SaveRetentionPolicyError,
            parakeet_application::storage::PreviewRetentionError,
            ImportRecordingsError,
            parakeet_application::scoring::ScoreEvaluationError,
            parakeet_application::audio_locations::FingerprintAudioError,
            parakeet_application::audio_locations::RelinkAudioError,
            ListRecordingsError,
            ShowRecordingError,
            RenameRecordingError,
            InspectTracksError,
            ExportRecordingError,
            DiscoverRecordingsError,
            MediaImportError,
            TranscribeMediaError,
            ImportLegacyError,
            PlayAudioError,
            PlaySegmentError,
            ListModelsError,
            InstallModelError,
            VerifyModelError,
            RemoveModelError,
            SelectModelError,
            SelectEmbeddingModelError,
            SelectedModelsError,
            TranscribeRecordingError,
            RetryProcessingError,
            parakeet_application::jobs::ListProcessingJobsError,
            parakeet_application::jobs::RecoverProcessingJobsError,
            ListSpeakersError,
            ListSuggestionsError,
            ReviewSuggestionError,
            parakeet_application::cluster_edits::InspectClustersError,
            parakeet_application::cluster_edits::EditClustersError,
            ResolveSuggestionError,
            ListVoiceExamplesError,
            CreateSpeakerError,
            RenameSpeakerError,
            DeleteSpeakerError,
            RemoveVoiceExampleError,
            EnrollSpeakerError,
            SuggestSpeakerError,
            CorrectSegmentError,
            UndoCorrectionError,
            EnrollmentCandidatesError,
            ListPromptsError,
            SavePromptError,
            RevokePromptError,
            ListShortcutsError,
            SaveShortcutsError,
            TransformTextError,
            ReadAudioPreferencesError,
            SaveAudioPreferencesError,
            CaptureRecordingError,
            RecoverCaptureError,
            ListAudioDevicesError,
            InspectPermissionsError,
            ListRecoverableCapturesError,
            parakeet_application::dictation::DictateSpeechError,
            parakeet_application::dictation::RecoverInsertionError,
            parakeet_application::dictation::ListDictationsError,
            parakeet_application::dictation::ShowDictationError,
            parakeet_application::dictation::ListenForDictationError,
            parakeet_application::evaluation::RunEvaluationError,
            parakeet_application::evaluation::ReadEvaluationReportError
        );
        if let Some(e) = cause.downcast_ref::<parakeet_application::meeting::RecordMeetingError>() {
            return e.kind();
        }
        if let Some(e) = cause.downcast_ref::<parakeet_application::meeting::FinalizeMeetingError>()
        {
            return e.kind();
        }
        if let Some(e) =
            cause.downcast_ref::<parakeet_application::processing_control::ControlProcessingError>()
        {
            return e.kind();
        }
        if cause.is::<ValidationError>() {
            return "invalid_input";
        }
        if let Some(error) = cause.downcast_ref::<ProcessingError>() {
            return match error {
                ProcessingError::Cancelled(_) => "cancelled",
                ProcessingError::NotFound(_) => "not_found",
                ProcessingError::Unavailable(_) => "source",
                ProcessingError::Conflict(_) => "conflict",
                ProcessingError::Invalid(_) => "invalid_input",
                ProcessingError::Persistence(_) => "persistence",
                ProcessingError::Engine(_) => "model",
            };
        }
        if cause.is::<CapabilityError>() {
            return "capability";
        }
    }
    "operation_failed"
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                print!("{e}");
                return;
            }
            println!(
                "{}",
                json!({"schema_version":1,"ok":false,"error":{"kind":"invalid_arguments","message":e.to_string()}})
            );
            std::process::exit(2)
        }
    };
    match execute(cli).await {
        Ok(data) => println!("{}", json!({"schema_version":1,"ok":true,"data":data})),
        Err(error) => {
            let kind = kind(&error);
            let retained = error
                .downcast_ref::<IncompleteMedia>()
                .map(|e| json!(e.0))
                .or_else(|| {
                    error
                        .downcast_ref::<IncompleteCleanup>()
                        .map(|e| json!(e.0))
                })
                .or_else(|| {
                    error.chain().find_map(|e| {
                        e.downcast_ref::<parakeet_application::meeting::FinalizeMeetingError>()
                            .and_then(|e| e.retained())
                    })
                })
                .or_else(|| {
                    error.chain().find_map(|e| {
                        e.downcast_ref::<parakeet_application::meeting::RecordMeetingError>()
                            .and_then(|e| e.retained_recording_id())
                            .map(|id| json!({"recording_id":id}))
                    })
                });
            println!(
                "{}",
                json!({"schema_version":1,"ok":false,"retained_result":retained,"error":{"kind":kind,"message":error.to_string(),"causes":error.chain().map(ToString::to_string).collect::<std::collections::BTreeSet<_>>()}})
            );
            std::process::exit(match kind {
                "invalid_input" | "invalid_arguments" => 2,
                "not_found" => 3,
                "confirmation_required" => 4,
                "conflict" => 5,
                "cancelled" => 6,
                _ => 1,
            });
        }
    }
}

#[derive(Debug)]
struct IncompleteCleanup(parakeet_application::cleanup::CleanupReview);
impl std::fmt::Display for IncompleteCleanup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cleanup needs attention; files retained; inspect review {}",
            self.0.id
        )
    }
}
impl std::error::Error for IncompleteCleanup {}
fn cleanup_value(
    review: parakeet_application::cleanup::CleanupReview,
) -> anyhow::Result<serde_json::Value> {
    if review.state == parakeet_application::cleanup::CleanupState::NeedsAttention {
        return Err(IncompleteCleanup(review).into());
    }
    value(review)
}
