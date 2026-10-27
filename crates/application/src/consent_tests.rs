use super::*;
use crate::settings::*;
use parakeet_domain::gestures::Shortcut;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
struct Store(Mutex<Vec<PromptProfile>>);
#[async_trait]
impl SettingsQueries for Store {
    async fn prompts(&self) -> Result<Vec<PromptProfile>, PersistenceError> {
        Ok(self.0.lock().unwrap().clone())
    }
    async fn shortcuts(&self) -> Result<Vec<Shortcut>, PersistenceError> {
        Ok(vec![])
    }
}
#[async_trait]
impl SettingsWrites for Store {
    async fn save_prompt(&self, p: &PromptProfile) -> Result<(), PersistenceError> {
        self.0.lock().unwrap().push(p.clone());
        Ok(())
    }
    async fn replace_shortcuts(&self, _: &[Shortcut]) -> Result<(), PersistenceError> {
        Ok(())
    }
}
#[derive(Default)]
struct Failure(AtomicUsize);
#[async_trait]
impl TextTransformation for Failure {
    async fn transform(&self, _: &PromptProfile, _: &str) -> Result<String, CapabilityError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(CapabilityError::message("deliberate failure"))
    }
}
fn profile() -> PromptProfile {
    PromptProfile {
        id: "p".into(),
        bundle_id: "com.example.target".into(),
        application_name: "Fixture".into(),
        enabled: true,
        prompt: "explicit test".into(),
        executable: "/bin/cat".into(),
        arguments: vec![],
        route: "local".into(),
        authorized: false,
        timeout_seconds: 5,
    }
}
fn input(bundle: &str, dry_run: bool) -> TransformTextInput {
    TransformTextInput {
        bundle_id: bundle.into(),
        raw: "Retain this original speech".into(),
        dry_run,
    }
}
#[tokio::test]
async fn unauthorized_wrong_target_and_dry_run_never_execute() {
    let store = Arc::new(Store::default());
    let adapter = Arc::new(Failure::default());
    let service = SettingsService::new(store.clone(), store.clone(), adapter.clone());
    store.0.lock().unwrap().push(profile());
    let r = service
        .transform(input("com.example.target", false))
        .await
        .unwrap();
    assert!(r.route.is_none());
    assert!(r.enhanced.is_none());
    store.0.lock().unwrap()[0].authorized = true;
    service
        .transform(input("com.example.other", false))
        .await
        .unwrap();
    service
        .transform(input("com.example.target", true))
        .await
        .unwrap();
    assert_eq!(adapter.0.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn failed_authorized_transformation_preserves_raw_text() {
    let store = Arc::new(Store::default());
    let adapter = Arc::new(Failure::default());
    let mut p = profile();
    p.authorized = true;
    store.0.lock().unwrap().push(p);
    let service = SettingsService::new(store.clone(), store, adapter.clone());
    let r = service
        .transform(input("com.example.target", false))
        .await
        .unwrap();
    assert_eq!(r.raw, "Retain this original speech");
    assert!(r.enhanced.is_none());
    assert!(r.failure.as_deref().unwrap().contains("deliberate failure"));
    assert_eq!(adapter.0.load(Ordering::SeqCst), 1);
}
