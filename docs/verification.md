# Implementation and verification — October 27, 2026

This is a working initial implementation with real local processing, not a completed or human-accepted replacement. No private audio, transcripts, embeddings or model weights are committed. Complete private receipts remain in ignored `.local/`; the portable public numeric report is [here](../benchmarks/results/2026-10-27-public-summary.json).

## Current status (October 28, after review pass 3)

This table is the authority; the dated records below keep the evidence behind each step, and where an older record disagrees, this table and the code win. Evidence categories follow B§22.

| Area | State | Evidence |
|---|---|---|
| Usable gate | Open | Needs the owner: a real AirPods meeting, a physically muted stretch, physical hold-to-dictate into another app, and naming voices in more meetings |
| Launch without a terminal | Desktop adds the Homebrew tool folders to its search path, remembers the library it was last started with, and shows startup failures in an alert | CLI `doctor` reproduced the missing-ffmpeg failure with a Finder-style PATH; the Finder launch itself is not yet observed |
| Editing while dictation, capture or archiving run | Long-lived work no longer holds the busy counter that disabled Save text, Undo, Save name and Detect speakers | Compiled; not yet exercised in the app with dictation on |
| Redraw cost | Job monitor redraws only when jobs or analysis activity change; the heartbeat redraws only the screen showing live levels | Code-traced; no new performance run |
| Learning from identifications | Every speaker decision removes the voice examples it contradicts and admits at most one for a confirmed clean segment, in one transaction; naming a voice learns only from played clips and never from speech assigned to someone else | SQLite-adapter tests (reassign, undo, Unknown, pre-corrected segment, heard clips) |
| Recognition criteria | Calibration on the owner's library (throwaway copy, counts only): 24 examples, 5 people; held-out 7 of 10 known voices recognized, 0 false accepts in 11 trials but only 1 unfamiliar-voice trial | Real data; too thin to turn suggestions on by default |
| Me / Them / Unknown | Microphone speech is "Me" only after the echo check covers it; otherwise "Microphone, not yet checked for echo". Speaker detection and the echo check run after Stop by default and are skipped with a stated reason when their models are missing | Unit (domain) and CLI contract |
| Modifier-only dictation shortcuts | Start after a 250 ms hold with no other key; another key cancels | Native self-test; physical keyboards untested |
| Dictations | Own kind, own Library filter, readable title | Unit (artifact parsing); no dictation has been recorded yet |
| Capture format | Microphone keeps its own channel count at 48 kHz or more; stable through format renegotiation | Native format, stall and route self-tests |
| Raw audio retention | 7 days, then WavPack with exact decoded-sample comparison | Synthetic audio plus SQLite tests; no real capture has aged seven days |
| Storage | Usage accounting and the archive only; the retention preview and storage lock were removed | CLI contract and SQLite tests |
| Usability pass (code review) | Shortcut recorder refuses Escape, bare Space and Command-letter keys and offers one-click modifier holds; Stop frees the recorder and dictation while the transcript finishes in the background; closed editors keep unsaved text; duplicate person names are refused; the transcript is one list; status messages are no longer erased by finished tasks | Unit (shortcut rules) and SQLite (duplicate names, record without a model); the app itself was not clicked through, and transcript scrolling is not re-measured |
| Automated gates | `just verify`: formatting, Clippy with warnings denied, all Rust tests, native route, hold, format and stall self-tests | Passing at the commit that added this table |

## Dated records

### Review follow-up — October 28

Two subsequent review findings are fixed:

- Finalizing converted audio no longer updates the last input-callback timestamp. The new native stall regression feeds 100 ms of synthetic audio at both 24 kHz and 48 kHz, then stops callbacks for 3.5 seconds. Both stalled cases retain the warning and all 4,800 stored frames; both normally finalized controls remain healthy. `just verify` includes this check.
- Name voices checks the analysis controller's ownership lease instead of treating a saved `running` label as proof of activity. An abandoned analysis offers **Resume detection**, which recovers only that recording's abandoned source jobs before resuming. The job monitor refreshes the recording when the controller finishes, including completion between source jobs. Real SQLite/filesystem-lease integration tests cover held versus released ownership, stale running metadata, recovery scoped to one recording, and successful resumption using deterministic inference.

Validation: `just verify` passed (155 Rust tests, four opt-in tests ignored, formatting, Clippy with warnings denied, seven routing cases, five format transitions and four stall/healthy cases). `just build` passed. The rebuilt app's interrupted-analysis state and enabled resume action were visually checked at 1,240- and 920-point window widths using an isolated private library copy; its tracks were marked externally owned to prevent archival writes to source files. Resumption itself was verified by integration tests, not by running models in that UI fixture. The app was then returned to the normal library on Record. No hardware stall or AirPods retest was performed in this follow-up. Private logs are under `.local/scratchpad/code-review/`.

### Fixes and app verification — October 28

The Usable gate remains open pending the human checks below. This section supersedes the earlier D2–D6 status. Changes were built into the macOS app bundle and inspected through computer use, including a 920-point-wide window.

