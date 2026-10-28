# Requirements

The product authority is the user's original brief, "MacParakeet Clone — Product Requirements and Autonomous Implementation Brief" (October 27, 2026). It is kept outside Git (the repo is public): read `.local/brief/original_brief.md` on a machine that has it, and ask the user for it otherwise. Cite it as B§n. This page extracts the requirements that earlier summaries lost or inverted; it does not replace the brief. Deviations are recorded in `.local/brief/parakeet_local_design_deviations.md`.

## Product

- A personal, local-only macOS tool. Transcription, diarization and voice embeddings run on-device; nothing goes to an AI service without explicit authorization (B§2).
- Priorities: reliable microphone selection, a responsive native UI, trustworthy speaker identification, correct transcript chronology, and no fabricated speech (B§1).

## Me, Them and Unknown (B§7, B§9, B§26)

- Source categories are **Me** (microphone), **Them** (system audio) and **Unknown** (unattributed or ambiguous). They are source attribution, not confirmed identities.
- Never label all microphone speech as the user. Bleed, echo, extra voices or ambiguity make a segment Unknown. Prefer Unknown to a false "Me".
- Distinguish original audio, source attribution, inferred speakers and confirmed identities.

## Learning from the user (B§1, B§10, B§20)

- The system learns from explicit human identification **and corrections**: a naming, accepted suggestion or correction the user confirms creates verified, reversible, inspectable voice examples.
- Model predictions and imported labels are never verified examples and never auto-enroll.
- Recognition is automatic: embed each new cluster, compare with compatible enrolled profiles, apply calibrated acceptance criteria, and suggest an identity or Unknown. Criteria are proposed from measured evidence on tuning data, never invented, and held-out data stays untouched.
- Profiles hold several examples, model/domain provenance, enrollment and recognition history, and optional notes.
- A training session should let the user identify several speakers quickly (play, choose person/new/Unknown, save) without listening to whole meetings.

## Capture and transcript (B§7, B§17)

- Meetings record with local transcription; independent mic and system capture with real offsets; sort by audio time; keep overlap, untimed text and raw output.
- A silent or failed microphone must not yield an apparently successful recording; device changes are handled and surfaced.
- Treat the false-attribution defect class as a priority: evaluate silence/noise, mic-system duplication, bleed, echo removal, missing audio, chunk boundaries and word alignment.

## Dictation, AI, media (B§8, B§13, B§14)

- Several shortcuts work at once on laptop and external keyboards; correct key lifecycle; clipboard preserved; failed insertion keeps the text.
- AI enhancement is per-bundle-ID, off by default, explicitly authorized, with raw text preserved.
- YouTube imports use source metadata for titles; every item has an editable, persistent title.

## Engineering (B§4, B§5, B§22, B§23, B§25)

- One shared Rust core with CLI and GPUI entry points; the CLI is the main verification surface.
- Report evidence by category: implemented, unit-tested, integration-tested, real-model-tested, real-recording-tested, hardware-tested, human-validated, blocked or unverified. Never claim an accuracy or performance gain that was not measured.
- Schedule concurrent work deliberately (capture, dictation, transcription, diarization, installs, CLI) without a distributed scheduler.
- Record material deviations from the brief and their reasons.
