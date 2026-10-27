use async_trait::async_trait;
use parakeet_application::{CapabilityError, settings::TextTransformation};
use parakeet_domain::PromptProfile;
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};
#[derive(Clone, Default)]
pub struct AuthorizedCli;
#[async_trait]
impl TextTransformation for AuthorizedCli {
    async fn transform(&self, p: &PromptProfile, raw: &str) -> Result<String, CapabilityError> {
        p.validate().map_err(CapabilityError::new)?;
        if !p.enabled || !p.authorized {
            return Err(CapabilityError::message("AI route is not authorized"));
        }
        if raw.len() > 1024 * 1024 || p.prompt.len() > 1024 * 1024 {
            return Err(CapabilityError::message("AI text or prompt exceeds 1 MiB"));
        }
        // No shell expansion. The configured CLI receives only this prompt and raw text, never voice profiles.
        let mut child = Command::new(&p.executable)
            .args(&p.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(CapabilityError::new)?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| CapabilityError::message("AI stdin unavailable"))?;
        let payload = format!("{}\n\n{}", p.prompt, raw);
        let mut writer = tokio::spawn(async move {
            input.write_all(payload.as_bytes()).await?;
            input.shutdown().await
        });
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CapabilityError::message("AI stdout unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CapabilityError::message("AI stderr unavailable"))?;
        let work = async {
            let read_out = async {
                let mut b = vec![];
                stdout
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut b)
                    .await
                    .map(|_| b)
            };
            let read_err = async {
                let mut b = vec![];
                stderr.take(65537).read_to_end(&mut b).await.map(|_| b)
            };
            let (out, err) = tokio::try_join!(read_out, read_err).map_err(CapabilityError::new)?;
            (&mut writer)
                .await
                .map_err(CapabilityError::new)?
                .map_err(CapabilityError::new)?;
            let status = child.wait().await.map_err(CapabilityError::new)?;
            if !status.success() {
                return Err(CapabilityError::message(format!(
                    "authorized CLI failed ({status}): {}",
                    String::from_utf8_lossy(&err)
                )));
            }
            if out.len() > 1024 * 1024 {
                return Err(CapabilityError::message("AI output exceeded 1 MiB"));
            }
            String::from_utf8(out).map_err(CapabilityError::new)
        };
        let result = tokio::time::timeout(Duration::from_secs(p.timeout_seconds), work).await;
        writer.abort();
        result.map_err(CapabilityError::new)?
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> PromptProfile {
        PromptProfile {
            id: "p".into(),
            bundle_id: "com.example".into(),
            application_name: "Example".into(),
            enabled: true,
            prompt: "prompt".into(),
            executable: "/bin/cat".into(),
            arguments: vec![],
            route: "local".into(),
            authorized: false,
            timeout_seconds: 2,
        }
    }
    #[tokio::test]
    async fn rejects_unapproved_and_executes_only_explicitly_authorized() {
        let mut p = profile();
        assert!(AuthorizedCli.transform(&p, "raw").await.is_err());
        p.authorized = true;
        assert_eq!(
            AuthorizedCli.transform(&p, "raw").await.unwrap(),
            "prompt\n\nraw"
        );
    }
    #[tokio::test]
    async fn timeout_also_bounds_a_child_that_does_not_read_stdin() {
        let mut p = profile();
        p.authorized = true;
        p.executable = "/bin/sleep".into();
        p.arguments = vec!["10".into()];
        p.timeout_seconds = 1;
        let start = std::time::Instant::now();
        assert!(
            AuthorizedCli
                .transform(&p, &"x".repeat(1024 * 1024))
                .await
                .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(4));
    }
}
