//! Refresh activity feedback while work is active; leave an idle library asleep.
use super::*;

impl Workbench {
    pub(super) fn start_heartbeat(&mut self, cx: &mut Context<Self>) {
        if self.heartbeat_running || (self.pending == 0 && self.metrics_path.is_none()) {
            return;
        }
        self.heartbeat_running = true;
        // A deliberately sleeping reader is not a stalled 100 ms heartbeat.
        self.heartbeat_last = Instant::now();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                let result = this.update(cx, |this, cx| {
                    if this.pending == 0 && this.metrics_path.is_none() {
                        this.heartbeat_running = false;
                        return None;
                    }
                    this.heartbeat_ticks += 1;
                    this.heartbeat_max_ms = this
                        .heartbeat_max_ms
                        .max(this.heartbeat_last.elapsed().as_secs_f64() * 1000.);
                    this.heartbeat_last = Instant::now();
                    if this.capture_stop.is_some()
                        || this.dictation_stop.is_some()
                        || (this.area == Area::Models && this.pending > 0)
                    {
                        cx.notify();
                    }
                    let metrics = this.metrics_path.as_ref().map(|path| {
                        (
                            path.clone(),
                            serde_json::json!({
                                "max_100ms_heartbeat_interval_ms":this.heartbeat_max_ms,
                                "render_construction_peak_ms":this.render_peak_ms,
                                "render_count":this.render_count,
                                "background_jobs":this.pending,
                                "activity_refresh_ticks":this.heartbeat_ticks,
                            }),
                        )
                    });
                    Some((this.runtime.clone(), metrics))
                });
                let Ok(Some((runtime, metrics))) = result else {
                    break;
                };
                if let Some((path, metrics)) = metrics {
                    // At most one diagnostics write is in flight, even on a slow disk.
                    let _ = runtime
                        .spawn(async move {
                            let _ = tokio::fs::write(path, metrics.to_string()).await;
                        })
                        .await;
                }
            }
        })
        .detach();
    }
}
