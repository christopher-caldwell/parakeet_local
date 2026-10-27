# Processing ownership, interruption and retry

New speech runs and derived cluster-edit runs acquire an application-owned processing lease before publishing `running`, and retain it through terminal persistence. The filesystem adapter scopes locks to the canonical database path, so a CLI database override or symlink alias uses the same ownership scope. These per-run locks are separate from the model-access lease that limits inference/install/removal resource contention.

```sh
target/debug/parakeet --data-root .local/app jobs pending
target/debug/parakeet --data-root .local/app jobs recover
target/debug/parakeet --data-root .local/app jobs retry RUN_ID
```

`pending` is read-only and creates neither a database nor a lock directory. `recover` is an explicit mutation. For each ownership-aware running row it attempts the run's lock without waiting. A held lock means active; a free lock permits a compare-and-set to `interrupted`. Concurrent completion wins safely. Recovery preserves source audio, request/configuration, partial evidence, segments and prior corrections. It records why and when the job was recovered. It does not resume inference automatically or claim successful transcription.

Earlier runs without `file-lock-v1` ownership metadata are reported as unmanaged and left untouched. Wall-clock age and PID guesses do not establish that an older process stopped. The initial development library had no pending runs when this feature was verified. Recovery is not silently run at startup, and a read-only inspection never settles jobs.

Retry accepts only failed/interrupted speech requests. It uses the original explicit model and interval, verifies the current pinned revision matches, and creates a new UUID with `retry_of_run_id`. It does not rewrite the old result or transplant corrections onto different segments. A model revision change requires deliberate new processing. Derived human cluster edits have no speech request to retry and must be repeated through the edit operation. An interrupted decoding/inference operation restarts the requested interval; chunk-level continuation is not implemented.

GPUI exposes pending inspection and explicit recovery in **Processing jobs**. The recording reader offers retry for failed/interrupted speech requests among its five latest runs. Older history remains available through CLI recording inspection. Active jobs can continue in another process without being interrupted by the recovery action.

Evidence includes an actual process-death lock test with a database symlink alias, actual SQLite state/correction preservation, and a real local Whisper process killed while running public media. Recovery protected it while alive, interrupted it after SIGKILL, and a linked retry completed with 46 segments and an unchanged source audio SHA-256. A separate native UI test exercised recovery and retry while navigating. Private receipts: `.local/processing-recovery/report.json`, `.local/suggestion-ui/recovery-ui-report.json`, `.local/jobs-tests.log`.

This is local crash recovery, not a general scheduler. Model-busy operations still fail explicitly; inference is loaded per operation; a priority queue, chunk continuation and whole-machine fault testing remain outside the verified behavior. Do not remove an active library's lease directory as cleanup: unlinking lock files can defeat cross-process ownership.
