# Project review — October 27, 2026

Audited product revision: `64a6278` (clean starting tree). Authority: the original October 27 product brief, repository `AGENTS.md`, installed Playbook **0.3.0**, and its clean human-source checkout at `14c3ee5cfdb84b7431258c6822205f91d61da8be`. This is an implementing-agent code/requirements review with new reviewer probes, not an independent external audit. No product code, personal state, model choice or consent setting was changed during the review.

## Assessment

The **architecture and privacy direction remain sound**. Keep the six existing packages and the shared Rust core; a rewrite or new framework is unnecessary. GPUI and CLI are real entry points, native/model SDKs stay outside the core, explicit SQLite lifecycle and pinned local models are implemented, and original evidence remains separate from corrections and predictions.

The project is **not fully Playbook-compliant and not yet an accepted daily replacement for MacParakeet**. Fresh review found six functional defects plus service-contract drift. Those repairs should precede the previously proposed background-queue feature. More fundamentally, low public diarization error does not establish correct end-to-end words/person attribution on personal meetings. Automatic post-meeting diarization/identity suggestions and useful, calibrated personal recognition are still unfinished workflows.

Recent reliability, UI/performance, cancellation, continuation and storage work supports the original goal. Further storage expansion would now be lower value than closing the core recording/transcript/speaker loop. Reversible staging does not reclaim space and must not be presented as completed disk reclamation.

## Findings, in priority order

### R1 — P1: shortcut-stream failure leaves active dictation uncancelled

**Location:** `crates/application/src/dictation.rs:289`; native error production in `crates/infrastructure/src/native.rs:468`.

The listener cleans up its active task on explicit stop and successful EOF. An error from `events.next_event()` instead escapes through `event?` before that cleanup. Dropping the task's JoinHandle detaches the spawned dictation; its cancellation flag stays false. Native nonzero helper exit, pipe errors and malformed/failed events can reach this path.

**Impact:** capture can continue after the listener has reported failure and lost key-release handling. When capture eventually ends, the task can continue into transcription and insertion. The unintended-insertion consequence follows from the production control flow; the probe deliberately produced no audio and performed no insertion or AI call.

**Evidence:** a temporary reviewer adapter sent Down, waited for capture to start, then returned a stream error. The actual application listener returned an error with the captured stop flag still false. A desired-behavior assertion failed. The probe then explicitly stopped its synthetic capture.

**Repair:** funnel every listener exit through cancellation and owned-task settlement; also cover dropping/aborting the listener itself. Test error/EOF/stop/caller loss while listening, transcribing and awaiting transformation, with zero unintended insertion.

### R2 — P1: the legacy source-database reader rejects MacParakeet databases

**Location:** `crates/database/src/source/mod.rs:8`, calling destination `open_read_only` in `crates/database/src/lib.rs:107`.

The catalog reader uses the Parakeet Local ownership-checking opener. A valid MacParakeet database is foreign by design and lacks this application's ownership marker/migration checksum, so it is rejected before source-schema discovery. The error even recommends the same `meetings import-source-db` command that is currently failing. Artifact-folder imports remain available; this does not mean they recover all database-only media metadata.

**Evidence:** the actual release CLI was run with `meetings import-source-db <synthetic-source> --dry-run`. The fixture had the reader's supported identity/title/date columns and BLOB UUID. It exited 1 with `kind: source` and the ownership rejection. The source hash was unchanged.

**Repair:** use a separate strictly read-only, WAL-aware source snapshot/transaction opener. Retain the destination ownership guard. Add supported/legacy source-schema and live-WAL importer tests that exercise the real catalog and CLI rather than only application-database ownership refusal.

### R3 — P1: an excerpt transcription replaces the full transcript in ordinary reads

**Location:** `crates/database/src/library/segments.sql:3`.

Current selection chooses the last completed transcription per track without distinguishing a full-source pass from a bounded interval. CLI transcription exposes start/duration, so transcribing an excerpt after a full run makes the rest disappear from the default reader/export selection. Original rows remain retained; this is a selection/publication defect, not deletion.

