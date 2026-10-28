# Parakeet Local

The October 27, 2026 user brief is the product authority. This workspace implements a personal Apple Silicon macOS tool with a Rust application core, SQLx/SQLite persistence, CLI, and native GPUI entry point. No HTTP frontend, accounts, synchronization, analytics, or cloud transcription.

Primary correctness requirements: independent microphone/system sources with real offsets; audio-time transcript ordering and explicit overlap; unknown attribution instead of guessed “Me”; raw model output separate from corrections; persistent title overrides; verified enrollment separate from predictions; model/version/capture-domain compatibility; multiple owning hold shortcuts; per-bundle-ID AI routing disabled until explicitly authorized.

Source recordings and the original MacParakeet database are read-only. Work products, model downloads, imported snapshots, and derived audio belong to this application. Referenced originals are fingerprinted; absence or movement is reported, never repaired in place. Private artifacts and model weights are excluded from Git.

## Executable milestones

1. Shared core: build/test; discover and audit actual artifacts; isolated database; import twice without duplicates; list/show/tracks/search/rename/export; source evidence unchanged.
2. Processing: explicit model install/verify; real local ASR and timed anonymous diarization on retained audio; durable run provenance and failures; source offsets preserved; reproducible public and personal diagnostic reports.
3. Recognition: verified examples; compatible embeddings; conservative evidence-based suggestions; reversible corrections; no enrollment from guesses.
4. Desktop: GPUI invokes those services, with background work and explicit errors; library/transcripts/models/speakers.
5. Integrations: capture/health/recovery, shortcuts/insertion, opt-in AI routes, media metadata/import. Hardware claims require actual hardware evidence.

These are implementation milestones, not human approval gates. Unavailable Swift tooling, model access, or hardware permission does not block independent work. Human checkpoints require only actual identity labels, OS permission, physical keyboard/capture checks, and subjective acceptance after useful workflows exist.

Current scope, conformance findings and reset priorities are tracked in the [top-to-bottom project review](project-review.md), audited at `64a6278`. The original product brief remains authoritative; a green build or isolated feature receipt is not whole-product acceptance.