- Microphone startup still requires actual buffers from the selected input before a meeting starts. The writer now converts changing input formats into one stable recording format and drains delayed samples at format changes, gaps and finalization. Five synthetic format transitions (24 kHz mono, 48 kHz stereo and 44.1 kHz stereo) retained exactly 24,000 frames, including the final buffer, with zero gaps or writer errors. This is native-buffer evidence, not an AirPods retest.
- A fresh app-driven 33.291-second webcam plus system-audio recording had zero interruptions, gaps or writer errors. Both tracks were 48 kHz. Public speech supplied the signal; transcription and speaker analysis completed automatically and two nameable voices appeared without manually refreshing. The earlier output-switch gap remains a known hardware limitation.
- Dictation now explains and disables startup without an enabled shortcut, checks microphone availability and permissions before starting, and retains errors inline. In the bundle, a temporary shortcut was recorded, the listener stayed enabled, and it was then stopped and the shortcut removed. Computer-generated held keys did not trigger a dictation; physical key delivery and the complete dictate/paste workflow remain unverified.
- Permission rows query the bundled helper and show their actual status. All four were visibly Granted in this run; no permissions were granted or revoked. Missing-permission navigation is implemented but was not exercised by revoking existing grants.
- Voice candidates reload after background completion with stale-response protection. The app distinguishes loading, running, failed and completed-with-no-candidates states. Real queue refresh, clip playback controls, skipping, persisted skips after restart, and restoring skipped voices were exercised. No identity was confirmed during this pass.
- Transcript correction and undo were exercised on the public-speech test recording. New meetings use readable local date/time titles. Record, Dictation, Permissions, Transcript and Name voices were visually inspected; narrower layouts use bounded controls and wrapping tabs. The Name voices form now scrolls so Save and Skip remain reachable. Advanced recording choices and troubleshooting sit under Details.
- Simultaneous microphone and system speech no longer causes microphone enrollment candidates to be discarded solely because their timestamps overlap. A regression test covers both sources speaking at once. This does not establish acoustic echo classification or personal speaker accuracy.
- Automatic archival now uses WavPack and requires exact decoded float sample bytes before replacing a raw recording. The former 24-bit FLAC conversion could change float samples. Per-attempt output names prevent concurrent cleanup from removing another attempt's copy. Synthetic audio and database tests cover exact decoding, changed-audio rejection, age and active-job exclusions, and preservation of an existing copy. A real capture aging through seven days has not been observed.

Validation: `just verify` passes formatting, Clippy with warnings denied, 155 Rust tests (four opt-in tests ignored), seven native route cases and the five-format native writer test. The signed app bundle builds successfully. Private receipts and logs are under `.local/scratchpad/takeover/`, including `fixes-summary.json`, `ux-final-verify.log` and `ux-build.log`.

Still required for the Usable gate: a real AirPods meeting with the owner's speech, a physically muted interval checked for invented words, source attribution during overlapping speech, and physical shortcut-to-paste dictation. This pass does not establish subjective stereo playback quality, M3 behavior, or complete visual coverage of People, import forms and every confirmation/error state.

### Current capture reliability checkpoint — October 28

The Usable gate remains open. This checkpoint supersedes older claims below that microphone capture has never been exercised; it does not establish reliable AirPods capture or personal transcription accuracy.

A read-only check of the reported failed recording found 27 microphone interruptions and 25 microphone gaps totaling about 46.1 seconds. The earlier handoff understated these counts. Two fresh AirPods probes differed: one delivered no microphone frames in 11.2 seconds, while another delivered continuous 24 kHz audio for 16.8 seconds. A USB webcam input initially failed at startup with native error -10868 while another input was the system default, but worked when it became the default. These observations do not prove a Bluetooth-profile cause.

The capture path now verifies buffers and effective microphone selection before starting system capture or emitting `capture_started`. Failure blocks the requested meeting; the successful check retains the same engine for recording. A native missing-device probe and a real app-driven webcam startup failure produced no start event, no audio files, and no new Library row. A subsequent AirPods startup check captured 9.5 seconds without gaps. Listed-device presence alone is never treated as successful initialization.