**Evidence:** actual SQLite adapter probe published a full run with two segments, then one 5–6-second excerpt. The default detail contained only `excerpt-only`; an older full segment remained individually retrievable. The preservation assertion failed.

**Repair:** define explicit current-result selection. Full passes, interval experiments and incomplete multi-source result sets need distinct publication semantics. Preserve coherent prior results until a replacement is deliberately selected; expose history and unmapped corrections. Do not concatenate overlapping full/excerpt results as a quick fix.

### R4 — P1: full-recording transcription can succeed with a missing source

**Location:** `crates/application/src/processing.rs:454`.

`ProcessingService::recording` filters unavailable tracks out before collecting failures. If system audio is available and an authoritative microphone track is marked unavailable, it returns success for the system run alone. This differs from the durable meeting-finalization path, which checks its expected source plan.

**Evidence:** real application orchestration and SQLite adapters, with a deterministic successful test engine, returned success for a recording with one available and one unavailable authoritative source. The expected-incomplete assertion failed. This is simulated inference evidence, not a real-model test.

**Repair:** share expected-source completeness semantics across reprocessing and meeting finalization. Permit useful partial work, retain its runs, and return a named incomplete result/error listing missing sources. Playback must never silently substitute for raw sources.

### R5 — P2: library search ignores human-corrected text

**Location:** `crates/database/src/library/list.sql:2`.

Search scans all stored raw segment text. It neither searches replacement text from correction history nor limits matches to current selected results. A correction can therefore be visible in the reader/export but undiscoverable in the library, while obsolete raw text still matches.

**Evidence:** actual SQLite import/correction/read probe changed a segment to a unique token and searched that token. The library returned zero recordings; the assertion expected one.

**Repair:** specify whether ordinary search covers current corrected text or historical evidence, then implement the same semantics for CLI and desktop. Keep historical search explicit if supported. Add edit/undo/reprocess and stale-history cases.

### R6 — P2: benchmark imports bypass cleanup's publication gate

**Location:** `crates/cli/src/main.rs:1452`; write at `crates/application/src/evaluation.rs:89`.

Most entry-point import paths receive `GuardedAudioWrites`. Benchmark composition still passes the raw `SqliteStore`, so its final publication omits shared-gate acquisition and source revalidation. SQLite serializes a database mutation, but it does not revalidate an already prepared file locator after a cleanup move.

**Evidence:** the actual release CLI ran a one-second synthetic-silence benchmark while another process held the exclusive canonical storage gate. One recording was imported anyway. No weights were installed; model unavailability stayed a benchmark result. The full file-move race was not exercised. Gate bypass is proven; the stale-reference consequence follows from the unguarded publication path.

**Repair:** inject the same guarded write capability into every production consumer, including evaluation. Inventory all graph edges, not just ordinary library commands. Add a production-wiring regression and a prepared-source publication-versus-staging race test.

### R7 — P2: public service contracts have drifted from the Playbook

**Locations:** `crates/application/src/cleanup.rs:115`, `crates/application/src/processing_control.rs:179`, `crates/application/src/meeting.rs:237`, `crates/application/src/scoring.rs:45`, and `crates/application/src/lib.rs:571`.

Playbook **E1 is a MUST**: errors belong to each use case, not to a service merely because its methods share dependencies. Cleanup list/show/review/apply/restore all return the broad `CleanupError`; show/cancel share `ControlProcessingError`; listing unfinished meetings uses `FinalizeMeetingError`; distinct scoring operations share `ScoreEvaluationError`; media inspection exposes the capability error directly. Some shared internal mechanics are justified, but each public operation still needs its own contract.

**AS3 is a DEFAULT**, and several multi-value service operations also have positional inputs without a documented reason: cleanup review/apply and retention-policy save are clear examples. These are smaller conformance repairs, not a reason to split into handler crates or add dependency injection machinery. Workflow controls can remain separate execution context where appropriate.

