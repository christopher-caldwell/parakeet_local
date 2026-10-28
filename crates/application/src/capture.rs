use super::*;
use std::sync::atomic::AtomicBool;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureRecordingInput {
    pub microphone_uid: Option<String>,
    pub system_audio: bool,
    pub title: String,
    pub maximum_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_transcription: Option<crate::meeting::MeetingFinalization>,
}
impl CaptureRecordingInput {
    pub fn validate(&self) -> Result<(), ValidationError> {
        Recording::validate_title(&self.title)?;
        if let Some(plan) = &self.final_transcription {
            plan.validate()?;
        }
        if self
            .microphone_uid
            .as_ref()
            .is_some_and(|uid| uid.trim().is_empty())
        {
            return Err(ValidationError("microphone UID cannot be empty".into()));
        }
        if self.microphone_uid.is_none() && !self.system_audio {
            return Err(ValidationError(
                "select an explicit microphone and/or system audio".into(),
            ));
        }
        if self.maximum_seconds == 0 || self.maximum_seconds > 7200 {
            return Err(ValidationError(
                "capture limit must be 1–7200 seconds".into(),
            ));
        }
        Ok(())
    }
}
#[async_trait]
pub trait AudioCapture: Send + Sync {
    fn current_status(&self) -> Value;
    async fn devices(&self) -> Result<Value, CapabilityError>;
    async fn permissions(&self) -> Result<Value, CapabilityError>;
    async fn record(
        &self,
        request: &CaptureRecordingInput,
        stop: Arc<AtomicBool>,
    ) -> Result<ImportBundle, CapabilityError>;
    async fn recoverable(&self) -> Result<Value, CapabilityError>;
    async fn recover(&self, directory: &Path) -> Result<ImportBundle, CapabilityError>;
}
operation_error!(CaptureRecordingError,sources{Invalid:ValidationError=>"invalid_input",Source:CapabilityError=>"source",Persistence:PersistenceError=>"persistence"},values{});
operation_error!(RecoverCaptureError,sources{Source:CapabilityError=>"source",Persistence:PersistenceError=>"persistence"},values{});
operation_error!(ListAudioDevicesError,sources{Source:CapabilityError=>"source"},values{});
operation_error!(InspectPermissionsError,sources{Source:CapabilityError=>"source"},values{});
operation_error!(ListRecoverableCapturesError,sources{Source:CapabilityError=>"source"},values{});
pub struct CaptureService {
    capture: Arc<dyn AudioCapture>,
    writes: Arc<dyn LibraryWrites>,
}
impl CaptureService {
    pub fn status(&self) -> Value {
        self.capture.current_status()
    }
    pub fn new(capture: Arc<dyn AudioCapture>, writes: Arc<dyn LibraryWrites>) -> Self {
        Self { capture, writes }
    }
    pub async fn recover(&self, path: &Path) -> Result<ImportBundle, RecoverCaptureError> {
        let bundle = self.capture.recover(path).await?;
        let mut bundle = bundle;
        if let Some(value) = bundle
            .recording
            .metadata
            .get("requested_final_transcription")
            .filter(|v| !v.is_null())
        {
            let plan: crate::meeting::MeetingFinalization =
                serde_json::from_value(value.clone()).map_err(CapabilityError::new)?;
            plan.attach(&mut bundle).map_err(CapabilityError::new)?;
        }
        self.writes.import(&bundle).await?;
        Ok(bundle)
    }
    pub async fn record(
        &self,
        request: CaptureRecordingInput,
        stop: Arc<AtomicBool>,
    ) -> Result<ImportBundle, CaptureRecordingError> {
        request.validate()?;
        let mut bundle = self.capture.record(&request, stop).await?;
        if let Some(plan) = &request.final_transcription {
            plan.attach(&mut bundle)?;
        }
        self.writes.import(&bundle).await?;
        Ok(bundle)
    }
}

pub struct AudioInspectionService {
    capture: Arc<dyn AudioCapture>,
}
impl AudioInspectionService {
    pub fn new(capture: Arc<dyn AudioCapture>) -> Self {
        Self { capture }
    }
    pub async fn devices(&self) -> Result<Value, ListAudioDevicesError> {
        Ok(self.capture.devices().await?)
    }
    pub async fn permissions(&self) -> Result<Value, InspectPermissionsError> {
        Ok(self.capture.permissions().await?)
    }
    pub async fn recoverable(&self) -> Result<Value, ListRecoverableCapturesError> {
        Ok(self.capture.recoverable().await?)
    }
}