The native tap now explicitly uses the selected input's hardware rate and channel count, rather than the client format inherited from the previous default input. A real webcam comparison exposed two hardware channels but one inherited client channel; the revised tap records two. The health loop also recognizes an engine that has stopped without changing device UID and attempts reconnection to the same selected microphone. Apple's [input-node format contract](https://developer.apple.com/documentation/avfaudio/avaudioinputnode) and [configuration-change behavior](https://developer.apple.com/documentation/foundation/nsnotification/name-swift.struct/avaudioengineconfigurationchange) explain these implementation requirements. The earlier AirPods 24 kHz versus webcam 48 kHz transition has not yet been repeated with the corrected tap.

A separate root cause was reproduced with the webcam: changing the output device can make Core Audio expose an aggregate device containing the selected microphone and an output-only member. The former raw UID comparison treated that valid route as a disconnect and stopped a still-running engine. The corrected check accepts the aggregate only when its sole input is the selected microphone and that member is active. Seven deterministic native cases cover accepted and rejected compositions; `just verify` now runs them alongside the Rust checks.

The 256.1-second app-driven retest accepted 20 valid aggregate health checks without false disconnects. Switching the output back nevertheless caused one genuine engine stop, followed by recovery and a 2.576-second microphone gap; system audio had no gaps. Both saved tracks were 48 kHz. The app visibly reported audio saved with problems, the Library record remained `partial_needs_inspection`, and automatic transcription and speaker analysis completed. This is evidence that false disconnects were repaired and real interruptions are exposed, **not** evidence of gap-free output switching. The original AirPods failure still requires a headset retest.

A fresh 66.0-second recording with the webcam selected as both the app input and macOS default, and no device changes, had zero interruptions, zero gaps and zero writer errors on both tracks. Both files independently decoded as 48 kHz stereo. Public speech playback supplied the test signal; this was not a human speech or muted-microphone accuracy test. The rebuilt app's healthy Saved state was visually inspected after artifact verification.

Capture health now checks every requested source, retains interruption warnings across later healthy events, and waits for artifact inspection before reporting successful capture. Partial and recovered recordings carry a visible Library warning; empty captures and failed startup do not show green Saved. Native startup diagnostics remain local. The app's live warning, saved warning, Library warning, blocked start, and disabled microphone recording controls for a disconnected selected input were inspected through computer use. The owner clarified that warning alone was insufficient: unusable microphones must block startup.

Private receipts and build/test logs are in `.local/scratchpad/takeover/`. Human speech accuracy, a physically muted microphone, long AirPods meetings, M3 behavior, dictation keys, and the rest of the Usable workflow remain unverified by this checkpoint. D2–D6 had not been addressed at this earlier checkpoint; see the newer fixes and app verification section above.

Current validation: `just verify` passes 155 Rust tests (four opt-in tests ignored), formatting, Clippy with warnings denied, and seven native routing cases. The signed app bundle builds successfully. These automated results are separate from the hardware observations above.

### Delivered surfaces

Six Rust packages implement Domain, Application, SQLx Database, Infrastructure, JSON CLI and GPUI Desktop boundaries. CLI and desktop compose the same application services. The working surfaces cover import/library/search/title overrides/export/playback, versioned local ASR and diarization, model installation and verification, speaker profiles and confirmed examples, conservative suggestions and correction history, derived cluster edits, capture and recovery, multi-binding dictation, retained insertion failures, disabled-by-default prompt routes and YouTube metadata/download/transcription.

The desktop provides library/transcript navigation, direct title editing, per-segment playback and corrections, speaker enrollment/example management, model controls with download progress, source/device/capture health, editable simultaneous bindings, retained dictation text and explicit profile authorization. Identity-comparison review and decisions, pending-job inspection/recovery and recent-run retry are also available. Anonymous cluster inspection, representative playback, split/merge and undo are also available in GPUI. See [README commands](../README.md#useful-commands).

### Source inventory and real processing

Read-only inspection found six meeting artifact folders and six meeting database rows, plus four media database rows. Five meetings retain both original tracks; one retains neither. All six contain diarization claims and alignment metadata, with usable retained-source alignment for five. Retained sources are M4A; legacy names and malformed fixtures also have automated coverage. Recording durations range from 12m45s to 92m51s, median approximately 38m28s. One system track has a severe metadata/decoded-duration discrepancy; its new output preserves track-local evidence without inventing meeting timing.

All ten source records were imported into the independent development library. Re-import added zero duplicate meetings. All ten retained raw tracks now have completed full-track Whisper base and Nemotron fast runs: **20 completed processing runs**, with no failed source runs in the final corpus batch. The missing meeting remains explicitly unavailable. Four earlier completed runs were reused; sixteen new runs completed in that batch. Fourteen enrollment candidates were prepared across five meetings. Imported labels and all current personal predictions remain unverified.

Summed engine wall time for these full-track runs was 321.71 seconds for Whisper base and 107.32 seconds for Nemotron fast; these exclude import/verification overhead. The maximum measured native helper RSS was 488,144,896 bytes, not total Rust-plus-GPU memory. An earlier full 13-minute speakrs run reached approximately 2.76–2.96 GB process-lifetime RSS. Long-file memory, model loading and resource contention remain practical concerns.

The final source audit found **no changes in 48 original artifact size/mtime fingerprints or recorded JSON hashes**. Source database access was read-only and WAL-aware. This is a scoped integrity check, not a byte-for-byte hash of every original audio file. Receipts: `.local/source-integrity-final.json`, `library-processing-report.json`, `final-personal-summary.json` and `all-enrollment-candidates.json`.

### Public model measurements

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

### Automated and integration gates

The initial `cargo test --workspace` gate passed **41 tests**: Application 2, CLI integration 4, Database 11, Domain 16, Infrastructure 8. Coverage includes timeline/overlap/unknown invariants, gesture ownership and cancellation, matching compatibility and duplicate-reference suppression, SQL migrations/mapping, foreign-database protection, idempotency, corrections and undo, import variations, authorization, bounded subprocess timeouts and model leases. A concurrent-process descriptor inheritance failure was repaired with explicit lease unlock; the final suite passes. These are behavioral tests, not evidence of physical key delivery.

`scripts/cargo fmt --all -- --check`, Clippy with `-D warnings`, SQLx offline metadata check, database integrity verification and `scripts/build-app` pass. The build retains upstream `block 0.1.6` future-compatibility and Swift CBLAS/preconcurrency warnings. The application bundle is locally ad-hoc signed. Evidence: `.local/tests-final.log`, `clippy.log`, `sqlx-check.log`, `db-verify-final.json`, `model-verification-final.json`, `build-final.log`.

Real YouTube metadata, download and local transcription succeeded for public Big Buck Bunny media; its explicit title override survived refresh/reprocessing. A separate missing-model test returned failure plus the retained imported recording, without downloading an ASR model implicitly. No personal recordings were sent to media services.

### Hardware and interface evidence

Test machine: Apple Silicon Mac mini Mac14,12, 16 GB RAM, macOS 26.3.1. M3 MacBook Air has not been exercised.

| Capability | Evidence | Still unverified |
|---|---|---|
| System audio | Actual 48 kHz capture of public speech, peak 0.783, first buffer 188 ms; local ASR persisted | Real meeting and simultaneous mic/system alignment |
| Live capture health | Seven live reports, peak 0.446; audio retained | Bluetooth/device changes during actual mic capture |
| Writer crash recovery | SIGKILL produced explicit failure; 833,536-byte CAF remained unchanged, recovered and reopened; gaps marked unknown | Broader power-loss/fault matrix |
| Microphone selection | Real device enumeration; unavailable UID rejected before asking permission | Actual microphone capture; permission remains not determined |
| Native shortcut tap | Tap creation and parent-death cleanup exercised; simulated gesture tests pass | Physical built-in/external key down/up; generated CUA keys did not reach tap |
| Text insertion | Native AX insertion succeeded in disposable TextEdit document; changed-focus insertion rejected | Other intended editors and actual clipboard event delivery |
| GPUI | Real title edit, playback, processing, model install/progress/select and disabled prompt draft exercised | Subjective acceptance, complete keyboard accessibility |
| UI under work | Approximately 220–222 ms maximum observed 100 ms heartbeat interval; render work around 1 ms or less in short probes | Sustained load/thermal test and comparison with MacParakeet |

The synthetic prompt draft was disabled and unauthorized in an isolated UI test library. No actual remote route was invoked. The native dictation listener is stopped. Private hardware/UI receipts are under `.local/hardware*`, `crash-probe/`, `orphan-probe/`, `ui-probe/` and `textedit-insertion-result.json`.

### Remaining work and risk

The [first voice-identification checkpoint](human-checkpoints.md) is now complete: three user-confirmed exact clips produced three profiles with one verified example each. Twelve separate candidate intervals were compared using an explicit conservative diagnostic policy; all remained Unknown. No predictions were enrolled. Those comparison intervals lack verified identity labels, so the run does not measure recognition accuracy. More independent verified examples and held-out meeting labels are needed for useful calibration.

The follow-up also repaired desktop example loading: selecting a profile loads its examples, refresh picks up CLI changes, enrollment refreshes the list, and late responses for a different selected person are ignored. Personal names and enrollment/diagnostic receipts remain under ignored `.local/enrollment-checkpoint/` and in the local database.

Follow-up verification: all 41 workspace tests and Clippy pass again. The desktop was exercised against all three newly enrolled profiles; each selection loaded one saved example. Example controls are stacked to fit the reader width and interval labels use seconds. A rebuild exposed Finder metadata on the generated bundle that prevented ad-hoc signing; the build now removes only FinderInfo/ResourceFork attributes from its own build product before signing, preserving security attributes. The rebuilt bundle passes strict signature verification. Follow-up logs and confirmation/diagnostic receipts are in `.local/enrollment-checkpoint/`.

Latest continuation verification: **46 workspace test cases pass** (Application 2, CLI 5, Database 13, Domain 16, Infrastructure 10), including a child-process lock-holder helper exercised by the process-death probe. Clippy and the bundle build pass. Actual SQLite tests cover active-owner protection, legacy preservation, conditional settlement, evidence preservation, retry lineage, refusal to retry live/completed jobs and refusal to silently change model revision. Pending CLI inspection does not initialize storage or create lease directories. New SQL uses checked external queries and regenerated offline metadata.

A real public-media ASR process was protected while active, killed with SIGKILL, recovered as interrupted, and successfully retried into a new linked run with **46 transcript segments**. The source SHA-256 and old run were preserved. Separately, GPUI recovered an intentionally abandoned public-media job and successfully retried it while navigation remained available. Speaker comparison review was exercised through native controls: accept → undo → reject → undo → replace → undo; all decisions persisted and no example was enrolled. The comparison fixture reused its public enrollment utterance and is UI evidence, not recognition accuracy. Private receipts: `.local/processing-recovery/report.json`, `.local/suggestion-ui/review-ui-report.json`, `.local/suggestion-ui/recovery-ui-report.json`, `.local/jobs-tests.log`, `.local/jobs-clippy.log`, `.local/jobs-sqlx.log` and `.local/jobs-bundle-build.log`. See [recovery behavior](processing-recovery.md). Personal enrollment still contains exactly three confirmed examples.

### Latest autonomous continuation

**50 workspace tests pass** (Application 2, CLI 6, Database 15, Domain 16, Infrastructure 11), plus the separately executed opt-in real-model cache test. Clippy with warnings denied, checked SQLx metadata, and the native bundle build pass. The cache integration test is intentionally ignored during ordinary tests because it requires explicitly selected model copies, public audio and a 75-second idle period; its explicit execution passed. Native clipboard self-tests are separate from the Rust count.

Delivered in this continuation:

* GPUI anonymous-cluster inspection, nonoverlapping representative playback, source-scoped split/merge and reversible edit history. A real public recording was transcribed and diarized, then native controls exercised split → merge → undo merge → undo split. Cluster counts were 2 → 3 → 2 → 3 → 2, and the original intervals were restored exactly. Raw ASR and diarization runs remained intact; no person was enrolled.
* Shared audio fingerprint/relink services, checked compare-and-set SQL, CLI and native controls. New valid imports receive SHA-256 identities; ten retained personal tracks were explicitly fingerprinted without writing the originals. A public audio copy passed native dry-run and saved relinking while preserving its track/run IDs and title. A byte-changed copy was refused. Missing originals without a previously saved content hash are refused rather than guessed. SQLite tests cover concurrent/stale updates and evidence preservation.
* Bounded reuse of one Whisper context and one WeSpeaker embedding context per process, with revision/verification/fingerprint invalidation and 60-second idle eviction (checked every ten seconds). A real-model test confirmed identical ASR text/timing, embedding cosine above 0.999999, rejection after removing the clone's verification receipt, and both models reloaded after 75 seconds. The 11-second public ASR probe measured 7.186 seconds cold (including 6.868 seconds loading/initial compilation) versus 0.263 warm. CPU embedding measured 1.763 versus 1.686 seconds. This is a small probe, not a universal speedup claim. Whole diarization pipelines still load per run; processes do not share loaded models.
* Clipboard fallback after direct AX insertion fails, restricted to the original frontmost app/element, readable value/selection and exact post-paste readback. Typed clipboard snapshots are restored unless a newer external copy replaced them. A private recovery journal supports interruption recovery through desktop startup, a native button and CLI. Private-pasteboard tests passed typed binary/string restoration, concurrent-copy preservation, interrupted restoration, real child-process SIGKILL recovery, 0700/0600 storage permissions and refusal of multiple items before mutation. **Actual fallback key delivery into a real editor is unverified.** Direct AX insertion was previously exercised in TextEdit. Ambiguous delivery retains text and asks for target inspection before retrying.

Private receipts: `.local/independent-tests.log`, `independent-clippy.log`, `independent-sqlx.log`, `independent-bundle-build.log`, `independent-ui/cluster-ui-report.json`, `independent-ui/relink-ui-report.json`, `clipboard-self-test.json`, `resources-test/report.json` and `personal-audio-identities.json`.

The expanded [public numeric report](../benchmarks/results/2026-10-27-expanded-public.json) records all 64 new processing runs, completed successfully: 16 diarization clips and 24 utterances for each ASR model. The diarization set contains all 16 qualifying 20–120-second multi-speaker VoxConverse test clips, including the original three held-out clips. Weighted DER is **4.83%** over all 16; the 13 additional clips have **4.69%** DER. Recording-macro JER is 11.70% and 12.09%, respectively. Speaker counts match in 11/16 recordings (8/13 additional); low DER does not mean speaker counting is always correct.

The ASR set contains one 8–30-second utterance from each of 24 additional LibriSpeech voices, excluding the six earlier fixture identities: 876 normalized reference words, 306.565 seconds of audio. Base made **33 errors (3.77% WER)**; small made **14 (1.60%)**. Summed engine wall times were 12.174 and 26.510 seconds, with 23/24 warm-context runs each. Process-lifetime peak RSS was 421,298,176 and 912,228,352 bytes, excluding some device allocations. Models/preset were fixed beforehand and personal model selections were unchanged. This clean read-speech result is not meeting accuracy, and the short diarization selection is not a full-corpus benchmark.

Independent follow-up remains: verified word/speaker alignment and meeting speaker-attributed word accuracy, recovery decisions for older unmanaged jobs, chunk-level continuation, a contention/priority strategy beyond fail-busy leases, broader editor compatibility and actual clipboard delivery, more adverse/long-form labeled benchmarks, and sustained resource/UI tests. Parakeet ASR is not integrated; the complete FluidAudio package needs newer Swift/SDK tooling. These are unfinished work, not waived requirements. There is no claim that every requested feature is complete or that personal accuracy exceeds MacParakeet.

The source/enrollment audit is repeated after this continuation. Original artifact size/mtime and JSON hashes, confirmed example IDs, pending jobs, model selections and SQLite integrity are recorded in `.local/independent-final-state.json`. Personal identity still has exactly three user-confirmed examples; no new permissions or external AI routes were enabled.

### Speaker-word continuation

The subsequent continuation adds read-only cpWER scoring through an application-owned capability/service and an opt-in Whisper token-timing mode shared by CLI and GPUI. The [speaker-word report](speaker-word-evaluation.md) documents the input contract, bounded scoring, explicit label provenance and substantial remaining quality errors. The Rust scorer matched MeetEval 0.4.3 on all 250 deterministic cases. A six-turn/three-voice synthetic public conversation and an overlap variant each contain 232 reference words. Same-ASR comparisons reduced cpWER from 50.00%/46.55% to 35.34%/40.95% (sequential base/small) and from 72.41%/65.52% to 30.17%/31.03% (overlap). This is diagnostic improvement, not an accuracy acceptance pass. Token times are unverified model estimates, no equal-duration word interpolation is used, and normal dictation/transcription remains on segment timing.

Latest Rust gate: **57 workspace tests pass** (Application 2, CLI 7, Database 15, Domain 20, Infrastructure 13), plus the separately passed real-model cache/word-mode probe. The latter confirms stable repeated estimated-word evidence and unchanged segment output after switching modes, verification invalidation, and idle expiry. Source-aware word validation recomputes common time from the actual saved track rather than trusting embedded common timestamps. Unknown origins, invalid ranges and uncertain speech remain unattributed. SQLx offline check, Clippy and the signed bundle build are rerun for this continuation. Receipts: `.local/word-timing-tests.log`, `word-timing-clippy.log`, `word-timing-sqlx.log`, `word-timing-bundle-build.log`, `word-timing/cache-probe.log`, `speaker-wer-oracle.json` and `speaker-word-estimated/pipeline-report.json`.

Native verification of this continuation invoked the explicit experimental action on public audio, saved a completed new run with the flag enabled, preserved all prior runs and displayed **231 original model words** with timed/Unknown anonymous evidence. Expanding controls exposed a reader-clipping issue; the reader now scrolls as one region and segment action buttons wrap, keeping corrections and Undo reachable. Both behaviors were checked in the rebuilt native app. The final source/enrollment audit again found 48 unchanged original fingerprints/JSON hashes, exactly the same three confirmed examples, unchanged model selections, no pending personal jobs and SQLite integrity `ok` at schema 3. Receipts: `.local/word-timing/ui-report.json` and `final-state.json`. The desktop was restored to the personal library.

### Native UI / UX pass

The [UI guide](ui-ux.md) records the Slack/MacParakeet design inspection, implemented interactions, keyboard behavior and remaining limits. The library now keeps its list beside the reader, with separate transcript, source and processing views. Native checks exercised title saving/reopening, debounced search, transcript find, multiline corrections, explicit Unknown through later text edits, Undo, keyboard save/navigation, file import, failed import feedback, playback/Stop and cancellation of model/person/example removal. The isolated library grew from 18 to 19 recordings through a public WAV import; reimport reported zero new and one existing item. Personal application data was not used as the mutation target.

**59 workspace tests pass** (Application 2, CLI 7, Database 15, Domain 22, Infrastructure 13). Clippy with warnings denied, SQLx offline metadata check and the signed native bundle build pass. The read-only source audit again reports 48 unchanged source artifacts, the same three confirmed example IDs, unchanged selected models, no pending personal jobs and SQLite integrity `ok` at schema 3. Tests and receipts are in `.local/ui-pass/`.

Real native processing produced a completed 11-second public-file transcription and a second 634.578-second public-media run with 46 segments in 21.360832 seconds. Navigation was exercised while the longer job ran. The measured render-construction peak was 2.31025 ms and the maximum nominal 100 ms heartbeat interval was 267.758958 ms for that probe. These do not prove frame-rate targets, sustained memory bounds or superiority to either reference product. Full VoiceOver, physical keyboard delivery, microphone capture approval, M3 behavior and subjective acceptance remain separate checks.

### Focused native performance pass

The [performance guide](ui-performance.md) and [numeric report](../benchmarks/results/2026-10-27-native-ui.json) document actual release-binary observations following Smaller Talk's benchmark patterns. Full GPUI frame work and input latency, macOS process CPU/RSS/footprint and background-job context replace the earlier construction/heartbeat-only probe. Instrumentation is explicitly opt-in, writes only local evidence, and does not initiate UI actions or download models.

The reader prepares corrected/searchable display data off the UI thread, reuses matching indices, and renders variable-height visible/overscan rows within the existing 40-row page. Final p95 frame work was 3.25 ms on the 593-segment personal transcript versus 13.59–14.69 ms baseline, and 3.07 ms on the 2,000-segment fixture versus 9.61–9.93 ms. Scrolling during real local ASR also improved (9.94 to 3.06 ms). Memory was approximately unchanged, with slightly higher sampled post-processing footprint after a different QA history. These are bounded observations on one busy Mac, not sustained FPS, leak-free operation or M3 acceptance.

**60 workspace tests pass** (Application 2, CLI 7, Database 15, Desktop 1, Domain 22, Infrastructure 13), plus six Python sampler/report tests. Default and performance-feature Clippy with warnings denied, formatting, SQLx offline metadata check and signed release builds pass. The existing opt-in real-model cache test was not rerun for this UI-only change; real public-media ASR was run through both instrumented desktop builds. A CPU-clock calibration caught and corrected an initial Mach-tick unit error in the baseline resource report; original evidence and an explicit correction receipt are preserved. One baseline and three candidate process reads failed during inference and remain missing evidence.

Native checks verified paging, multiline edits, offscreen editor focus, corrected search, Undo, both Find-dismissal paths, and narrow-width wrapping/scrolling. Selection spanning recycled rows and full VoiceOver traversal remain unverified. All source evidence is unchanged: 48 original fingerprints/JSON hashes, exactly the same three confirmed example IDs, unchanged models, no pending personal jobs, SQLite integrity `ok`, schema 3. Private receipts are in `.local/performance/`; no personal text, identities, audio, embeddings or raw process paths were added to the public numeric report. Physical input, microphone permissions and M3 limitations still apply.

### Idle activity and evidence indexing follow-up

**61 workspace tests pass** (Desktop now 2), with default/performance-feature Clippy, formatting and signed normal/instrumented release builds. Six Python report tests are repeated. The new reader test covers foreign segment IDs, repeated attribution claims, source order, unknown/untimed words and the existing 100-word display bound while retaining complete raw evidence.

The application activity loop no longer polls when settled idle. A real native public-audio run, quiet interval, second run and quiet interval verified restart/stop behavior through explicit observer counters. The independent sequential fixture remained exactly unchanged across all segments, corrections, attribution and 231 raw words. Expanded evidence was scrolled in the native reader. Ordinary release idle sampling observed a modest CPU/interrupt-wakeup reduction; package-idle wakeups rose with different host conditions, so no battery-energy claim is made. See the [follow-up measurements](ui-performance.md) and [numeric report](../benchmarks/results/2026-10-27-native-idle.json).

Source/enrollment verification remains read-only and unchanged: 48 source artifacts, exactly three confirmed example IDs, selected models, no pending personal work and SQLite integrity/schema 3. Private activity traces, isolated public reprocessing output, idle samples and tests are in `.local/refinement/`. No microphone, physical input, M3, identity calibration or remote AI permission was exercised by this follow-up.

### Durable meeting workflow and streamed normalization

**70 workspace tests pass** (Application 2, CLI 8, Database 20, Desktop 2, Domain 22, Infrastructure 16). Default and performance-feature Clippy with warnings denied, SQLx offline metadata check, formatting and signed release builds pass. The explicit decoder resource probe was additionally run in two separate processes; the unrelated opt-in model cache test remains skipped in ordinary tests.

The [meeting workflow](meeting-workflow.md) shares automatic after-Stop local ASR and idempotent finalization across CLI/GPUI. Audio and pending intent are saved before inference; per-source progress, model/revision and run correlation persist. Actual SQLite tests cover partial failure, missing source with available playback, interrupted checkpoint reuse, active-controller refusal, forged completion, title/provenance preservation and model revision changing between preflight and run start. CLI tests cover read-only/provisioning behavior, explicit settings and refusal to capture when automatic-ASR preflight fails.

Six new isolated recordings have completed finalization plans, with eight completed source runs and two deliberately failed source runs retained. Two-source public fixtures exercised unavailable test copies/restoration, 0/1250 ms alignment and title overrides; resume reused completed work and repeated completion added no runs. Actual system-only CLI capture retained 7.8 seconds and transcribed locally. SIGKILL capture recovery retained 3.1 seconds, restored its pending plan identity and finalized while retaining partial/gap warnings. Native Jobs/Open/Resume, preference persistence, system start/Stop and automatic transcript opening passed. A regression check retained old library/Find filters during capture, then confirmed both cleared when the new transcript opened. These are workflow checks, not personal speech/identity accuracy.

Disk-backed Nemotron normalization removes the whole-interval Rust PCM vector from that path. The [numeric report](../benchmarks/results/2026-10-27-meeting-workflow.json) records a 30-minute synthetic decoder probe and identical real-model intervals on one public clip. Decoder peak RSS fell from about 231.5 to 15.6 MiB in the separate probe processes; Core ML/helper and file-cache memory are not covered by that comparison. Native runtime/model licensing and defaults did not change.

Final private receipts are under `.local/meeting-workflow/`. Original MacParakeet audio/database are read-only throughout. Microphone permission/capture, Bluetooth changes, simultaneous intended sources, physical keyboards and M3 still need their separate checks. Live preview, retention policy, queued priorities, fine-grained cancellation/progress and within-source chunk continuation remain unfinished; this batch does not complete every remaining bucket.

The final audit confirms 48 unchanged original source fingerprints/JSON hashes, the same exact three confirmed example IDs, unchanged selected models, no pending personal jobs, SQLite integrity `ok`/schema 3 and 18 personal-library records. Automatic transcription was enabled only in the isolated test library; the personal preference remains off. All isolated finalization plans are completed. The ordinary signed release is restored to the personal library; source/audio/model data remain outside Git.

### Durable inference progress and cancellation

**78 workspace tests pass** (Application 2, CLI 9, Database 25, Desktop 2, Domain 22, Infrastructure 18), with two unrelated opt-in probes skipped. Default/performance-feature Clippy with warnings denied, formatting, regenerated SQLx metadata/offline check, and the signed normal release build pass. SQLx check also reports potentially unused cached queries; all committed query texts match current external SQL. The [control guide](processing-control.md) documents application-owned contracts, atomic cancellation/publication, worker ownership, adapter stop boundaries and chunk evidence.

The final release was exercised on a 634.578-second public media file. Whisper cancellation settled interrupted and exited 6 in **0.1372 seconds**, retaining two private completed chunk files and publishing no cancelled segments. Linked retry completed with 46 text segments/times matching a prior completed run. Tiny probability serialization differences prevent claiming exact raw JSON equality. Nemotron cancellation settled/exited in **0.1375 seconds**; its exact owned child had exited and 40,611,636 bytes of prepared PCM remained. A complete public 11-second speech run retained exactly the previous three diarization intervals. These are individual workflow observations, not latency guarantees or new accuracy benchmarks.

Native verification observed source progress/saved chunks, cancellation, live CLI-owned progress and terminal removal in Background jobs, retry cancellation from history, and persistent cancellation feedback after detail refresh. A later native Whisper transcription completed while reusing the same process's cached model after an abort. Private SDK/progress/run receipts are in `.local/job-control/`; the [public numeric report](../benchmarks/results/2026-10-27-processing-control.json) contains no private transcript, identity, path or embedding content.

The read-only audit again confirms 48 original artifact fingerprints/JSON hashes unchanged, exactly the same three confirmed enrollment examples, unchanged selected models, no pending personal work and SQLite integrity `ok`/schema 3. The main library remains at 18 records, and its automatic-after-Stop preference remains off. The ordinary signed release is restored to that library. Actual microphone approval/capture, physical keyboard lifecycle, Bluetooth changes, simultaneous intended sources, M3 and personal identity calibration remain unverified by this pass.

**Next:** validated continuation from retained Whisper chunks, including source/model/settings verification, contiguous-prefix checks and resumed-versus-uninterrupted comparisons. Existing local models and public fixtures are sufficient; no user input is needed. Current retry still restarts its requested interval. Priorities, live preview, retention policy and the separate human hardware checks remain outstanding.

### Storage accounting and retention previews

**91 workspace tests pass** (Application 2, CLI 10, Database 28, Desktop 2, Domain 24, Infrastructure 25). Default/performance-feature Clippy with warnings denied, formatting, regenerated SQLx metadata/offline check and the signed normal release build pass. The explicit model-cache/decoder probes were skipped for this metadata feature; real local Whisper was run and cancelled to exercise active/recoverable file protection. Existing upstream/compiler/cache warnings remain separate from those gates.

[Storage](storage.md) now shares metadata accounting, default-Keep-all preview policy, revision-checked saves and conservative potential-candidate selection across CLI/GPUI. The four new checked SQL files and explicit schema-4 migration stay in the persistence adapter; Domain owns age/ownership/evidence decisions and infrastructure owns bounded filesystem observation.

A real isolated library copy preserved 18 recording IDs/titles, three confirmed examples, model choices, source content and existing files across hypothetical previews and policy saves. A live public-media Whisper chunk was protected while active and after cancellation. Five final CLI metadata probes took 11.6–12.5 ms each, observing approximately 1.38 GB of root files and 425 MB of external referenced audio; this is a bounded observation, not a large-library guarantee. The two known missing source references kept inventory completeness false. Allocated blocks are reported separately; shared APFS extents/actual space freed are not measured. (The retention preview this measured was removed on October 28.)

Native checks exercised hypothetical preview, a conflicting CLI policy change/stale save refusal, refresh, successful save, Keep-all restoration, invalid-age recovery, warning expansion/collapse and protected-work display. Positive old-job candidates are tested with deterministic actual SQLite/filesystem fixtures; personal job ages were not fabricated. The Copy JSON button and full VoiceOver traversal were not exercised. Storage prepares aggregates outside rendering, bounds displayed rows and has a separate busy state so inference does not disable ordinary inspection.

Explicit schema-3-to-4 migration was tested on a snapshot and then applied to the personal Parakeet Local library after a SQLite backup. All 13 pre-existing table content hashes remain unchanged. The final read-only audit retains 48 unchanged original artifact fingerprints/JSON hashes, the exact same three confirmed example IDs, selected models, no pending personal work and SQLite integrity `ok`/schema 4. The personal library has 18 recordings, automatic-after-Stop remains off, and Keep all/revision 0 remains saved. The ordinary release is restored there on the new Storage screen. Private receipts and backup are in `.local/storage-pass/`.


### Integrated meeting and speaker workflow

**120 workspace tests pass**, including 12 real-adapter/deterministic-inference analysis tests and a new CLI contract test. Default/performance Clippy, formatting, SQLx metadata/offline checks and the signed release pass. Existing upstream/native warnings remain; the opt-in cache/decoder/cleanup fixture preparations were not repeated for this orchestration pass.

[The integrated workflow](meeting-analysis.md) now connects ASR, source-aware diarization, playable candidates, optional explicit-policy suggestions and resumable after-Stop processing across CLI/GPUI. Real public two-source audio completed initial analysis in 3.942 seconds, with 20 segments/one candidate; explicit diagnostic comparison returned Unknown and did not enroll data. A retained 12m45s personal meeting completed four source runs in 42.917 seconds with 226 segments/two candidates, preserving its 2884 ms offset. Those outputs are diagnostics, not personal speaker-accuracy measurements. Native analysis/review/playback and default-off option dependencies were exercised.

Backed-up personal schema 5→6 migration passed an isolated rehearsal, preserved all 15 existing table hashes, 18 recordings, exact three confirmed samples, selected models and 48 original artifact fingerprints/JSON hashes. Keep all/revision 0 and both automatic after-Stop options remain off. No personal cleanup or remote AI occurred. Private receipts are in `.local/integrated-pass/`; portable numeric results omit content/identities.

Next: job scheduling and explicit dictation priority; current knowledge and fixtures suffice. Physical capture/keyboard/editor, Bluetooth/M3 and additional personal-quality validation remain separate user/hardware checkpoints.

### Design-alignment pass (branch `feat/design-alignment`)

Earlier entries above are historical. Where they describe behavior this pass changed (a correction not creating examples, record-only meetings by default, Whisper chunk resume, fixed 60 s Whisper windows, drop-on-drift capture), the code and the current docs win.

Evidence is stated by category; "unit" means no real model or hardware.

| Change | Evidence |
|---|---|
| Stale library schema refused at open (BUG-2) | SQLite-adapter test |
| Full-source diarization is current; bounded runs never replace it (BUG-1) | unit + SQLite-adapter |
| Centroid scoring, scoring method recorded (SPK-4) | unit |
| Meetings transcribe after Stop by default; record-only fallback without a verified model (MTG-1) | SQLite-adapter + CLI contract |
| Imported labels shown as unverified claims; imported source tracks (BUG-6, SEP-3) | unit; not seen in the GUI |
| Silent source warning (CAP-1) | unit on artifact metadata; no hardware |
| Lease files removed on release (BUG-3) | unit |
| Pooled cluster speech, no candidate truncation (SPK-5) | unit |
| Name a whole cluster; confirmed actions admit reversible examples (SPK-1, SPK-2) | SQLite-adapter; GUI compiled, not exercised |
| Imported-label clip proposals (SPK-3) | SQLite-adapter, synthetic labels |
| Calibration, adoptable policy, opt-in suggestions inline (SPK-6) | unit on synthetic embeddings; no personal calibration run |
| Re-embed, notes, history (SPK-7, SPK-10) | SQLite-adapter; no GUI for notes/history |
| Speech-aligned Whisper chunks, no chunk resume (SIMP-2, ASR-2, ASR-3) | unit VAD; real-model-tested on public silence, noise, tone (no text published) and 38 s of public meeting speech (118 words, RTF 0.08 in a debug build); WER against the old 60 s windows on 7 long public clips is in [the chunking comparison](../benchmarks/results/2026-10-28-long-clip-chunking.json): parity (3.7% vs 3.8%), no accuracy gain; a sparse-chunk re-decode recovers a sentence Whisper skipped in one 25 s chunk |
| Me/Them/Unknown (SEP-6), fuzzy duplicates, bleed detection (SEP-1 detection) | unit; synthetic two-track bleed incl. double-talk and offsets; decoded-WAV integration test; not run on a real meeting |
| Drift never drops audio (BUG-5) | compiled-helper self-test at ±200 ppm over one simulated hour |
| Mic reconnect (CAP-2) | compiled; logic not hardware-tested |
| Permission requests (CAP-4), modifier-only and letter shortcuts (DIC-3) | unit for validation; hardware not tested |

Limits of this pass: SEP-1's cancelled-microphone transcription (ASR on echo-cancelled audio) is not built; only bleed detection withholds "Me". The energy VAD removes silence and stationary noise, not speech-like noise. Clock-drift correction is recorded, not applied: a device clock 200 ppm fast leaves about 0.7 s of mic-to-system skew per hour. No UI performance harness was re-run after the UI changes.

### Accuracy pass (October 28)

All numbers are on public audio only; none were measured on the owner's recordings. Scripts and results are in `benchmarks/`.

| Question | Result |
|---|---|
| New speech-aligned chunks versus the old 60 s windows | Parity on long read speech (3.7% vs 3.8% error); better on sparse meeting tracks (AMI, same small model: 24.3% to 20.8%). [Result](../benchmarks/results/2026-10-28-long-clip-chunking.json) |
| Skipped sentences | One 25 s chunk silently dropped 23 words; chunks with implausibly few words are now re-decoded in halves, which recovered it |
| Larger Whisper models | large-v3 best on read speech (2.0%), ties medium.en on AMI (19.5%); [result](../benchmarks/results/2026-10-28-whisper-models.json) |
| Parakeet TDT v2 in the app | Best everywhere: 1.66% read speech, 16.42% AMI, versus 2.03% and 19.52% for large-v3, and about 100 times faster than real time; made the default. [Result](../benchmarks/results/2026-10-28-parakeet-tdt.json) |
| Words invented on muted or idle microphones | Seven synthetic non-speech tracks: Parakeet TDT and Whisper small.en print nothing; large-v3 still printed 11 words on breathing-like noise. Sound annotations and caption sign-offs are suppressed. [Result](../benchmarks/results/2026-10-28-muted-mic.json) |
| Raw capture retention | 7 days then a verified lossless copy (now WavPack with exact sample comparison; the FLAC version described here was replaced the same day) |

Test count: 149 workspace tests pass; clippy with warnings denied and fmt are clean. Build note: after the Command Line Tools 26.6 update, C++ headers are missing from the compiler's default search path; set `CPLUS_INCLUDE_PATH="$(xcrun --show-sdk-path)/usr/include/c++/v1"` when building (or reinstall the Command Line Tools).

