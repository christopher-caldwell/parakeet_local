# Integrated meeting and speaker analysis

CLI and GPUI now use `MeetingAnalysisService` for the same durable ASR → diarization → clean voice-candidate workflow. This prepares review material; it does not establish a person's identity. Source categories remain independent of people.

## Usage

```sh
# Uses deliberately installed/verified selections; downloads nothing.
parakeet --data-root /existing/root meetings analyze RECORDING_ID
# Inspect without running or initializing storage.
parakeet --data-root /existing/root meetings analysis RECORDING_ID
# Optional comparison, only after deliberately choosing all three thresholds:
parakeet --data-root /existing/root meetings analyze RECORDING_ID --new-run \
  --minimum-score SCORE --minimum-margin MARGIN --minimum-examples COUNT
# Automatic anonymous analysis after a future recording:
parakeet --data-root /existing/root capture record --system --analyze-speakers
parakeet --data-root /existing/root capture settings --analyze-after-stop true
```

The score/margin/count above are placeholders, not an approved personal policy. No numerical default is supplied. Default analysis makes no embedding requests or comparisons. An explicit policy yields saved suggestions or Unknown, never verified examples. Existing human-confirmed examples are unchanged.

Each recording's **Processing → Meeting & speakers** panel provides analyze/resume, new anonymous analysis, explicit-policy analysis and review of prepared voices. "Name the voices" opens the recording's **Name voices** tab, where each prepared voice is played, named by typing a person, and saved; naming still requires you to type or pick the person. Match checks (under People) support accepting/replacing/rejecting suggestions and correction history.

**Record a meeting** has a separate after-Stop speaker option, on by default since October 28 (accuracy first; without its echo check microphone speech is never labeled "Me"). A saved preference is kept. Enabling it also enables final transcription; disabling final transcription disables speaker analysis. CLI `--analyze-speakers` explicitly requests the whole pipeline for that capture. Both ASR and diarization readiness are checked before capture: when the preference finds a model missing, the meeting still records and the app says why speakers were not detected; an explicit request fails before capture. An unavailable model is never downloaded silently.

## Evidence and persistence

A versioned recipe pins catalog IDs/revisions. Sources retain exact track snapshots, content SHA-256 and actual offsets. Current full ASR is reused only when model/revision, source hash and track snapshot match. Otherwise all needed source runs share a fixed publication group; partial completion cannot replace a full transcript. New analysis archives the earlier recipe/receipt and retains original runs, title overrides and corrections. Starting a new recipe does not force duplicate ASR when compatible current full results exist.

Diarization runs are correlated to the analysis. Clean candidates use those exact runs, skip overlap and unknown meeting timing, and retain anonymous source-scoped identity. No candidate-count target is forced. The current candidate selector remains a conservative heuristic, with at most 12 longest eligible intervals; this is not an acoustic quality guarantee.

The analysis receipt lives in recording metadata, with conditional feature-local SQL updating only that field. This avoids another table for a single recording-owned checkpoint. Separate current source results, prior recipes, suggestions and user corrections remain inspectable. Stable suggestion IDs support a crash between suggestion persistence and checkpoint publication. Suggestions save the runtime catalog identity separately from the embedding vector-space identity; CPU/Core ML compatibility remains governed by the existing vector revision and capture-domain checks.

Per-recording file leases serialize controllers across CLI/desktop. An owned Tokio driver retains ownership if its caller disappears. Source and stage checkpoints permit resume without repeating completed work. Typed cancellation records Interrupted. Source replacement, changed model revisions, a conflicting recipe/policy or missing completion evidence causes a visible refusal. A still-running correlated source row requires inspection and explicit `jobs recover` after owner death; resume does not start duplicate work over it. Active owners remain protected by that existing recovery protocol.

After-Stop intent is saved in the existing capture/finalization sidecar before capture. Final ASR completion can remain durable while analysis needs attention. `meetings finalize` and native resume continue requested analysis; pending-meeting inspection includes this state. Storage cleanup protects unfinished analysis, including the gap after ASR completion before the first analysis checkpoint. No automatic cleanup or new retention permission is introduced.

## Verification and limits

Twelve new tests exercise actual SQLite, external SQL, filesystem identity and ownership with deterministic inference. They cover anonymous defaults, compatible synthetic identities, Unknown, explicit policy conflict, partial-source publication, manual job retry/correlation, resume, orphan recovery, caller abort, typed cancellation, source/model changes, archived recipes/corrections and automatic after-Stop failures. An additional CLI contract test verifies read-only behavior, complete thresholds and opt-in preference dependencies. These tests do not prove acoustic quality or actual capture.

Real local Whisper, Nemotron and WeSpeaker ran on a public synthetic two-source fixture (11.0 and 53.856 seconds, 0/1250 ms offsets). Initial analysis completed four runs, produced 20 transcript segments and one clean candidate in 3.942 seconds. Idempotent verification took 14.8 ms. A new diarization/comparison recipe reused ASR, completed two more runs and one Unknown comparison in 1.439 seconds. CLI maximum RSS observations were approximately 518/427 MB; native helper/process-tree memory is not fully accounted for. These are individual observations, not throughput or memory guarantees.

A retained 764.941-second personal meeting also completed four source runs in an isolated destination in 42.917 seconds, preserving 0/2884 ms offsets and producing 226 segments/two candidates. No identity comparison was requested. This is real-recording diagnostic evidence, with no new ground-truth accuracy claim. Existing three confirmed examples remained exactly unchanged.

Native checks exercised analysis launch/completion, blank-policy refusal, prepared candidate selection/playback and the after-Stop preference dependency/restoration. No real capture was started. The final schema 5→6 migration used a SQLite backup and isolated rehearsal: all 15 existing table hashes, 18 personal recordings, exact three confirmed examples and 48 original artifact fingerprints/JSON hashes remained unchanged. Keep all/revision 0, selected models and both default-off after-Stop preferences were preserved.

No remote AI was used. Microphone approval, physical keyboards/editor insertion, Bluetooth, M3 and additional personal identity validation remain outstanding. Live transcript preview, scheduling/dictation priority and calibrated personal recognition are separate remaining work. The next independent pass can implement scheduling/priority using the current ownership and cancellation boundaries; it needs no new user input.
