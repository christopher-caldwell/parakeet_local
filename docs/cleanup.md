# Reviewed staging and restore

Storage now offers a reversible cleanup step for completed processing scratch. A reviewed list is persisted before any move. **Staging reclaims zero bytes:** files remain on the same volume in `.cleanup/<review-id>/files`. There is no permanent-delete endpoint or automatic cleanup. Keep all remains the default preview policy. Original audio, model weights, transcripts, corrections and voice examples are not cleanup payloads.

The desktop Storage screen and CLI invoke the same application service. First preview an age threshold, then choose **Review up to 40 eligible files**. Inspect the concrete list and choose **Move reviewed files to recovery…**; a separate confirmation names that review. Recent reviews can be reopened and their retained files restored. Refresh reads current saved reviews. These actions do not enable recurring cleanup or change the saved preview policy.

```sh
parakeet --data-root <existing-root> storage cleanup review --completed-scratch-days 30
parakeet --data-root <existing-root> storage cleanup list
parakeet --data-root <existing-root> storage cleanup show <review-id>
parakeet --data-root <existing-root> storage cleanup apply <review-id> --review-digest <digest-from-review> --confirm
parakeet --data-root <existing-root> storage cleanup restore <review-id>
```

`list`/`show` are read-only and do not create lease files, initialize storage, migrate it, or install models. Review persists a concrete list and can create ownership files. `--completed-scratch-days` is a one-time policy override, including when the saved policy is Keep all; it authorizes no move by itself. Without it, review uses the saved policy. A null `review` means no files qualified within the bounds. Reviews contain at most 128 files (`--limit`, desktop 40) and 512 MiB total; larger files and files beyond the batch bounds are skipped. This is deliberately bounded, not a large-file cleanup facility.

Applying requires both explicit confirmation and the immutable review digest. Missing confirmation exits 4; wrong digest, changed policy or newly protected files exit 5; invalid inputs exit 2. Transfer failures exit 1 with a `retained_result` containing the review, locations derivable from its index, and errors. No failed transfer is reported as completed. Repeated completed apply/restore requests return the prior receipt. `staged` is a durable operation receipt, not continuous monitoring against later manual filesystem changes. Restore checks the actual files again.

## Evidence and protection

Reviews bind the canonical data root, root directory identity, canonical database, policy revision, exact paths, run UUIDs, device/inode, byte lengths, nanosecond modification/change times and SHA-256 content hashes. The review digest covers only this immutable evidence, so a new candidate cannot enter an existing approval. Fresh checks reject policy changes and newly active, unfinished, corrected, enrolled or referenced work. A missing source protects its own recording's scratch, even when that scratch might aid recovery. Unknown/partial layouts, multiple links, linked source aliases and invalid completion times remain excluded.

Only canonical `models/work/<run-uuid>/audio.f32`, `request.json`, and bounded complete Whisper chunk names are recognized. Hashing streams a fixed 128 KiB buffer. Descriptor-relative parent traversal and final opens reject symlinks and special files. Moves use macOS `renameatx_np(RENAME_EXCL)`, never overwrite a destination, and sync files/affected directories. Rooted recovery files are themselves classified as protected recovery evidence.

A review becomes `applying` in a committed transaction **before** the first move. A second transaction re-checks database facts before transferring and settling `staged` or `needs_attention`. An owned application task retains its transaction and leases until its blocking worker exits and settlement finishes, even if its caller is dropped. Process death or a commit failure can leave `applying` or `restoring`; restore inspects both locations against reviewed identities. An already restored item is accepted only when the recovery source is absent and the original matches. Conflicts retain both existing destination and recovery bytes for inspection. No path is unlinked and no fallback cross-volume copy/delete is attempted.

Keep the application database and recovery directory together. Moving them to a different root/database path or replacing the root directory changes the review binding; automatic restore refuses that case and requires inspection. Deleting either manually can prevent automatic restore. Restoring a backup alone is not a substitute for preserving its payloads. An occupied original location must be deliberately resolved before retrying restore; the application never removes the conflicting file for you.

## Ownership and transaction resources

Cleanup obtains the canonical-database exclusive storage gate, the data-root model lease, and sorted per-run leases **before** beginning SQLite work. Busy ownership fails rather than waiting indefinitely. Normal imports/capture/media imports/relinks in both entry points use a shared-gate decorator that revalidates available source files at final publication. A stale prepared import cannot publish an available locator after the reviewed file moves. Missing imported evidence remains importable as missing evidence.

The application owns `CleanupTransactions`, consuming `CleanupUnit::commit`, feature-specific `CleanupStore<Unit>` and filesystem capability ports. SQLite units use `BEGIN IMMEDIATE`; dropped units roll back. After acquiring a unit, all policy/audio/run/review operations use that unit's supplied connection. Filesystem operations acquire no database connection. Production pools have at most four connections with a five-second SQLite busy timeout. Hashing and moving a bounded batch can delay other writes; the UI stays on background tasks and receives busy/conflict errors for retry. Tests run the full path with a one-connection pool to expose accidental nested acquisitions.

Coordinated access covers cooperating current application processes using the selected database and its selected data root. Do not share one mutable scratch root between independently managed databases. Older binaries, direct database modifications, arbitrary filesystem writers and malicious same-user replacement of whole directory trees are outside the advisory-lock guarantee. Defensive opens/hashes and retained payload errors reduce damage from unexpected changes; this is not a hostile filesystem sandbox. Restore and safe failure are tested; universal power-loss/filesystem guarantees are not claimed.

## Verification

Actual SQLite/filesystem integration tests cover exact stage/restore, title/result/source preservation, fresh policy/reference/missing-source/finalization/correction/enrollment protection, symlinks, hard links, changed bytes, busy model/run owners, cross-root refusal, no-overwrite restore, partial durable intent, caller cancellation after a real move while retaining the database unit and ownership, prepared-import refusal, canonical database aliases and single-connection rollback/resource behavior. CLI tests verify read-only behavior, confirmation/digest exit codes and retained failure JSON. The opt-in synthetic native fixture supports UI testing without changing personal job ages or processing private audio.

This pass does not change model quality, establish hardware acceptance, or free disk space. Permanent reclamation, recovery expiry and larger-file handling remain future explicit operations.
