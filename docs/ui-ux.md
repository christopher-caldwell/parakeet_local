# Native UI and usability pass

The October 27 pass inspected the installed Slack and MacParakeet interfaces through native accessibility/screenshot controls, MacParakeet's checked-out library/meeting screenshots and reader source, and Smaller Talk's current GPUI/theme implementation. Slack and MacParakeet were used for read-only design inspection; no messages were sent or source recordings edited.

## Design and interaction

Slack informed the persistent navigation, visible active destination, compact rows, subdued secondary actions and separation between a list and its content. MacParakeet informed source categories, recording metadata, the transcript-first reader and deliberate processing/playback actions. The implementation remains native GPUI, using the existing GPUI Base dependency and system font; it does not embed either reference application.

The Library has three regions: grouped navigation, a persistent recording list, and a reader. All/Meetings/Media filters, date/duration summaries and a debounced library search make recordings easier to find. Search generations discard stale results. The reader separates Transcript, Audio & sources, and Processing. Source paths, relinking, raw word evidence and run history stay available without occupying the ordinary reading view.

Transcript rows show source and audio time beside a distinct anonymous, Unknown or confirmed speaker label. Text can be selected, copied per segment or copied as a corrected chronological transcript. Each row opens its own multiline correction editor. Save closes it after success; Undo restores saved evidence and closes the stale editor. A manual Unknown choice remains Unknown through text edits; the same Domain correction resolver is used by the reader and text/Markdown exports. No correction enrolls a voice example.

Recording titles are edited inline with Save/Cancel and survive reopening. Import audio opens the native file/folder picker; explicit path and YouTube inputs remain available. Cancelling the picker does not import anything. Reimport uses the existing idempotent application operation. Import warnings remain in the activity message rather than being discarded.

All shared controls use GPUI Base's focusable, keyboard-aware Button. Selected rows retain their active background on hover; disabled actions communicate unavailable operations. Transcript text and speaker labels are exposed in the accessibility tree. This is not a complete VoiceOver certification.

| Shortcut | Action |
| --- | --- |
| Command-1 / 2 / 3 / 4 | Library / meeting recording / dictation / speakers |
| Command-Shift-F | Search library |
| Command-F | Find within the current transcript, or search library |
| Command-O | Import audio picker |
| Command-Return | Save the focused transcript correction without inserting a newline |
| Escape | Close an editor, find field or transient panel without saving |
| Tab / Shift-Tab | Move between enabled controls |

Keyboard focus is restored when a transient control disappears, using GPUI's focus-lost subscription. Normal Return remains a newline in a transcript editor and saves an inline title. Navigation and application work remain separate: all persistence, processing and correction actions call the existing application services on background tasks.

A persistent playback strip exposes Stop from every area. Starting another clip stops the previous one. The owned playback task is cancelled, its afplay child uses kill-on-drop, and a scoped owner removes only its generated temporary WAV, including cancellation while the blocking writer finishes. Source audio and capture recovery files are not cleanup targets.

Models show human-readable names, footprint, readiness and selected roles. Download/removal remain deliberate; model, person and verified-example removal require a visible confirmation with Cancel. The recording screen reports explicit microphone selection, a no-input state and Refresh inputs. Dictation and AI settings preserve their existing consent boundaries. Enrollment shows the prepared recording/source and editable track-local bounds; unavailable enrollment/playback actions are disabled.

## Verification and remaining limits

Native checks use `.local/ui-pass/app`, a SQLite backup of the new application's database and local model copies. The original MacParakeet database is not used as a test target. A public WAV was imported through the actual macOS picker, renamed and transcribed locally. Corrections and title changes were made only in the isolated library. Private receipts and screenshots remain under `.local/ui-pass`.

Checked through the native app: library search and empty state, transcript find, inline title saving, multiline correction saving, Unknown persistence, Undo, Command-Return, navigation shortcuts, playback/Stop, source/processing tabs and model-removal cancellation. The reader was resized to approximately the 920-point minimum width and retained its find, reader and navigation controls. Settings and profile screens were inspected without enabling dictation, recording audio, enrolling voices or authorizing an AI route.

The automated gate includes 59 workspace tests (Application 2, CLI 7, Database 15, Domain 22, Infrastructure 13), Clippy with warnings denied, checked offline SQLx metadata and the native signed bundle build. Two new Domain tests cover identity/text/undo isolation; the CLI integration test covers corrected text export and restoration. The opt-in model cache test remains separate and is not counted as an ordinary passing test.

The library currently renders its returned recording rows directly and pages transcripts at 40 segments. Large-library virtualization, full VoiceOver navigation, continuous playback/waveform seeking, sustained resource stress and a measured comparison with Slack or MacParakeet are not established by this pass. Existing microphone permission, physical keyboard/dictation, M3 and real-editor fallback limitations still apply. Visual improvements do not establish better ASR or speaker accuracy.

The public load probe processed 634.578 seconds of retained Big Buck Bunny audio through Whisper Base in 21.360832 seconds, producing 46 segments. While that job was running, native navigation to Models succeeded and model mutation controls were visibly disabled. At the captured running-state observation, render construction peaked at 2.31025 ms and the largest interval of the nominal 100 ms UI heartbeat was 267.758958 ms. These are construction/heartbeat measurements, not end-to-end frame latency or a sustained performance guarantee. An RSS observation was 524,608 KiB for the desktop process, excluding some device allocations; it is not peak memory. Complete receipts: `.local/ui-pass/load-observation.json` and `ui-report.json`.
