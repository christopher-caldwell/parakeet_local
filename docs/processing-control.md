# Inference progress and cancellation

CLI and GPUI use the same application-owned control service. `jobs show RUN_ID` is read-only; `jobs cancel RUN_ID` records an idempotent cancellation request. Ordinary inspection does not create/migrate a database or initialize model storage. Speech runs advertise `processing-control-v1` and their cancellation mode. Older unmanaged runs refuse cancellation rather than guessing which process to terminate.

```sh
parakeet --data-root <existing-root> jobs pending
parakeet --data-root <existing-root> jobs show <run-id>
parakeet --data-root <existing-root> jobs cancel <run-id>
parakeet --data-root <existing-root> jobs retry <interrupted-run-id>
```

A successful cancellation command means the request was saved. The run remains `running` until its worker exits. The original processing command then exits with code **6**, error kind `cancelled`, and the retained run ID. Terminal cancellation returns `requested: false`. Progress is durable JSON under `result.progress`: sequence, stage, audio completed/total milliseconds when known, saved chunk count, current Whisper chunk percentage when available, and observation time. These are engine observations, not an ETA or a claim of transcription accuracy. No streaming progress is mixed into JSON stdout.

The application polls the small control-state query every 100 ms while inference runs and coalesces changed progress into SQLite. Sequence checks reject stale observations. The SQL write initializes an initially null result object; an actual-file probe found that SQLite does not create a nested key beneath JSON null. Terminal publication and cancellation arbitrate in the same conditional SQL transaction: a winning cancellation cannot publish unwanted transcript segments. A late completed output is retained as unpublished evidence. A driver task owns the run lease independently of a GUI/CLI waiter. Storage/control failure requests a stop and drains the worker before releasing ownership. Process death still requires explicit recovery.

Whisper uses synchronous scoped C callbacks to observe SDK progress and abort cooperatively. Model initialization itself must reach its safe boundary. Each completed 60-second chunk is retained privately at `<data-root>/models/work/<run-id>/whisper-chunks/chunk-NNNN.json`, with model revision, runtime/processing settings, source locator/provenance, raw recognition and segment evidence. Files use 0600, the chunk directory 0700; flush, fsync and no-replacement publication precede the saved-chunk progress increment. Partial chunks are not presented as completed transcripts.

Nemotron prepares disk-backed PCM, runs an owned native child, and emits bounded progress on stderr. Cancellation terminates and waits for that exact child and drains both output pipes before settlement. Prepared PCM/options are retained on interruption. Speakrs supports cancellation at an engine boundary; its current blocking diarization call must finish. The UI states that limitation. No adapter kills arbitrary PIDs or releases the model lock while its own SDK call continues.

GPUI refreshes active job observations every 500 ms outside rendering. Source, Processing and Background jobs screens expose the same Cancel action. A request disables its button until settlement; cancellation feedback survives the background detail refresh. The monitor stops outside Background jobs when no own task or observed active run remains. Cancellation of a source in either whole-recording workflow stops the remaining sources; meeting finalization retains a resumable failed plan and its already completed sources.

## Verification and limits

Actual SQLite tests cover cancellation versus terminal publication, aborting the outer waiter while a gated worker still holds ownership, recovery refusal for a live owner, storage failure while a worker remains active, null-result progress initialization, monotonic observations, idempotency/older-protocol refusal, and cancellation stopping subsequent sources. A filesystem test checks durable chunk metadata, private modes, UUID traversal refusal and no-replacement behavior. These controlled-worker tests do not establish SDK behavior alone.

Real local Whisper and Nemotron cancellation were separately exercised on a 634.578-second public media file in an isolated application backup. Both retained source audio unchanged, settled interrupted and exited 6. Whisper retained completed chunks and published no cancelled segments; a linked retry completed. Its 46 text segments and times matched a prior completed run; tiny floating-point serialization differences in raw probabilities remain, so complete raw JSON equality is not claimed. The completed public speech diarization produced the same three intervals as the prior implementation. Native verification exercised live source progress, cancellation, processing history and background job observation. Numeric results are in `benchmarks/results/2026-10-27-processing-control.json`; private receipts are in `.local/job-control/`.

The initial control pass retained version-1 chunks without resuming them. Validated version-2 Whisper continuation is now implemented for CLI/native retry and meeting finalization, with fresh source/model/settings checks and repeated-interruption comparisons. Older/incompatible checkpoints and other engines still restart their interval. Queued priorities, live transcript preview, retention policy, microphone approval, physical keyboards, M3 and personal recognition acceptance remain separate work. No remote AI route was invoked.
