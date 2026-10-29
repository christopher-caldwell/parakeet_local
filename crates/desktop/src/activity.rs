//! Refresh activity feedback while work is active; leave an idle library asleep.
use super::*;

impl Workbench {
    fn live_activity(&self) -> bool {
        self.busy() || self.capture_stop.is_some() || self.dictation_stop.is_some()
    }
    pub(super) fn start_heartbeat(&mut self, cx: &mut Context<Self>) {
        if self.heartbeat_running || (!self.live_activity() && self.metrics_path.is_none()) {
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
                    if !this.live_activity() && this.metrics_path.is_none() {
                        this.heartbeat_running = false;
                        return None;
                    }
                    this.heartbeat_ticks += 1;
                    this.heartbeat_max_ms = this
                        .heartbeat_max_ms
                        .max(this.heartbeat_last.elapsed().as_secs_f64() * 1000.);
                    this.heartbeat_last = Instant::now();
                    // Only the screen showing live levels or progress needs redrawing.
                    if (this.capture_stop.is_some() && this.area == Area::Capture)
                        || (this.dictation_stop.is_some() && this.area == Area::Dictation)
                        || (this.area == Area::Models && this.busy())
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
