# Architecture and decisions

`parakeet-domain` owns observed values, validation, chronology, gesture transitions, compatible voice matching, and evaluation math. It has no runtime or provider dependencies. Serde is a neutral representation helper and UUID supplies deterministic imported IDs.

`parakeet-application` owns focused feature services, capability contracts, and persistence Queries/Writes. `parakeet-database` implements SQLx operations and explicit database provisioning/migration. `parakeet-infrastructure` owns artifact/file access, FFmpeg, models, subprocesses and macOS adapters. Binary entry points explicitly compose the graph. There is no service locator or generic repository.

Database lifecycle is explicit: create an empty local database, then migrate it. Ordinary reads, desktop startup, and doctor never create or migrate a database. SQLite does not have PostgreSQL owner/runtime roles; that part of Playbook migration guidance does not apply. A local single-user database uses foreign keys, WAL, a busy timeout and short atomic mutations.

Imported source records have UUIDv5 identities based on original source identity; local new operations use UUIDv4. Imported snapshots are preserved, and re-import does not overwrite local title overrides or manual corrections. Original source labels are claims without confirmed identity. Source kind is never a person identity.

Reference inspection (October 27): Playbook 0.3.0; Smaller Talk HEAD 19d69563f979a8a80c311fbc0389453f440a390a (GPUI pre 0.3.9, Rust 1.99.1, SQLx 0.8.6 and local frame-memory patches); Control Tower current clone and ADRs 0004–0006; MacParakeet current clone and artifact v1 contract. Control Tower's shared-core rule is retained; its HTTP entry point is not required here. Only the relevant GPUI foundations are vendored and reused.

The complete FluidAudio package requires Swift tools 6.0; installed Swift is 5.8.1. A minimal, attributed Nemotron source subset now builds independently with that compiler, alongside speakrs and whisper-rs. Measured comparisons drove the diarization preset choice; see audio-models.md and verification.md. Model installs are deliberate commands; processing cannot trigger downloads.

Application services have operation-specific Inputs/Results/Errors and own both capability and Queries/Writes ports. Binary entry points own environment configuration, tool paths, data roots and explicit composition. SQLx checked external query files are compiled against committed offline metadata. The variable-schema MacParakeet reader uses runtime SQL only for whitelisted third-party schema projections; database ownership/migration PRAGMAs are operational introspection exceptions.

An application_id marker guards the writable database boundary. Earlier development databases are recognized only by the exact initial migration checksum. Ownership is checked through a read-only connection before enabling WAL or migrations. Foreign databases are rejected unchanged. Source reads use their own read-only snapshot adapter and do not pass through application provisioning.

SQLite atomic imports, terminal run settlement and compare-and-append correction/suggestion revisions handle CLI/desktop concurrency. A single native capture lease and a single global model lease per data root prevent unsafe overlap across processes. Capture and inference leases are independent. Concurrent dictation during an owned meeting capture is currently rejected; a model-busy dictation retains its audio/text and reports failure. There is no priority queue or cross-process in-memory model server.

GPUI owns forms, navigation, focus, rendering, copy actions and transient task feedback. Tokio runs application calls; blocking model work uses blocking workers or bounded native subprocesses. The render thread does no decoding, inference, model hashing or SQL work. Native helpers write separate source CAF and sidecars with host-clock offsets, bounded buffers and capture-health evidence. A parent-death watchdog releases orphan event taps; capture EOF/timeout and restricted recovery preserve interrupted audio.
