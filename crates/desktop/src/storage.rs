use super::*;
use parakeet_application::storage::StorageArea;
fn size_label(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GiB", bytes as f64 / (1024. * 1024. * 1024.))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024. * 1024.))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.)
    } else {
        format!("{bytes} bytes")
    }
}
fn area_label(area: StorageArea) -> &'static str {
    match area {
        StorageArea::Database => "Library database",
        StorageArea::RecordedAudio => "Recordings and media",
        StorageArea::Models => "Local models",
        StorageArea::Processing => "Processing files",
        StorageArea::Playback => "Playback files",
        StorageArea::Recovery => "Recovery files",
        StorageArea::Evaluations => "Evaluation reports",
        StorageArea::Other => "Other files",
    }
}
impl Workbench {
    pub(super) fn measure_storage(&mut self, cx: &mut Context<Self>) {
        if self.storage_busy {
            return;
        }
        self.storage_busy = true;
        let service = self.storage.clone();
        self.task(
            "Measuring local storage without changing files",
            async move {
                Ok(Update::StorageUsage(
                    service
                        .usage()
                        .await
                        .map(Arc::new)
                        .map_err(|e| e.to_string()),
                ))
            },
            cx,
        );
    }
    pub(super) fn storage_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div().flex().flex_col().gap_3().child(section(
            "Storage",
            "How much space Parakeet Local uses on this Mac.",
        ));
        let Some(usage) = &self.storage_usage else {
            return view.child(caption(if self.storage_busy {
                "Measuring…"
            } else {
                "Nothing measured yet."
            }));
        };
        let mut summary = panel().child(
            div()
                .flex()
                .items_start()
                .gap_6()
                .flex_wrap()
                .child(stat(
                    size_label(usage.owned_totals.logical_bytes),
                    "used by your library",
                ))
                .when_some(usage.volume_available_bytes, |row, n| {
                    row.child(stat(size_label(n), "free on this Mac"))
                })
                .child(div().flex_1())
                .child(
                    button("refresh-storage", "Measure again")
                        .disabled(self.storage_busy)
                        .on_click(cx.listener(|this, _, _, cx| this.measure_storage(cx))),
                ),
        );
        summary = summary.child(caption(usage.root.clone()));
        if !usage.complete {
            summary = summary.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(pill("Incomplete", WARNING))
                    .child(caption(
                        "Some files could not be measured, so these totals are a little low.",
                    ))
                    .when(!usage.warnings.is_empty(), |row| {
                        row.child(
                            quiet(
                                "storage-path-warnings",
                                if self.storage_show_warnings {
                                    "Hide which"
                                } else {
                                    "Show which"
                                },
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.storage_show_warnings = !this.storage_show_warnings;
                                cx.notify();
                            })),
                        )
                    }),
            );
            if self.storage_show_warnings {
                for warning in usage.warnings.iter().take(5) {
                    summary = summary.child(caption(warning.clone()));
                }
            }
        }
        view = view.child(summary);
        let mut areas: Vec<_> = usage
            .areas
            .iter()
            .filter(|(_, s)| s.logical_bytes > 0)
            .collect();
        areas.sort_by_key(|(_, s)| std::cmp::Reverse(s.logical_bytes));
        let largest = areas
            .first()
            .map(|(_, s)| s.logical_bytes)
            .unwrap_or(1)
            .max(1);
        let mut rows = panel();
        for (index, (area, size)) in areas.iter().enumerate() {
            rows = rows.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(index > 0, |row| {
                        row.pt_3().border_t_1().border_color(rgb(BORDER))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(area_label(**area)),
                            )
                            .child(div().child(size_label(size.logical_bytes))),
                    )
                    .child(
                        div()
                            .h(px(6.))
                            .w_full()
                            .rounded_full()
                            .bg(rgb(ELEVATED))
                            .child(
                                div()
                                    .h_full()
                                    .rounded_full()
                                    .bg(rgb(0x6b7488))
                                    .w(relative(size.logical_bytes as f32 / largest as f32)),
                            ),
                    )
                    .child(caption(format!("{} files", size.files))),
            );
        }
        view = view.child(group(
            "What takes the space",
            "Models are the largest part. Original recordings are kept for 7 days, then replaced by a lossless copy.",
        ));
        view = view.child(rows);
        if usage.referenced_external_file_bytes > 0 || usage.unresolved_audio_references > 0 {
            view = view.child(caption(format!(
                "Audio stored outside the library folder: {}{}.",
                size_label(usage.referenced_external_file_bytes),
                if usage.unresolved_audio_references > 0 {
                    format!(
                        " · {} referenced recordings are missing",
                        usage.unresolved_audio_references
                    )
                } else {
                    String::new()
                }
            )));
        }
        view
    }
}

impl Workbench {
    /// Raw captures stay untouched for a week, then become verified lossless copies. Runs once at
    /// startup; anything that fails verification is left exactly as it was.
    pub(super) fn archive_old_captures(&mut self, cx: &mut Context<Self>) {
        let service = self.archive.clone();
        self.background_task(
            "Compressing raw recordings older than a week",
            async move {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis() as u64;
                let report = service.archive_due(now, false).await?;
                Ok(Update::Message(if report.archived.is_empty() && report.failed.is_empty() {
                    "Ready".into()
                } else {
                    let saved: u64 = report
                        .archived
                        .iter()
                        .map(|a| a.bytes_before.saturating_sub(a.bytes_after))
                        .sum();
                    format!(
                        "Compressed {} raw recordings older than a week ({} saved, no quality lost); {} left unchanged",
                        report.archived.len(),
                        size_label(saved),
                        report.failed.len()
                    )
                }))
            },
            cx,
        );
    }
}
