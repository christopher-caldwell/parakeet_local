# Storage accounting and retention previews

The native **Storage** screen and JSON CLI share `application::storage::StorageService`. Inspection measures local file metadata and referenced audio without opening model weights or decoding audio. It neither initializes/migrates a database nor downloads models. Schema 4 is installed only by an explicit `db migrate` command; the new singleton policy starts at **Keep all**, revision 0.

```sh
parakeet --data-root <existing-root> storage usage
parakeet --data-root <existing-root> storage policy
parakeet --data-root <existing-root> storage preview
parakeet --data-root <existing-root> storage preview --completed-scratch-days 30
parakeet --data-root <existing-root> storage preview --keep-all
parakeet --data-root <existing-root> storage set-policy --completed-scratch-days 30 --expected-revision 0
parakeet --data-root <existing-root> storage set-policy --keep-all --expected-revision 1
```

`usage`, `policy` and `preview` are read-only. An age/keep-all argument overrides only that preview. `set-policy` saves a preview threshold and requires the observed revision; another writer winning returns conflict/exit 5. Invalid ages return exit 2. The threshold accepts 1–3650 elapsed 24-hour days after a run's completion, independent of source recording date and file mtime. A preview-policy save grants no cleanup permission. There is no automatic removal. Preview JSON exposes `automatic_cleanup: false` and `execution_available: false`: an observational preview cannot be executed directly. The separate [reviewed staging/restore workflow](cleanup.md) now supplies a concrete reviewed operation; staging does not free disk space.

## Accounting scope

Root totals count observed regular files and link entries, with hard-link identities counted once inside that root. Categories separate the selected SQLite database/sidecars, saved audio, models, processing files, playback, clipboard recovery, evaluation reports and other files. A database override outside the selected root is measured separately. Referenced source audio outside the root is inspected only at its exact saved locator, grouped by physical file identity, and is not added to the root total. External source directories are not recursively inventoried. Scope totals should not be added together as a physical-space calculation: links can cross scopes.

Logical file sizes, allocated blocks and available volume space are distinct observations. APFS clones/compression/shared extents make actual freed space uncertain; no reclaimed-space claim is made. Missing or unresolved sources are reported, not assumed to consume zero bytes or repaired. Scan bounds are 100,000 entries and 32 directory levels; child symlinks/special files are not followed. Errors, skipped links and bounds mark the inventory incomplete. Directory metadata itself is excluded. Files and SQLite state can change during observation; this is not an atomic deletion plan.

## Conservative preview rules

Only recognized job-owned scratch layouts are considered: complete Whisper chunk JSON, normalized PCM and native runtime request files under a canonical run UUID in `models/work`. A saved completed status, known ownership protocol and valid completion time are required. Partial, unexpected, unknown-owner and out-of-range chunk names remain protected. Failed/interrupted work remains recoverable regardless of age. Active processing remains protected.

Original or referenced audio, all model data, SQLite data, playback, recovery journals, lease files and other unknown files are excluded. Inode matches to saved audio protect aliases regardless of path; files with multiple hard links remain excluded. Any speaker example, manual correction history or unresolved meeting finalization protects that recording's completed job files. Future/invalid timestamps cannot age files into eligibility. These rules live in Domain/Application; filesystem layout observation and SQL projections stay in outer adapters. No embeddings or person names are fetched for this report.

Candidates are **potential completed files**, not authority to delete. The separate reviewed operation requires a concrete file list, fresh source/reference/job checks, identity revalidation, exclusive ownership and restore support. An old preview or a saved non-null age must never implicitly authorize removal.

## Native behavior and evidence

Policy preview/save, storage totals, protected reasons and expandable path warnings are available without blocking GPUI rendering. Filesystem work uses a blocking worker; SQL and aggregate preparation run through background application calls. Display renders bounded candidate/warning rows; full JSON is available through CLI and a deliberate Copy action. The Storage task has its own busy state, so inference elsewhere does not disable ordinary inspection. A conflict or invalid age returns actionable feedback and leaves controls usable after dismissal.

91 workspace tests pass. Actual SQLite/filesystem tests cover policy defaults and concurrent revisions, read-only database access, time boundaries, source-audio hard links, active/recoverable/unknown work, speaker/correction/finalization guards, partial artifacts, database overrides, external audio and no initialization. A real public-file Whisper job was inspected while active and after cancellation; its saved chunk was protected in both states. Native verification exercised hypothetical preview, a conflicting CLI save, recovery/refresh, successful policy save, Keep-all restoration, invalid-age recovery, warning expansion and protected-work display. The clipboard-copy button was not exercised in this pass.

The isolated library inventory observed approximately 1.38 GB of root file sizes and 425 MB of external referenced audio; two known missing source references kept completeness false. Five final CLI inspections took 11.6–12.5 ms each on this Mac mini. These are small, bounded metadata probes, not a large-library latency guarantee or a measurement of space reclaimable from shared APFS data. Current real jobs are recent; positive old-job candidate selection is verified with deterministic SQLite/filesystem fixtures, not fabricated ages in personal data.

Explicit schema-3-to-4 migration was tested on an isolated snapshot, then on the personal Parakeet Local library after a SQLite backup. All 13 pre-existing table content hashes were unchanged. The personal library remains 18 recordings/three confirmed examples, Keep all/revision 0, with automatic transcription still off. Original MacParakeet sources remain read-only. Private receipts are under `.local/storage-pass/`; public aggregates are in `benchmarks/results/2026-10-27-storage-preview.json`.

Next independent step: reviewed, recoverable cleanup of eligible completed scratch with fresh revalidation and restore support. That code can be developed using current fixtures; applying any policy to personal data remains an explicit action. Raw/source audio removal, model removal, abandoned-job recovery and speaker deletion remain separate operations.
