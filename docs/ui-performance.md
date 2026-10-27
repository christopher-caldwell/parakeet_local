# Native UI performance observations

The harness follows Smaller Talk at `19d69563f979a8a80c311fbc0389453f440a390a`: explicit opt-in recording, isolated data, semantic phases, executable hashes, and nonsuspending Darwin `proc_pid_rusage` accounting. It reuses the same vendored GPUI profiler APIs rather than treating application render-construction time as frame latency. No provider, chat or service functionality was copied.

Build with `CARGO_BUILD_JOBS=2 scripts/build-app --release --performance`. The default build remains uninstrumented. Runtime observation requires an explicitly prepared directory; ordinary launches do not open an evidence channel. All output is local, and raw traces belong under ignored `.local/`.

```
python3 scripts/ui-performance.py prepare --out .local/performance/trial
PARAKEET_PERFORMANCE_DIR="$PWD/.local/performance/trial" scripts/run-desktop <isolated-existing-data-root>
python3 scripts/ui-performance.py record --out .local/performance/trial \
  --pid <app-pid> --binary '<absolute-bundle>/Contents/MacOS/ParakeetLocal' \
  --data-root <isolated-existing-data-root> --label baseline
python3 scripts/ui-performance.py phase --out .local/performance/trial --kind idle idle
python3 scripts/ui-performance.py phase --out .local/performance/trial --kind scroll long-transcript-scroll
python3 scripts/ui-performance.py stop --out .local/performance/trial
# Quit the app normally to flush its observer, then:
python3 scripts/ui-performance.py report --out .local/performance/trial
```

The recorder observes; phase commands do not perform UI actions. Native scrolling, navigation, finding and processing use the actual application controls. `PARAKEET_APP_BUNDLE` selects a preserved benchmark bundle when using `scripts/run-desktop`. It does not change database provisioning rules. `scripts/prepare-ui-fixture.py --directory <new-path>` generates a deterministic 2,000-segment transcript without audio or identities; import it through the ordinary `meetings import` operation.

Use the same window dimensions, data snapshot, page, text, model settings and scroll sequence on both builds. Mark idle, open, repeated down/up scrolling, find, inference and post-close phases. Wait for content and allow multiple resource samples. Repeat captures; warm filesystem/font caches, host load and trial order can affect the result. The harness is intended for targeted diagnosis, not an automatic release verdict.

Each resource sample records per-process RSS, physical footprint, cumulative user/system CPU time and process start identity. CPU percentages are interval deltas; 100% is one logical CPU. App descendants are tracked separately and observed reparented descendants retain their identity checks. Missing reads are recorded as failures, never zero usage. Summed footprint is accounting rather than deduplicated physical memory. GPU/ANE utilization is not fully observed.

The app records full GPUI draw phases, submission, frame work relative to the real refresh budget, dirty-to-present duration and cumulative input-to-frame latency histograms. Input histogram phase boundaries have up to one observer interval of uncertainty. Sparse input or manual pauses are not low FPS. Actual scrolling must produce at least ten frames and three observed offsets; empty/missing metrics remain null. CPU/frame statistics are instrumented observations, not proof of every displayed pixel or a physical keyboard test.

A bounded background writer handles trace files; the UI observer takes snapshots without forcing a redraw. Transcript text, names, audio and embeddings are not logged by the app observer. Numeric context includes segment/page counts, window bounds, pending work and scroll offset. Process paths and isolated data roots are still private local metadata. No trace is uploaded.

The candidate reader uses GPUI's variable-height `ListState`, with 300 points of overscan and exact initial measurement of its existing 40-row page. It prepares corrected display state and case-folded search text in the background task that receives a recording, then recomputes matching indices only when data or find input changes. New results rebuild this derived state; original segments and corrections stay unchanged. Page/data/editor/word-evidence changes invalidate row measurements, preserving the logical top for content edits. The active editor's focus handle is registered with the list so scrolling does not strand an active input.

Copy transcript still exports the complete corrected recording, and row Copy exports its full segment independently of rendering. Native text selection operates on mounted rows; selection spanning rows while they are recycled and complete VoiceOver traversal need separate verification. A scrolling benchmark does not establish those behaviors.

CPU counters require Mach timebase conversion on Apple Silicon. This sampler converts ticks using `mach_timebase_info`; a controlled calibration compared converted CPU deltas against Python's process CPU clock. The initial baseline capture mislabeled raw Mach ticks as nanoseconds; its original samples remain intact and an explicit `accounting-correction.json` applies the measured 125/3 scale when regenerating that report. Frame/RSS/footprint measurements were unaffected. Apple's [rusage implementation](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/bsd_kern.c#L1187) assigns task power counters to these fields; the installed SDK and local CPU calibration were also inspected. A missing unit declaration now requires an explicit correction receipt rather than silently using the old numbers.
