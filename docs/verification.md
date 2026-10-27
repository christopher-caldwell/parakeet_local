# Implementation and verification — October 27, 2026

This is a working initial implementation with real local processing, not a completed or human-accepted replacement. No private audio, transcripts, embeddings or model weights are committed. Complete private receipts remain in ignored `.local/`; the portable public numeric report is [here](../benchmarks/results/2026-10-27-public-summary.json).

## Delivered surfaces

Six Rust packages implement Domain, Application, SQLx Database, Infrastructure, JSON CLI and GPUI Desktop boundaries. CLI and desktop compose the same application services. The working surfaces cover import/library/search/title overrides/export/playback, versioned local ASR and diarization, model installation and verification, speaker profiles and confirmed examples, conservative suggestions and correction history, derived cluster edits, capture and recovery, multi-binding dictation, retained insertion failures, disabled-by-default prompt routes and YouTube metadata/download/transcription.

The desktop provides library/transcript navigation, direct title editing, per-segment playback and corrections, speaker enrollment/example management, model controls with download progress, source/device/capture health, editable simultaneous bindings, retained dictation text and explicit profile authorization. Detailed suggestion resolution and cluster editing currently use CLI commands. See [README commands](../README.md#useful-commands).

## Source inventory and real processing

Read-only inspection found six meeting artifact folders and six meeting database rows, plus four media database rows. Five meetings retain both original tracks; one retains neither. All six contain diarization claims and alignment metadata, with usable retained-source alignment for five. Retained sources are M4A; legacy names and malformed fixtures also have automated coverage. Recording durations range from 12m45s to 92m51s, median approximately 38m28s. One system track has a severe metadata/decoded-duration discrepancy; its new output preserves track-local evidence without inventing meeting timing.

All ten source records were imported into the independent development library. Re-import added zero duplicate meetings. All ten retained raw tracks now have completed full-track Whisper base and Nemotron fast runs: **20 completed processing runs**, with no failed source runs in the final corpus batch. The missing meeting remains explicitly unavailable. Four earlier completed runs were reused; sixteen new runs completed in that batch. Fourteen enrollment candidates were prepared across five meetings. Imported labels and all current personal predictions remain unverified.

Summed engine wall time for these full-track runs was 321.71 seconds for Whisper base and 107.32 seconds for Nemotron fast; these exclude import/verification overhead. The maximum measured native helper RSS was 488,144,896 bytes, not total Rust-plus-GPU memory. An earlier full 13-minute speakrs run reached approximately 2.76–2.96 GB process-lifetime RSS. Long-file memory, model loading and resource contention remain practical concerns.

The final source audit found **no changes in 48 original artifact size/mtime fingerprints or recorded JSON hashes**. Source database access was read-only and WAL-aware. This is a scoped integrity check, not a byte-for-byte hash of every original audio file. Receipts: `.local/source-integrity-final.json`, `library-processing-report.json`, `final-personal-summary.json` and `all-enrollment-candidates.json`.

## Public model measurements

All six registry models were installed and explicitly checksum-verified. Versions, formats and licenses are documented in [audio models](audio-models.md). There was no cloud transcription or external AI enhancement.

| Evaluation | Measured result | Limit |
|---|---|---|
| VoxConverse development, three short clips | Nemotron fast weighted DER 4.37%; offline 4.42% | Preset comparison only |
| Separate three-clip VoxConverse held-out subset | Nemotron fast DER **6.29%**; speakrs Core ML **11.12%** | Zero collar, overlap included; tiny subset |
| LibriSpeech ASR development, 212 words | Base and small each 5 errors, WER 2.36% | Six clean utterances |
| Disjoint ASR held-out, 201 words | Base 9 errors, WER **4.48%**; small 6 errors, **2.99%** | Base selection fixed before held-out run |
| LibriSpeech identification, held-out | 5/6 known accepted correctly; all 6 unfamiliar Unknown; zero observed false acceptances | Three enrolled dataset identities, two references each; clean read speech |

The identification policy (0.65 score, 0.05 margin, two independent examples) was selected on separate tuning utterances. CPU and Core ML matched on this tiny probe. It is **not enabled as a global personal threshold**. Exact provenance, hashes and per-clip DER/JER/counting results are in the public report and ignored full receipts. See [scoring methodology](evaluation-methodology.md).

A strict earlier recognition smoke remains a known failed quality gate: after fixing padded-silence contamination, same/different-voice scores improved to 0.873/0.824, but its unchanged 0.9 threshold rejects both. `benchmarks/recognition-public.py` therefore does not pass its recall assertion. The separate calibrated probe does not erase that result or establish personal meeting accuracy.

Eight-second synthetic silence, tone and seeded noise also exposed real ASR hallucinations. Base produced 1/0/2 false words; small produced 2/1/2. Every nonempty output was flagged uncertain and retained; empty reference WER is undefined. Dictation tests verify that flagged output is withheld from automatic enhancement/insertion. This safeguard reduces one failure path; it does not prove all hallucinations are detected. Receipt: `.local/non-speech-benchmark.json`.

## Automated and integration gates

Final `cargo test --workspace` passes **41 tests**: Application 2, CLI integration 4, Database 11, Domain 16, Infrastructure 8. Coverage includes timeline/overlap/unknown invariants, gesture ownership and cancellation, matching compatibility and duplicate-reference suppression, SQL migrations/mapping, foreign-database protection, idempotency, corrections and undo, import variations, authorization, bounded subprocess timeouts and model leases. A concurrent-process descriptor inheritance failure was repaired with explicit lease unlock; the final suite passes. These are behavioral tests, not evidence of physical key delivery.

`scripts/cargo fmt --all -- --check`, Clippy with `-D warnings`, SQLx offline metadata check, database integrity verification and `scripts/build-app` pass. The build retains upstream `block 0.1.6` future-compatibility and Swift CBLAS/preconcurrency warnings. The application bundle is locally ad-hoc signed. Evidence: `.local/tests-final.log`, `clippy.log`, `sqlx-check.log`, `db-verify-final.json`, `model-verification-final.json`, `build-final.log`.

Real YouTube metadata, download and local transcription succeeded for public Big Buck Bunny media; its explicit title override survived refresh/reprocessing. A separate missing-model test returned failure plus the retained imported recording, without downloading an ASR model implicitly. No personal recordings were sent to media services.

## Hardware and interface evidence

Test machine: Apple Silicon Mac mini Mac14,12, 16 GB RAM, macOS 26.3.1. M3 MacBook Air has not been exercised.

| Capability | Evidence | Still unverified |
|---|---|---|
| System audio | Actual 48 kHz capture of public speech, peak 0.783, first buffer 188 ms; local ASR persisted | Real meeting and simultaneous mic/system alignment |
| Live capture health | Seven live reports, peak 0.446; audio retained | Bluetooth/device changes during actual mic capture |
| Writer crash recovery | SIGKILL produced explicit failure; 833,536-byte CAF remained unchanged, recovered and reopened; gaps marked unknown | Broader power-loss/fault matrix |
| Microphone selection | Real device enumeration; unavailable UID rejected before asking permission | Actual microphone capture; permission remains not determined |
| Native shortcut tap | Tap creation and parent-death cleanup exercised; simulated gesture tests pass | Physical built-in/external key down/up; generated CUA keys did not reach tap |
| Text insertion | Native AX insertion succeeded in disposable TextEdit document; changed-focus insertion rejected | Other intended editors and clipboard fallback |
| GPUI | Real title edit, playback, processing, model install/progress/select and disabled prompt draft exercised | Subjective acceptance, complete keyboard accessibility |
| UI under work | Approximately 220–222 ms maximum observed 100 ms heartbeat interval; render work around 1 ms or less in short probes | Sustained load/thermal test and comparison with MacParakeet |

The synthetic prompt draft was disabled and unauthorized in an isolated UI test library. No actual remote route was invoked. The native dictation listener is stopped. Private hardware/UI receipts are under `.local/hardware*`, `crash-probe/`, `orphan-probe/`, `ui-probe/` and `textedit-insertion-result.json`.

## Remaining work and risk

The [first voice-identification checkpoint](human-checkpoints.md) is now complete: three user-confirmed exact clips produced three profiles with one verified example each. Twelve separate candidate intervals were compared using an explicit conservative diagnostic policy; all remained Unknown. No predictions were enrolled. Those comparison intervals lack verified identity labels, so the run does not measure recognition accuracy. More independent verified examples and held-out meeting labels are needed for useful calibration.

The follow-up also repaired desktop example loading: selecting a profile loads its examples, refresh picks up CLI changes, enrollment refreshes the list, and late responses for a different selected person are ignored. Personal names and enrollment/diagnostic receipts remain under ignored `.local/enrollment-checkpoint/` and in the local database.

Follow-up verification: all 41 workspace tests and Clippy pass again. The desktop was exercised against all three newly enrolled profiles; each selection loaded one saved example. Example controls are stacked to fit the reader width and interval labels use seconds. A rebuild exposed Finder metadata on the generated bundle that prevented ad-hoc signing; the build now removes only FinderInfo/ResourceFork attributes from its own build product before signing, preserving security attributes. The rebuilt bundle passes strict signature verification. Follow-up logs and confirmation/diagnostic receipts are in `.local/enrollment-checkpoint/`.

Independent follow-up remains: desktop suggestion/cluster-edit controls, word-aligned speaker-attributed accuracy, explicit moved-file relinking, recovery of model jobs left `running` after whole-process death, a contention/priority strategy beyond fail-busy leases, clipboard fallback, wider editor compatibility, broader benchmark corpora and long-running resource tests. Model loading is currently per operation. There is no claim that every requested feature is complete or that personal accuracy exceeds MacParakeet.
