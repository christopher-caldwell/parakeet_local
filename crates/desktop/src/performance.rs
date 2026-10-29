//! Explicit opt-in observation of the real window, not a separate UI workload.
//! Pattern: Smaller Talk's local benchmark evidence + GPUI frame instrumentation.
use super::*;
use gpui::profiler::{FrameEvent, FrameTimingCollector};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{BufWriter, Write},
    os::unix::fs::OpenOptionsExt,
    sync::{atomic::AtomicU64, mpsc},
    thread::JoinHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) struct Recorder {
    sender: mpsc::SyncSender<Value>,
    dropped: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
fn unix_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
impl Recorder {
    pub(super) fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(root) = std::env::var_os("PARAKEET_PERFORMANCE_DIR") else {
            return Ok(None);
        };
        let root = PathBuf::from(root).canonicalize()?;
        let config: Value = serde_json::from_slice(&std::fs::read(root.join("benchmark.json"))?)?;
        anyhow::ensure!(config["protocol"] == 1, "unsupported performance protocol");
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(root.join("app-events.jsonl"))?;
        let (sender, receiver) = mpsc::sync_channel::<Value>(64);
        let dropped = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (lost, ending) = (dropped.clone(), stop.clone());
        gpui::profiler::set_trace_enabled(true);
        let mut collector = FrameTimingCollector::new();
        let worker = std::thread::Builder::new().name("performance-evidence".into()).spawn(move || {
            let mut writer = BufWriter::new(file);
            let mut pending = HashMap::new();
            let mut emit = |kind: &str, at: f64, data: Value| {
                // Evidence failures are explicit; no silently successful benchmark.
                writeln!(writer, "{}", json!({"event":kind,"at_unix_s":at,"data":data}))
                    .expect("performance evidence write failed");
            };
            emit("started", unix_seconds(), json!({"protocol":1,"pid":std::process::id(),"release":!cfg!(debug_assertions)}));
            loop {
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(data) => emit("ui", unix_seconds(), data),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                for event in collector.collect_unseen() {
                    match event {
                        FrameEvent::Draw(frame) => { pending.insert(frame.window_id, frame); }
                        FrameEvent::Present(presentation) => {
                            if let Some(frame) = pending.remove(&presentation.window_id) {
                                let at = unix_seconds() - presentation.present_end.elapsed().as_secs_f64();
                                let ms = |d: Duration| d.as_secs_f64() * 1000.;
                                emit("frame", at, json!({
                                    "window":format!("{:?}", frame.window_id),
                                    "draw_ms":ms(frame.draw_duration()),
                                    "request_layout_ms":ms(frame.phases.request_layout),
                                    "prepaint_ms":ms(frame.phases.prepaint),
                                    "paint_ms":ms(frame.phases.paint),
                                    "present_ms":ms(presentation.present_duration()),
                                    "dirty_to_present_ms":frame.dirty_at.map(|t|ms(presentation.present_end.saturating_duration_since(t))),
                                    "work_ms":frame.work(presentation.present_start).map(ms),
                                    "late":frame.is_late(presentation.present_start),
                                    "missed_refreshes":frame.missed_refreshes(presentation.present_start),
                                    "refresh_ms":frame.refresh_interval.map(ms),
                                    "animation_interval_ms":presentation.animation_interval.map(ms)
                                }));
                            }
                        }
                    }
                }
                if ending.load(Ordering::Relaxed) {
                    emit("finished", unix_seconds(), json!({"dropped_ui_samples":lost.load(Ordering::Relaxed)}));
                    break;
                }
            }
            writer.flush().expect("performance evidence flush failed");
        })?;
        Ok(Some(Self {
            sender,
            dropped,
            stop,
            worker: Some(worker),
        }))
    }
    fn observe(&self, sample: Value) {
        if self.sender.try_send(sample).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(super) fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("performance evidence worker failed");
        }
        gpui::profiler::set_trace_enabled(false);
    }
}
impl Workbench {
    pub(super) fn start_performance_observer(&self, window: &Window, cx: &mut Context<Self>) {
        if self.performance.is_none() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                if this.update_in(cx, |this, window, _| {
                    let Some(recorder) = &this.performance else { return; };
                    let input = window.input_latency_snapshot();
                    let input_buckets = input.latency_histogram.iter_recorded()
                        .map(|b| (b.value_iterated_to(), b.count_at_value())).collect::<Vec<_>>();
                    recorder.observe(json!({
                        "area":match this.area { Area::Library=>"library",Area::Speakers=>"speakers",Area::Models=>"models",Area::Dictation=>"dictation",Area::Capture=>"capture",Area::Jobs=>"jobs",Area::Privacy=>"privacy",Area::Storage=>"storage" },
                        "segments":this.detail.as_ref().map(|d|d.segments.len()),
                        "corrections":this.detail.as_ref().map(|d|d.corrections.len()),
                        "render_count":this.render_count,
                        "render_construction_peak_ms":this.render_peak_ms,
                        "pending":this.pending,"query_characters":this.transcript_query.chars().count(),
                        "activity_refresh_running":this.heartbeat_running,
                        "activity_refresh_ticks":this.heartbeat_ticks,
                        "scroll_y":f32::from(this.reader_list.scroll_px_offset_for_scrollbar().y),
                        "max_scroll_y":f32::from(this.reader_list.max_offset_for_scrollbar().y),
                        "scroll_item":this.reader_list.logical_scroll_top().item_ix,
                        "scroll_offset_in_item":f32::from(this.reader_list.logical_scroll_top().offset_in_item),
                        "input_latency_buckets_ns":input_buckets,
                        "mid_draw_input_events_dropped":input.mid_draw_events_dropped,
                        "window_width":f32::from(window.bounds().size.width),
                        "window_height":f32::from(window.bounds().size.height)
                    }));
                }).is_err() { break; }
            }
        }).detach();
    }
}