The older review ledger's broad statement that operation-specific contracts are compliant is therefore no longer a valid current-project conclusion. This review supersedes that claim for the audited revision.

**Repair:** operation-specific public errors and named inputs, preserving sources and existing CLI error kinds/exit codes. Keep narrow shared internal helpers. Document any concrete default exception rather than treating convenience as policy.

## Original-intention traceability

“Implemented” below does not mean hardware-tested or human-accepted.

| Original goal | Current position | Remaining evidence/work |
|---|---|---|
| Rust-first, macOS native GPUI, one core/two entry points | Implemented; sound package/dependency direction | Maintain graph parity; repair R6/R7 rather than rewrite |
| Inspectable CLI and isolated storage | Implemented; JSON/errors, explicit create/migrate, real SQLite tests | R2/R4 expose semantic gaps not caught by the baseline suite |
| Existing MacParakeet import | Artifact path and existing personal imports have evidence | Source-database regression R2; strengthen compatibility/WAL tests |
| Reliable physical input selection | Explicit UID selection, effective-device checks, health/gap evidence and retained failures implemented | Actual microphone permission/capture, disconnect/Bluetooth/default changes and simultaneous intended sources still need controlled physical testing |
| Chronological/source-aware transcript | Offset-aware processing, stable ordering, overlap/unknown evidence implemented | Current-result selection/completeness R3/R4; final personal-meeting review |
| Avoid fabricated “Me” statements | Source is not identity; guesses never become verified examples; uncertain dictation is withheld | Better echo/bleed/silence/overlap evaluation on realistic meetings; no universal hallucination guarantee |
| High-quality local diarization | Speakrs and Nemotron real pipelines, pinned provenance, reproducible public diagnostics | End-to-end meeting integration, long meetings/adverse acoustics/slot saturation; personal truth and direct comparative baseline |
| Persistent known-person recognition | Local compatible embeddings, verified references, manual suggestions/corrections/undo implemented | Automatic cluster-to-suggestion workflow; calibrated policy and held-out personal evidence. Three exact confirmed clips are one example per person, not established recognition quality |
| Efficient enrollment and learning | Playable candidates and native/CLI confirmation exist; predictions never auto-enroll | Prepare targeted independent clean references, then a short identification checkpoint; preserve held-out labels |
| Multiple push-to-talk shortcuts | Configurable simultaneous bindings and simulated owning-key tests implemented | R1 repair; actual built-in/external delivery and target-editor acceptance |
| Intended-target insertion with recovery | AX/clipboard adapters, target checks and retained text implemented | Broader real-editor/fallback checks and cancellation boundaries; TextEdit acknowledgement is limited evidence |
| Application-specific opt-in AI | Bundle-ID routing, user prompts, declared routes and consent gates implemented | No remote route is authorized or needed for current work; user choice remains separate |
| YouTube titles/local transcription | Metadata-based import, durable overrides and local ASR path implemented | Retrieval/auth/rate-limit variability; source-DB import is separate R2 |
| Model choice/offline use | Explicit pinned installs/checksums/categories and local runtime selection implemented | ASR comparison currently covers Whisper variants; Parakeet/other promised candidates remain investigation work, not silently completed evaluation |
| Responsive native UI | Virtualized reader and measured bounded scrolling improvements; work off render thread | Long-session memory, full accessibility, broader editor/keyboard usability and M3 remain unverified |
| Safe processing durability | Owned jobs, progress, cancellation, recovery and validated chunk continuation implemented | R1/R3/R4; resource contention still rejects work rather than queues/prioritizes it |
| Safe local retention | Usage/preview and reviewed reversible staging/restore implemented | R6 wiring; staging frees zero bytes; permanent reclamation/expiry remains separate lower-priority work |
| Better personal experience than MacParakeet | Plausible foundation, not demonstrated acceptance | Close defects and integrated quality loop, then focused physical/personal acceptance |

The automatic after-Stop path currently performs ASR finalization. It does **not** automatically run diarization, extract clean cluster examples and generate known-person suggestions. Those capabilities currently require manual actions/explicit thresholds. A user should not be expected to assemble that pipeline for every ordinary meeting.

