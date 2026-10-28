use super::*;
use parakeet_application::storage::StorageArea;
use parakeet_domain::storage::{RetentionPolicy, StorageProtection};
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
fn policy_label(p: &RetentionPolicy) -> String {
    p.completed_scratch_after_days
        .map(|d| format!("Processing files from jobs completed at least {d} days ago"))
        .unwrap_or_else(|| "Keep all processing files".into())
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
fn reason_label(reason: StorageProtection) -> &'static str {
    use StorageProtection::*;
    match reason {
        EligibleCompletedScratch => "Completed processing files",
        KeepAll => "Keep-all policy",
        ActiveRun => "Active processing",
        RecoverableRun => "Interrupted or failed processing",
        UnknownRun => "Unknown processing ownership",
        PendingMeeting => "Unfinished meeting transcription",
        SpeakerReference => "Saved speaker examples",
        ManualCorrection => "Manual corrections",
        CompletionTimeUnknown => "Unknown completion time",
        TooRecentOrFuture => "Recent work or clock discrepancy",
        RecordedAudio => "Recording files",
        Database => "Library database",
        Models => "Local model data",
        Recovery => "Recovery evidence",
        Playback => "Playback may be active",
        OtherFiles => "Other files",
        UncertainArtifact => "Unrecognized or partial files",
        LinkedAudio => "Referenced source audio",
        MultipleHardLinks => "Files with multiple links",
    }
}
pub(super) fn cleanup_state_label(
    state: parakeet_application::cleanup::CleanupState,
) -> &'static str {
    use parakeet_application::cleanup::CleanupState::*;
    match state {
        Reviewed => "Ready to stage",
        Applying => "Moving to recovery",
        Staged => "In recovery",
        Restoring => "Restoring",
        Restored => "Restored",
        NeedsAttention => "Needs attention",
    }
}
fn review_date(raw: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%b %-d, %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| raw.chars().take(20).collect())
}
impl Workbench {
    pub(super) fn preview_storage(
        &mut self,
        policy: Option<RetentionPolicy>,
        cx: &mut Context<Self>,
    ) {
        if self.storage_busy {
            return;
        }
        self.storage_busy = true;
        let service = self.storage.clone();
        let cleanup = self.cleanup.clone();
        self.task(
            "Measuring local storage without changing files",
            async move {
                let result = async {
                    Ok((
                        Arc::new(service.preview(policy).await.map_err(|e| e.to_string())?),
                        cleanup.list().await.map_err(|e| e.to_string())?,
                    ))
                }
                .await;
                Ok(Update::StoragePreview(result))
            },
            cx,
        );
    }
    pub(super) fn storage_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div().flex().flex_col().gap_3().child(section(
            "Storage",
            "Inspect usage, review completed processing files, and restore staged files.",
        ));
        view = view.child(
            button("refresh-storage", "Refresh saved-policy preview")
                .self_start()
                .disabled(self.storage_busy)
                .on_click(cx.listener(|this, _, _, cx| this.preview_storage(None, cx))),
        );
        if let Some(policy) = &self.storage_policy {
            view = view.child(caption(format!(
                "Saved: {} · revision {}",
                policy_label(&policy.policy),
                policy.revision
            )));
        }
        view = view.child(
            div()
                .flex()
                .gap_2()
                .items_center()
                .flex_wrap()
                .child(
                    button("preview-keep-all", "Preview keep all")
                        .disabled(self.storage_busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.preview_storage(Some(RetentionPolicy::default()), cx)
                        })),
                )
                .child(
                    div()
                        .w(px(140.))
                        .flex()
                        .flex_col()
                        .child(caption("Age in days"))
                        .child(Input::new(&self.storage_days)),
                )
                .child(
                    button("preview-storage-age", "Preview by age")
                        .disabled(self.storage_busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            match this.storage_days.read(cx).value().parse::<u16>() {
                                Ok(days) => this.preview_storage(
                                    Some(RetentionPolicy {
                                        completed_scratch_after_days: Some(days),
                                    }),
                                    cx,
                                ),
                                Err(_) => {
                                    this.last_error =
                                        Some("Enter a whole number of days from 1 to 3650".into());
                                    cx.notify();
                                }
                            }
                        })),
                ),
        );
        view = view.child(self.cleanup_panel(cx));
        let Some(preview) = &self.storage_preview else {
            return view.child(caption(
                "Refresh to measure storage. A saved policy does not enable automatic cleanup.",
            ));
        };
        view=view.child(section(if preview.hypothetical{"Hypothetical retention preview"}else{"Saved-policy preview"},policy_label(&preview.effective_policy)))
            .child(format!("{} potential completed files · {} file size · {} allocated blocks",preview.candidate_totals.files,size_label(preview.candidate_totals.logical_bytes),size_label(preview.candidate_totals.allocated_bytes)))
            .child(caption("Original audio, models, speaker references, corrections, and active or recoverable work stay protected. This preview does not authorize deletion."));
        if preview.effective_policy != preview.saved_policy.policy {
            let service = self.storage.clone();
            let policy = preview.effective_policy.clone();
            let revision = preview.saved_policy.revision;
            view = view.child(
                button("save-retention-policy", "Save this preview policy")
                    .self_start()
                    .disabled(self.storage_busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.storage_busy = true;
                        let service = service.clone();
                        let policy = policy.clone();
                        this.task(
                            "Saving preview policy only",
                            async move {
                                Ok(Update::StoragePolicySaved(
                                    service
                                        .save_policy(policy, revision)
                                        .await
                                        .map_err(|e| e.to_string()),
                                ))
                            },
                            cx,
                        );
                    })),
            );
        }
        let usage = &preview.usage;
        view=view.child(div().p_4().rounded_lg().bg(rgb(PANEL)).flex().flex_col().gap_2()
            .child(format!("{} in {} unique files · {} allocated blocks",size_label(usage.owned_totals.logical_bytes),usage.owned_totals.files,size_label(usage.owned_totals.allocated_bytes)))
            .when_some(usage.volume_available_bytes,|row,n|row.child(caption(format!("{} available on this volume",size_label(n)))))
            .child(caption(usage.root.clone()))
            .child(caption("File sizes and allocated blocks are different measurements. Shared APFS extents and actual freed space are not measured.")));
        for (area, size) in &usage.areas {
            view = view.child(
                div()
                    .flex()
                    .gap_3()
                    .justify_between()
                    .child(area_label(*area))
                    .child(caption(format!(
                        "{} · {} unique files",
                        size_label(size.logical_bytes),
                        size.files
                    ))),
            );
        }
        view=view.child(caption(format!("Referenced audio outside this root: {} · {} missing or unresolved references. External bytes are not added to the root total.",size_label(usage.referenced_external_file_bytes),usage.unresolved_audio_references)));
        if usage.database_outside_root_bytes > 0 {
            view = view.child(caption(format!(
                "Selected database outside this root: {}",
                size_label(usage.database_outside_root_bytes)
            )));
        }
        if !usage.complete {
            view = view.child(pill(
                "Some paths could not be measured; totals are incomplete",
                WARNING,
            ));
        }
        if !usage.warnings.is_empty() {
            view = view.child(
                button(
                    "storage-path-warnings",
                    if self.storage_show_warnings {
                        "Hide path warnings"
                    } else {
                        "Show path warnings"
                    },
                )
                .self_start()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.storage_show_warnings = !this.storage_show_warnings;
                    cx.notify();
                })),
            );
            if self.storage_show_warnings {
                for warning in usage.warnings.iter().take(5) {
                    view = view.child(caption(warning.clone()).text_color(rgb(WARNING)));
                }
            }
        }
        for (reason, size) in &preview.protected {
            view = view.child(caption(format!(
                "Protected · {} · {} files · {}",
                reason_label(*reason),
                size.files,
                size_label(size.logical_bytes)
            )));
        }
        for candidate in preview.candidates.iter().take(40) {
            view = view.child(caption(format!(
                "{} · {}",
                candidate.relative_path,
                size_label(candidate.logical_bytes)
            )));
        }
        if preview.candidates.len() > 40 {
            view = view.child(caption(format!(
                "Showing 40 of {} paths. Copy the full preview to inspect every path.",
                preview.candidates.len()
            )));
        }
        let preview = preview.clone();
        view.child(
            button("copy-storage-preview", "Copy preview JSON")
                .self_start()
                .on_click(cx.listener(move |this, _, _, cx| {
                    let preview = preview.clone();
                    this.task(
                        "Preparing full storage preview",
                        async move {
                            Ok(Update::CopyText(serde_json::to_string_pretty(
                                preview.as_ref(),
                            )?))
                        },
                        cx,
                    );
                })),
        )
    }
}

