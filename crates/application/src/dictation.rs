use super::*;
use crate::{
    capture::{CaptureRequest, CaptureService},
    processing::ProcessingService,
    settings::{SettingsService, TransformResult},
};
use parakeet_domain::gestures::InsertionTarget;
use std::sync::atomic::{AtomicBool, Ordering};
#[derive(Clone, Default)]
pub struct DictationControl {
    pub stop: Arc<AtomicBool>,
    pub cancelled: Arc<AtomicBool>,
}
impl DictationControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationReceipt {
    pub id: String,
    pub recording_id: String,
    pub target: InsertionTarget,
    pub raw: String,
    pub enhanced: Option<String>,
    pub route: Option<String>,
    pub status: String,
    pub failure: Option<String>,
    pub created_at: String,
}
#[async_trait]
pub trait TextInsertion: Send + Sync {
    async fn insert(&self, target: &InsertionTarget, text: &str) -> Result<(), CapabilityError>;
}
#[async_trait]
pub trait DictationWrites: Send + Sync {
    async fn save_dictation(&self, receipt: &DictationReceipt) -> Result<(), PersistenceError>;
}
operation_error!(DictateSpeechError,sources{Capture:crate::capture::CaptureRecordingError=>"nested",Persistence:PersistenceError=>"persistence"},values{});
#[derive(Debug, Clone)]
pub struct DictateSpeechInput {
    pub microphone_uid: String,
    pub target: InsertionTarget,
}
pub struct DictationService {
    capture: Arc<CaptureService>,
    processing: Arc<ProcessingService>,
    settings: Arc<SettingsService>,
    insertion: Arc<dyn TextInsertion>,
    writes: Arc<dyn DictationWrites>,
}
impl DictationService {
    pub fn new(
        capture: Arc<CaptureService>,
        processing: Arc<ProcessingService>,
        settings: Arc<SettingsService>,
        insertion: Arc<dyn TextInsertion>,
        writes: Arc<dyn DictationWrites>,
    ) -> Self {
        Self {
            capture,
            processing,
            settings,
            insertion,
            writes,
        }
    }
    pub fn capture_status(&self) -> Value {
        self.capture.status()
    }
    pub async fn dictate(
        &self,
        input: DictateSpeechInput,
        control: DictationControl,
    ) -> Result<DictationReceipt, DictateSpeechError> {
        let microphone_uid = input.microphone_uid;
        let target = input.target;
        let bundle = self
            .capture
            .record(
                CaptureRequest {
                    microphone_uid: Some(microphone_uid),
                    system_audio: false,
                    title: format!("Dictation {}", now()),
                    maximum_seconds: 120,
                },
                control.stop.clone(),
            )
            .await?;
        let mut receipt = DictationReceipt {
            id: new_id(),
            recording_id: bundle.recording.id.clone(),
            target,
            raw: String::new(),
            enhanced: None,
            route: None,
            status: "cancelled".into(),
            failure: None,
            created_at: now(),
        };
        if !control.cancelled.load(Ordering::SeqCst) {
            let result = async {
                let runs = self
                    .processing
                    .recording(crate::processing::TranscribeRecordingInput {
                        id: bundle.recording.id.clone(),
                        model: "selected-asr".into(),
                    })
                    .await
                    .map_err(CapabilityError::new)?;
                receipt.raw = runs
                    .iter()
                    .flat_map(|r| r.result["raw"].as_array().into_iter().flatten())
                    .filter_map(|v| v["text"].as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
                    .trim()
                    .to_owned();
                if receipt.raw.is_empty() {
                    return Err(CapabilityError::message(
                        "recognition produced no text; nothing inserted",
                    ));
                }
                if control.cancelled.load(Ordering::SeqCst) {
                    receipt.status = "cancelled".into();
                    return Ok(());
                }
                if runs.iter().flat_map(|r|r.result["raw"].as_array().into_iter().flatten()).any(|segment|segment["evidence"]["uncertain_speech"]==true){
                    return Err(CapabilityError::message("Recognition includes low-energy or uncertain speech; raw text retained for review instead of automatic insertion"));
                }
                let TransformResult {
                    enhanced,
                    route,
                    failure,
                    ..
                } = self
                    .settings
                    .transform(crate::settings::TransformTextInput {
                        bundle_id: receipt.target.bundle_id.clone(),
                        raw: receipt.raw.clone(),
                        dry_run: false,
                    })
                    .await
                    .map_err(CapabilityError::new)?;
                receipt.enhanced = enhanced;
                receipt.route = route;
                if let Some(failure) = failure {
                    return Err(CapabilityError::message(format!(
                        "AI transformation failed; raw text retained: {failure}"
                    )));
                }
                if control.cancelled.load(Ordering::SeqCst) {
                    receipt.status = "cancelled".into();
                    return Ok(());
                }
                let text = receipt.enhanced.as_deref().unwrap_or(&receipt.raw);
                self.insertion.insert(&receipt.target, text).await?;
                receipt.status = "inserted".into();
                Ok::<_, CapabilityError>(())
            }
            .await;
            if let Err(error) = result {
                receipt.status = "retained_after_failure".into();
                receipt.failure = Some(error.to_string());
            }
        }
        self.writes.save_dictation(&receipt).await?;
        Ok(receipt)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ShortcutEvent {
    Down {
        binding: String,
        target: InsertionTarget,
    },
    Up {
        binding: String,
    },
    Cancel {
        reason: String,
    },
}
#[async_trait]
pub trait ShortcutStream: Send {
    async fn next_event(&mut self) -> Result<Option<ShortcutEvent>, CapabilityError>;
}
#[async_trait]
pub trait GlobalShortcuts: Send + Sync {
    async fn listen(
        &self,
        bindings: &[gestures::Shortcut],
    ) -> Result<Box<dyn ShortcutStream>, CapabilityError>;
}
pub trait DictationObserver: Send + Sync {
    fn event(&self, event: Value);
}
operation_error!(ListenForDictationError,sources{Settings:crate::settings::ListShortcutsError=>"nested",Source:CapabilityError=>"source"},values{NoBindings=>"invalid_input"});
pub struct DictationListener {
    shortcuts: Arc<dyn GlobalShortcuts>,
    settings: Arc<SettingsService>,
    dictation: Arc<DictationService>,
    observer: Arc<dyn DictationObserver>,
}
impl DictationListener {
    pub fn new(
        shortcuts: Arc<dyn GlobalShortcuts>,
        settings: Arc<SettingsService>,
        dictation: Arc<DictationService>,
        observer: Arc<dyn DictationObserver>,
    ) -> Self {
        Self {
            shortcuts,
            settings,
            dictation,
            observer,
        }
    }
    pub async fn run(
        &self,
        microphone_uid: String,
        stop: Arc<AtomicBool>,
    ) -> Result<(), ListenForDictationError> {
        let bindings = self
            .settings
            .shortcuts()
            .await
            .map_err(CapabilityError::new)?;
        if !bindings.iter().any(|b| b.enabled) {
            return Err(ListenForDictationError::NoBindings(
                "configure at least one enabled shortcut before listening".into(),
            ));
        }
        let mut events = self.shortcuts.listen(&bindings).await?;
        let mut gesture = gestures::HoldGesture::default();
        let began = std::time::Instant::now();
        let mut current: Option<(
            DictationControl,
            tokio::task::JoinHandle<Result<DictationReceipt, DictateSpeechError>>,
        )> = None;
        self.observer.event(json!({"state":"waiting"}));
        loop {
            if stop.load(Ordering::SeqCst) {
                if let Some((control, task)) = current.take() {
                    control.cancel();
                    let _ = task.await;
                }
                return Ok(());
            }
            tokio::select! {
             event=events.next_event()=>{let Some(event)=event?else{if let Some((control,task))=current.take(){control.cancel();let _=task.await;}return Err(CapabilityError::message("native shortcut stream ended").into());};
              match event{
               ShortcutEvent::Down{binding,target}=>{if current.is_none()&& let gestures::GestureEffect::Start(target)=gesture.down(&binding,target,began.elapsed().as_millis() as u64){let control=DictationControl::default();let job_control=control.clone();let service=self.dictation.clone();let uid=microphone_uid.clone();let task=tokio::spawn(async move{service.dictate(DictateSpeechInput{microphone_uid:uid,target},job_control).await});current=Some((control,task));self.observer.event(json!({"state":"starting","owner":binding}));}},
               ShortcutEvent::Up{binding}=>{if matches!(gesture.up(&binding),gestures::GestureEffect::Finish(_))&& let Some((control,_))=&current{control.stop.store(true,Ordering::SeqCst);self.observer.event(json!({"state":"transcribing"}));}},
               ShortcutEvent::Cancel{reason}=>{gesture.cancel();if let Some((control,_))=&current{control.cancel();}self.observer.event(json!({"state":"cancelled","reason":reason}));}
              }
             },
             result=async{match current.as_mut(){Some((_,task))=>task.await,None=>std::future::pending().await}}=>{current=None;gesture.cancel();match result{Ok(Ok(receipt))=>self.observer.event(json!({"state":receipt.status,"receipt":receipt})),Ok(Err(error))=>self.observer.event(json!({"state":"failed","message":error.to_string()})),Err(error)=>self.observer.event(json!({"state":"failed","message":error.to_string()}))}},
             _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{if current.as_ref().is_some_and(|(control,_)|!control.stop.load(Ordering::SeqCst)){let capture=self.dictation.capture_status();if capture["microphone"]["first_buffer_ready"]==true || (capture["event"]=="first_buffer" && capture["source"]=="microphone"){self.observer.event(json!({"state":"listening","capture":capture}));}}
              if matches!(gesture.tick(began.elapsed().as_millis() as u64),gestures::GestureEffect::Cancel){if let Some((control,_))=&current{control.cancel();}self.observer.event(json!({"state":"cancelled","reason":"lost_release_timeout"}));}}
            }
        }
    }
}

#[async_trait]
pub trait DictationQueries: Send + Sync {
    async fn dictations(&self) -> Result<Vec<DictationReceipt>, PersistenceError>;
    async fn dictation(&self, id: &str) -> Result<Option<DictationReceipt>, PersistenceError>;
}
operation_error!(ListDictationsError,sources{Persistence:PersistenceError=>"persistence"},values{});
operation_error!(ShowDictationError,sources{Persistence:PersistenceError=>"persistence"},values{NotFound=>"not_found"});
pub struct DictationHistoryService {
    queries: Arc<dyn DictationQueries>,
}
impl DictationHistoryService {
    pub fn new(queries: Arc<dyn DictationQueries>) -> Self {
        Self { queries }
    }
    pub async fn list(&self) -> Result<Vec<DictationReceipt>, ListDictationsError> {
        Ok(self.queries.dictations().await?)
    }
    pub async fn show(&self, id: &str) -> Result<DictationReceipt, ShowDictationError> {
        self.queries
            .dictation(id)
            .await?
            .ok_or_else(|| ShowDictationError::NotFound(id.into()))
    }
}
