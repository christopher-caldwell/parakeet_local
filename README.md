# Parakeet Local

A personal Apple Silicon macOS transcription workbench: one Rust application core, a JSON CLI, and a native GPUI desktop. This is an initial working implementation, not a completed or human-validated MacParakeet replacement.

The development library already contains ten imported MacParakeet records (six meetings and four media items), plus public evaluation fixtures. Original recordings remain referenced in place. No source files or source database are modified. Model weights, private transcripts, audio, embeddings, caches and test databases are excluded from Git.

## Try the existing development library

```sh
scripts/run-desktop .local/app

target/debug/parakeet --data-root .local/app doctor
target/debug/parakeet --data-root .local/app meetings list
target/debug/parakeet --data-root .local/app models selected
```

The app bundle is `target/Parakeet Local.app`. Use the launcher above to select the existing development library explicitly. Opening the bundle directly selects `~/Library/Application Support/ParakeetLocal`; desktop startup never creates or migrates a library silently.

## Build and initialize a separate library

Requirements: Apple Silicon, macOS 14+, Rust 1.99.1, macOS command-line developer tools, CMake, FFmpeg/FFprobe, and yt-dlp for requested YouTube operations. The current build works with Swift 5.8.1 and the macOS 13.3 SDK. `scripts/cargo` avoids an older Homebrew compiler taking precedence over the pinned rustup compiler.

```sh
scripts/build-app
scripts/cargo test --workspace
scripts/cargo clippy --workspace --all-targets -- -D warnings

# Explicit provisioning, using an independent storage directory:
target/debug/parakeet --data-root /path/to/new-library db create
target/debug/parakeet --data-root /path/to/new-library db migrate
target/debug/parakeet --data-root /path/to/new-library models install whisper-base.en
target/debug/parakeet --data-root /path/to/new-library models install nemotron3-fast128
scripts/run-desktop /path/to/new-library
```

Ordinary operations do not migrate databases or download weights. SQLx checked-query metadata is committed in `.sqlx`; builds use it offline. After changing SQL or migrations, `scripts/prepare-sql` creates an isolated compiler-only schema and regenerates that metadata. It requires `sqlx-cli` 0.8.6 built with SQLite support.

## Useful commands

All normal results are JSON envelopes (`schema_version: 1`, `ok`, `data` or `error`). Progress is stderr. Help/version are plain text. Exit codes: 0 success, 2 invalid input, 3 missing entity, 4 confirmation required, 5 conflict, 1 other failure. Use `--data-root` and optionally `--database` to isolate experiments. `--json` is accepted; JSON is already the default.

```sh
target/debug/parakeet --data-root .local/app meetings discover
target/debug/parakeet meetings import /path/to/audio-or-artifacts --dry-run
target/debug/parakeet --data-root .local/app meetings import /path/to/artifacts
target/debug/parakeet --data-root .local/app meetings import-source-db /path/to/macparakeet.db
target/debug/parakeet --data-root .local/app meetings show RECORDING_ID
target/debug/parakeet --data-root .local/app meetings tracks RECORDING_ID
target/debug/parakeet --data-root .local/app meetings rename RECORDING_ID 'My title'
target/debug/parakeet --data-root .local/app meetings export RECORDING_ID --format markdown
target/debug/parakeet --data-root .local/app meetings play RECORDING_ID --track TRACK_ID --start-ms 10000 --duration-ms 5000
target/debug/parakeet --data-root .local/app transcribe RECORDING_ID --track TRACK_ID --model selected-asr
target/debug/parakeet --data-root .local/app transcribe-all RECORDING_ID --model selected-diarization
```

`meetings import --dry-run` inspects candidates without opening or creating a destination database. Mutating imports are additive and idempotent. Re-import does not refresh or overwrite local overrides. Processing creates new runs and retains old output/corrections. `transcribe-all` attempts every available authoritative source, preserves partial results, and reports failure if any source fails. Playback and exports are deliberate side effects; exports are returned in JSON rather than silently writing arbitrary files.

Models expose `list`, `install`, `verify`, `remove`, `selected`, `select`, and `select-embedding`. Explicit verification hashes against the pinned checksum catalog and updates a verification receipt. Ordinary readiness checks validate that receipt against file size/mtime; failed verification invalidates old receipts. Installation, inference and removal share a process/file lease and fail explicitly when busy.

Speakers expose profile creation/rename/deletion, verified `enroll`, `examples`, `remove-example`, `candidates`, `suggest`, `suggestions`, `resolve-suggestion`, individual `assign`/`unknown`/`text`/`undo`, and `split-cluster`/`merge-clusters`/`undo-cluster-edit`. Run `speakers COMMAND --help` for inputs. Suggestions require explicit score, margin and sample-count policies, write inspectable receipts, and never enroll themselves. `resolve-suggestion --action accept --segment SEGMENT_ID` requires an explicit segment; undo is recorded separately. Cluster edits derive new diarization results without changing raw predictions.

`speakers review-suggestion ID` returns a saved comparison and its fully covered current transcript segments. GPUI also supports comparison creation, evidence playback, explicit confirmation/replacement, rejection and undo. No comparison decision becomes verified enrollment data.

`jobs pending` is read-only. `jobs recover` explicitly marks abandoned ownership-aware runs interrupted while preserving active/older unmanaged runs and original evidence. `jobs retry RUN_ID` creates a linked new speech run from a failed/interrupted request with the same pinned model revision. GPUI has a Processing jobs screen and retry controls in recent recording history. See [recovery semantics and evidence](docs/processing-recovery.md).

`capture devices`, `permissions` and `recoverable` are inspections. `capture record` needs an explicit microphone UID and/or `--system`; `capture recover` is restricted to this application's own capture directories. `dictation listen` starts the configured native hold bindings. `dictation history` and `dictation show ID` expose retained results. `dictation audio-settings` without flags is read-only; `--microphone-uid UID`, `--clear-microphone` and `--system true|false` change only the requested preferences. Shortcut settings accept editable JSON through `shortcuts set FILE` and are also editable in GPUI. `meetings play-segment RECORDING_ID SEGMENT_ID` resolves playback from the shared timeline service.

`prompts put FILE` saves a disabled authorization state unless `--authorize` is explicitly supplied; `prompts revoke ID` revokes it. Profiles specify an exact bundle identifier, user prompt, absolute CLI executable, argument array, declared local/remote route and timeout. `prompts test BUNDLE_ID TEXT --dry-run` resolves without execution. No remote route has been authorized during development.

`media inspect URL` reads metadata, `media import URL` downloads locally, and `media transcribe URL` also runs selected local ASR. Configure `PARAKEET_YT_DLP` if yt-dlp is not on PATH. The desktop additionally checks the existing MacParakeet bundled executable. Reliable source titles and explicit title overrides survive reprocessing. An ASR failure returns `ok: false`, exit 1 and a `retained_result` identifying the saved import, rather than reporting complete success or discarding the download.

## Evidence and next checkpoint

See [verification](docs/verification.md), [audio models](docs/audio-models.md), [speaker recognition](docs/speaker-recognition.md), and [the human checkpoint](docs/human-checkpoints.md). [Benchmark preparation and recognition probes](benchmarks/README.md) live in `scripts/` and `benchmarks/`. Public numeric summaries contain no personal transcript content; complete local run receipts remain in ignored storage.

Native screen/audio permissions, physical keyboard delivery, supported target editors, and human voice identity are separate acceptance checks. The current direct Accessibility insertion adapter works with TextEdit; unsupported editors retain text and report failure. Clipboard paste fallback is not implemented. This project does not yet establish better personal transcription or speaker accuracy than MacParakeet.
