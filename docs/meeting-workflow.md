# Recording and durable final transcription

The native recording screen can explicitly enable **After Stop: transcribe locally (on)**. Existing settings default to recording audio only. Automatic mode checks the selected local ASR model before capture and freezes its identity/revision. It never downloads weights or follows a later model-selection change. A missing model fails preflight; record-only mode remains available.

The shared `MeetingService` drives both desktop recording and CLI `capture record --transcribe`. Capture imports its separate source audio and a pending finalization plan atomically before inference. Native session sidecars also retain the requested plan so an interrupted capture can recover it. A saved recording remains usable if final transcription fails. The CLI returns a nonzero exit and a structured `retained_result`; the desktop keeps the saved recording and displays its error/history.

```
parakeet --data-root <existing-root> capture settings
parakeet --data-root <existing-root> capture settings --transcribe-after-stop true
parakeet --data-root <existing-root> capture record --system --seconds 30 --transcribe
parakeet --data-root <existing-root> meetings pending-transcriptions
parakeet --data-root <existing-root> meetings finalize <recording-id>
```

The saved preference controls desktop recording. CLI capture deliberately requires `--transcribe`; omitting it records only. `meetings finalize` also prepares a plan for an existing recording, with optional `--model` on first preparation. Repeating it resumes the saved plan or returns the already completed result. For deliberate reprocessing through another model, use `transcribe-all`; it creates separate processing evidence.

Each plan has a stable UUID, recording/model/revision, frozen authoritative source IDs, attempts, completed source-run IDs, failures and status. Its file lease is scoped to the canonical database and prevents concurrent controllers. An operation-specific SQL compare-and-set changes only this metadata, preserving title overrides, original provenance, runs and corrections. No schema migration is required. Finalization never substitutes derived playback for a missing raw source.

Each source run records its finalization identity. Resumption checks durable completed run evidence, not just the plan's summary. A process failure between saving a source run and updating the plan therefore does not duplicate that source. Failures retain completed sources and attempt the remaining sources; explicit cancellation stops the remaining sources; a later resume retries only unfinished work. Model revision is rechecked at actual source-run start. A changed revision is refused. Older model output and unmappable corrections remain in history.

After whole-process interruption, inspect `jobs pending` and use `jobs recover` to settle abandoned source ownership before resuming the meeting. Active owners are preserved. For interrupted capture, use `capture recoverable` and `capture recover <owned-directory>` first. Recovery keeps partial capture/gap warnings even if subsequent transcription completes. Pending plans are visible in Background jobs; no startup loop silently executes them. The recording's Processing tab shows source progress/errors and Resume, while Audio & sources can start the same shared final pass.

The controller checkpoints **between sources**, not within an ASR chunk. [Inference progress/cancellation](processing-control.md) is available; validated within-source chunk continuation, queued priority scheduling, live transcription, retention controls and concurrent desktop capture while its final pass runs remain separate work. Microphone approval, simultaneous intended microphone/system alignment and device changes still require actual hardware checks. System-only capture has been exercised, including automatic final transcription and interruption recovery; this does not establish microphone reliability.

Verification uses an isolated application backup and public speech. A two-source test preserved 0/1250 ms offsets, an edited title and its completed source run after the other test copy disappeared and was restored. Three total runs remained: one failed and two completed. Repeating finalization added no runs. Actual system capture retained 7.8 seconds of audio and produced a completed one-segment ASR run. A SIGKILL capture recovery retained 3.1 seconds, recovered its pending plan identity, finalized successfully and kept `recovered_partial` state. Native Jobs/Open/Resume and the explicit preference/start/Stop workflow were also exercised. These are workflow/recovery checks, not transcription or identity accuracy measurements. Private receipts are under `.local/meeting-workflow/`.
