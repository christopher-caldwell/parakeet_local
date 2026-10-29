use super::*;
use crate::library::time_label;
impl Workbench {
    pub(super) fn monitor_jobs(&mut self, cx: &mut Context<Self>) {
        if self.job_monitor_running {
            return;
        }
        self.job_monitor_running = true;
        let service = self.recovery.clone();
        let library = self.library.clone();
        let analysis = self.analysis.clone();
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let mut first = true;
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
                let keep = this.update(cx, |this, _| {
                    if first {
                        first = false;
                        return true;
                    }
                    if !this.busy() && this.area != Area::Jobs && this.pending_jobs.is_empty() && !this.analysis_active {
                        this.job_monitor_running = false;
                        false
                    } else {
                        true
                    }
                });
                if !matches!(keep, Ok(true)) { break; }
                let service = service.clone();
                let result = runtime.spawn(async move { service.pending().await }).await;
                let Ok(Ok(jobs)) = result else {
                    let _ = this.update(cx, |this, cx| {
                        this.job_monitor_running = false;
                        this.last_error = Some("Unable to refresh processing progress; use Refresh pending jobs to retry".into());
                        cx.notify();
                    });
                    break;
                };
                let selected = this.update(cx, |this, _| this.requested_recording.clone()).ok().flatten();
                let activity = if let Some(id) = selected.clone() {
                    let analysis = analysis.clone();
                    runtime.spawn(async move { analysis.is_active(&id).await }).await.ok().and_then(Result::ok)
                } else { None };
                let ended = this.update(cx, |this, cx| {
                    let controller_ended = selected == this.requested_recording
                        && activity == Some(false) && this.analysis_active;
                    let mut changed = serde_json::to_value(&jobs).ok()
                        != serde_json::to_value(&this.pending_jobs).ok();
                    if selected == this.requested_recording && let Some(active) = activity {
                        changed |= this.analysis_active != active;
                        this.analysis_active = active;
                    }
                    if !changed {
                        // Unchanged progress must not redraw the window every tick.
                        return None;
                    }
                    let ended = this.detail.as_ref().filter(|detail| {
                        controller_ended || this.pending_jobs.iter().any(|run| run.recording_id == detail.recording.id && !jobs.iter().any(|job| job.id == run.id))
                    }).map(|detail| detail.recording.id.clone());
                    this.pending_jobs = jobs;
                    cx.notify();
                    ended
                });
                match ended {
                    Ok(Some(id)) => {
                        let library = library.clone();
                        let result = runtime.spawn(async move { library.show(&id).await.map(reader::Document::new) }).await;
                        let _ = this.update(cx, |this, cx| {
                            match result {
                                Ok(Ok(detail)) => this.receive_detail(detail, cx),
                                _ => this.last_error = Some("Processing stopped, but its recording could not be refreshed. Reopen the recording to retry.".into()),
                            }
                            // A background refresh must preserve cancellation/failure feedback.
                            cx.notify();
                        });
                    },
                    Ok(None) => {},
                    Err(_) => break,
                }
            }
        }).detach();
    }
    pub(super) fn processing_job(&self, run: &ProcessingRun, cx: &mut Context<Self>) -> Div {
        let p = &run.result["progress"];
        let requested = run.configuration["cancellation_requested_at"].is_string();
        let stage = if requested {
            "Cancellation requested · waiting for the worker to stop"
        } else {
            p["stage"].as_str().unwrap_or("preparing")
        };
        let id = run.id.clone();
        div()
            .p_3()
            .bg(rgb(PANEL))
            .rounded_lg()
            .flex()
            .flex_col()
            .gap_2()
            .child(format!("{} · {}", run.model, stage.replace('_', " ")))
            .child(caption(format!(
                "{} / {} audio processed · {} saved chunks{}",
                time_label(p["audio_completed_ms"].as_i64()),
                time_label(p["audio_total_ms"].as_i64()),
                p["completed_chunks"].as_u64().unwrap_or(0),
                p["current_chunk_percent"]
                    .as_u64()
                    .map(|n| format!(" · current ASR chunk {n}%"))
                    .unwrap_or_default()
            )))
            .when(
                run.configuration["cancellation_mode"] == "at_engine_boundary",
                |row| {
                    row.child(caption(
                        "This runtime stops at its next safe engine boundary.",
                    ))
                },
            )
            .child(
                button(
                    format!("cancel-job-{}", run.id),
                    if requested {
                        "Cancellation requested"
                    } else {
                        "Cancel processing"
                    },
                )
                .disabled(requested)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let service = this.job_control.clone();
                    let id = id.clone();
                    this.task(
                        "Requesting cancellation",
                        async move {
                            let r = service.cancel(&id).await?;
                            Ok(Update::Message(if r.requested {
                                "Cancellation requested. Source audio and saved work are retained."
                                    .into()
                            } else {
                                format!("Job already {}", r.status)
                            }))
                        },
                        cx,
                    );
                })),
            )
    }
}
