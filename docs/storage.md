# Storage

The native **Storage** screen and the JSON CLI share `application::storage::StorageService`, which reports what the library uses on disk. Inspection reads file metadata only. It never deletes or moves files, never opens model weights or decodes audio, and never initializes or migrates a database. The only code that replaces files is the raw-capture archive below.

```sh
parakeet --data-root <existing-root> storage usage
parakeet --data-root <existing-root> storage archive-audio --dry-run
parakeet --data-root <existing-root> storage archive-audio
```

## Raw recordings: seven days, then lossless

Captured audio (`*-raw.caf`, application-owned) is kept exactly as recorded for **seven days**, so a wrong transcript or speaker label can be redone from the original. After that it is replaced by a **lossless WavPack copy**, preserving the captured floating-point samples exactly. Lossy encoding is never used and nothing is deleted outright.

- The copy is written beside the original, then both are decoded and compared sample by sample (exact decoded sample bytes; no tolerance). Any difference or error leaves the original untouched.
- Only then does the library switch the track to the copy (a compare-and-set locator update, after the copy is validated as decodable audio) and the raw file is removed. The track keeps its transcripts and voice examples; its history records the original path, original hash and the verification.
- Tracks with a queued or running job are skipped and retried next time. Imported (externally owned) audio is never touched.
- The desktop app does this in the background once at startup, without disabling other controls. From the CLI use the commands above.
- Age is the file's modification time, which for a capture is when recording ended.
- Microphone captures keep the device's own channel count (a mono headset stays mono), at 48 kHz or more so a renegotiating Bluetooth device does not change the file format mid-recording.

## Accounting scope

Root totals count observed regular files and link entries, with hard-link identities counted once inside that root. Categories separate the selected SQLite database and its sidecars, saved audio, models, processing files, playback, clipboard recovery, evaluation reports and other files. A database override outside the selected root is measured separately. Referenced source audio outside the root is inspected only at its exact saved locator and is not added to the root total.

Logical file sizes, allocated blocks and available volume space are distinct observations. APFS clones, compression and shared extents make actual freed space uncertain; no reclaimed-space claim is made. Missing or unresolved sources are reported, not repaired. Scan bounds are 100,000 entries and 32 directory levels; child symlinks and special files are not followed. Errors, skipped links and bounds mark the inventory incomplete.

The retention-policy preview and the storage lock that served the removed cleanup feature were deleted on October 28: once cleanup was gone they could never delete anything. The `storage_policy` and `cleanup_reviews` tables remain in the schema, unused, so existing libraries need no migration.