impl Workbench {
    pub(super) fn run_cleanup(
        &mut self,
        id: String,
        digest: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.storage_busy {
            return;
        }
        self.storage_busy = true;
        let service = self.cleanup.clone();
        self.task(
            "Updating reviewed processing files",
            async move {
                let result = async {
                    let review = if let Some(digest) = digest {
                        service.apply(&id, &digest, true).await
                    } else {
                        service.restore(&id).await
                    }
                    .map_err(|e| e.to_string())?;
                    Ok((
                        service.list().await.map_err(|e| e.to_string())?,
                        Some(review),
                    ))
                }
                .await;
                Ok(Update::CleanupReviews(result))
            },
            cx,
        );
    }
    fn cleanup_panel(&self, cx: &mut Context<Self>) -> Div {
        use parakeet_application::cleanup::CleanupState;
        let mut view=div().p_4().rounded_lg().bg(rgb(PANEL)).flex().flex_col().gap_2()
            .child(section("Reviewed cleanup", "Stage completed processing files in recovery, then restore when needed. Staging frees no disk space; permanent deletion is not available."));
        let policy = self
            .storage_preview
            .as_ref()
            .map(|p| p.effective_policy.clone());
        view = view.child(
            button("review-cleanup", "Review up to 40 eligible files")
                .self_start()
                .disabled(self.storage_busy || policy.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.storage_busy {
                        return;
                    }
                    this.storage_busy = true;
                    let service = this.cleanup.clone();
                    let policy = policy.clone();
                    this.task(
                        "Hashing a concrete cleanup review",
                        async move {
                            let result = async {
                                let review = service
                                    .review(policy, 40)
                                    .await
                                    .map_err(|e| e.to_string())?;
                                Ok((service.list().await.map_err(|e| e.to_string())?, review))
                            }
                            .await;
                            Ok(Update::CleanupReviews(result))
                        },
                        cx,
                    );
                })),
        );
        if !self.cleanup_reviews.is_empty() {
            view = view.child(caption(
                "Recent reviews · choose one to inspect its exact file list",
            ));
        }
        for review in &self.cleanup_reviews {
            let id = review.id.clone();
            let label = format!(
                "{} · {} {} · {}",
                cleanup_state_label(review.state),
                review.files.len(),
                if review.files.len() == 1 {
                    "file"
                } else {
                    "files"
                },
                review_date(&review.reviewed_at)
            );
            view = view.child(
                button(SharedString::from(format!("cleanup-{id}")), label)
                    .self_start()
                    .disabled(self.storage_busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.cleanup_selected =
                            this.cleanup_reviews.iter().find(|r| r.id == id).cloned();
                        this.confirm_action = None;
                        cx.notify();
                    })),
            );
        }
        if let Some(review) = &self.cleanup_selected {
            view = view
                .child(caption(format!(
                    "Review {} · {} · {}",
                    review.id,
                    cleanup_state_label(review.state),
                    size_label(review.files.iter().map(|f| f.bytes).sum())
                )))
                .child(caption(format!("SHA-256 review digest: {}", review.digest)));
            for file in &review.files {
                view = view.child(caption(format!(
                    "{} · {}",
                    file.relative_path,
                    size_label(file.bytes)
                )));
            }
            for error in &review.errors {
                view = view.child(pill(error.clone(), WARNING));
            }
            let id = review.id.clone();
            let digest = review.digest.clone();
            if review.state == CleanupState::Reviewed {
                view = view.child(
                    button("stage-cleanup", "Move reviewed files to recovery…")
                        .self_start()
                        .disabled(self.storage_busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.confirm_action = Some(shell::ConfirmAction::StageCleanup {
                                id: id.clone(),
                                digest: digest.clone(),
                            });
                            cx.notify();
                        })),
                );
            } else if review.state != CleanupState::Restored {
                view = view.child(
                    button("restore-cleanup", "Restore retained files")
                        .self_start()
                        .disabled(self.storage_busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_cleanup(id.clone(), None, cx)
                        })),
                );
            }
        }
        view
    }
}