## Playbook evidence ledger

Classification concerns this audited revision and the cited path; it is not blanket certification of all possible behavior.

| Rule / source | Classification | Actual evidence and limit |
|---|---|---|
| WS1/WS2/WS3/WS5 — workspace shape | COMPLIANT WITH RULE | Six qualified packages; Domain/Application separated; production arrows inward; Database→Infrastructure only a dev-test dependency; binary-only entry points |
| D1 — foundational Domain dependencies | COMPLIANT WITH RULE | Serde/JSON/UUID/thiserror uses are neutral values, computation and errors; no runtime/SDK/database access in Domain |
| AS1 — feature services/private dependencies | COMPLIANT WITH RULE | Explicit constructors and private service fields; no service locator/generic dependency bag |
| AS3 — multi-value inputs | PLAYBOOK VIOLATION | Undocumented DEFAULT departures in cleanup and policy contracts (R7); distinguish execution context from operation values during repair |
| AS4 — named accurate outputs | INSUFFICIENT EVIDENCE | Most use cases have accurate domain/named projections; generic JSON report outputs and broad completion semantics need contract review; R4 shows a result can be structurally valid but semantically incomplete |
| AS5 — service orchestration | DEFAULT DEVIATION WITH JUSTIFICATION | Meeting/dictation/media workflow services compose existing services to preserve one shared capture/transcription behavior and durable intent; avoid expanding routine cross-feature call chains |
| E1 — per-use-case errors | PLAYBOOK VIOLATION | R7's public signatures reuse broad errors or expose capability failures directly; baseline type checking does not establish compliance |
| E4/P1/P2/P3 — inward capabilities/provider boundaries | COMPLIANT WITH RULE | Application-owned capture/models/media/embedding/storage/insertion ports; SDK/OS/SQLx types outside core; opaque source-preserving errors used |
| W3/W5 — direct mutation outcomes/concurrency | INSUFFICIENT EVIDENCE | Actual CAS corrections/finalization and SQLite tests exist; R6 disproves the claimed coverage of all cooperating import writers. Do not infer whole-system protection from the guarded-path tests |
| T1/T2/T3/T4 — earned transactions | COMPLIANT WITH RULE | Cleanup supplies its typed unit to feature Store methods; commit consumes it; SQLx rollback-on-drop; one-connection, real transfer and caller-loss ownership tests. No claim of atomic SQLite+filesystem commit |
| Transaction resource acquisition | COMPLIANT WITH RULE | Inspected cleanup acquisition order; no independent DB calls after begin; reads and writes share supplied connection. Model/run contention fails explicitly; bounded hashes can still delay writes |
| SQL1/SQL4 — external checked SQL | DEFAULT DEVIATION WITH JUSTIFICATION | Feature-local checked files and committed `.sqlx`; runtime third-party projection is whitelisted, operational ownership/schema/PRAGMA introspection documented. Fresh offline check passes |
| M1/M3/M4 — database lifecycle | COMPLIANT WITH RULE | Ordinary paths never initialize/migrate; explicit provisioning/migration; additive five-migration SQLite history. This does not excuse source/destination confusion R2 |
| M2/server runtime roles | NOT APPLICABLE | SQLite-only personal application; no PostgreSQL/server roles |
| C1/C2/C5 — explicit entry composition | INSUFFICIENT EVIDENCE | Both graphs are explicit and no peer adapter assembly/runtime CLI subprocess core exists, but one production decorator edge is wrong (R6). Large desktop source warrants incremental organization, not a rewrite |
| C3/C4 — external configuration | COMPLIANT WITH RULE | Runtime env/tool/root selection at entry points; adapter env hits are test-only; Cargo version stamp is compile-time provenance |
| Identity/Principal/HTTP/authenticated-resource rules | NOT APPLICABLE | No accounts, remote API, authenticated actors or tenant policy. Local opt-in route consent and OS permissions remain required behavior; no speculative Identity/HTTP subsystem is warranted |
| TEST2 — real persistence specifics | COMPLIANT WITH RULE | Actual SQLite adapter tests run; reviewer SQL probes expose missing behavioral cases rather than claiming mocks prove DB correctness |
| TEST3/hardware/personal quality | NOT EXERCISED | No new microphone/physical-keyboard/M3/meeting-identity/remote-AI acceptance or new accuracy benchmark in this review |

Sources: `playbooks/rust/docs/workspace/crate-boundaries/workspace-shape.md`; `application/service-organization/feature-services.md`; `application/service-inputs/use-case-inputs.md`; `application/error-boundaries/capability-and-use-case-errors.md`; `application/persistence-boundaries/unit-of-work.md`; `database/sqlx/query-files-and-mapping.md`; and the installed narrow guide actions named in this review. Human-source documents were read locally; the Playbook CLI does not automatically inspect or certify this repository.

## Executed evidence and limits

- **Submitted baseline:** 100 workspace tests pass, 3 intentional ordinary-suite skips (real model cache probe, decoder resource probe, native-fixture preparer). Default/performance-feature Clippy with warnings denied and checked SQLx offline metadata pass. Existing upstream warnings remain outside workspace warning denial.
- **Reviewer-added desired-behavior probes:** four failures reproduce R1/R3/R4/R5 against production application services/SQLite adapters. Test-only capture/ASR substitutes are identified above. They exercised no microphone, real insertion or external AI. Temporary probe source was removed from the product tree and retained privately with the red log for repair/reproduction.
- **Actual release CLI:** R2 source-import rejection and R6 exclusive-gate bypass reproduced on independent synthetic files/databases. The existing signed release was not rebuilt because product code did not change.
- **Preservation:** all 48 original artifact fingerprints/JSON hashes unchanged; all 14 previously recorded personal-table hashes unchanged; 18 personal recordings, exact same three confirmed example IDs, zero personal cleanup reviews, integrity `ok`, schema 5. No model downloads, new real-model benchmark or remote route invocation.
- **Prior quality evidence inspected, not rerun:** expanded short public diarization 4.83% DER, exact count 11/16; small clean-read ASR/identification subsets and speaker-attributed word diagnostic reports. Good anonymous diarization scores do not establish personal named attribution. Constructed conversation cpWER still has substantial errors; no superiority claim over MacParakeet is supported.
- Private receipts and reproducible reviewer probe source: `.local/zoom-out-review/`. A saved probe that augments an adapter does not certify a missing production path. This review did not exercise every interleaving, dataset or physical integration.

## Reset execution order

1. **Correctness repair pass:** R1–R6 with permanent regressions, including source WAL/schema cases and coherent full/partial transcript selection. Repair R7's public contracts while touching those use cases, then complete the remaining contract inventory. **Can proceed now.**
2. **Integrated meeting quality pass:** explicit selectable post-meeting ASR → source-aware diarization → clean anonymous-cluster review → compatible identity suggestions, with Unknown and provenance throughout. Add realistic bleed/overlap/silence/turn-transition evaluation and personal diagnostic candidate preparation; do not invent a personal threshold. **Most implementation/evaluation preparation can proceed now.**
3. **Focused human checkpoint:** approve/test intended microphone and both keyboards/target editors; identify a few additional independent clean samples and separate held-out intervals. **Requires physical interaction and actual identity knowledge.** No need to label entire meetings.
4. **Resource scheduling:** durable queue, dictation priority and contention/cancellation behavior, followed by long-session/resource and M3 checks. **Queue implementation can proceed now; M3/physical acceptance needs that machine/user.** Advance it sooner only if contention blocks the integrated quality pass.
5. **Lower-priority refinements:** permanent disk reclamation/expiry, broader model candidates and polish after correctness/quality is protected. **Much can proceed independently; deletion and remotely backed AI remain deliberate separate actions.**

Do not treat the final two storage passes or a green baseline suite as completion of the project's defining value: trustworthy source capture, correct transcript publication, and conservative, useful speaker learning in ordinary meetings.
