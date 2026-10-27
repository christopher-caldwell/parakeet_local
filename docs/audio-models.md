# Local models and runtime decisions

Weights are deliberately installed from pinned HTTPS model revisions, hashed against the checked-in `crates/infrastructure/src/model-checksums.json`, and used without network inference. The downloaded-file manifests and verification receipts stay under the selected data root. No SDK auto-download feature is enabled.

| Registry ID | Runtime | Pinned weights revision | Installed bytes in this run |
|---|---|---|---:|
| whisper-base.en | whisper-rs 0.16.0 / whisper.cpp Metal | 5359861c739e955e79d9a303bcbc70fb988958b1 | 147,964,211 |
| whisper-small.en | same runtime | same revision | 487,614,201 |
| speakrs-community-1-coreml | speakrs 0.7.0 Core ML | 67f963b6a1d94fcd4678948237b3cc8bd69bf1d8 | 272,600,614 |
| speakrs-community-1-cpu | speakrs 0.7.0 native CPU | same revision | 32,692,756 |
| nemotron3-fast128 | selected FluidAudio source subset / Core ML CPU+GPU | 1b0b133f6f8820292010afd776d8f9fbc9fca17e | 199,096,435 |
| nemotron3-offline | same runtime, offline preset | same revision | approximately 199 MB |

Whisper converts mono 16 kHz PCM in independent 60-second chunks with no context carryover. Segment timings are model estimates, not word alignments. There is no overlapping chunk concatenation to deduplicate. Text, source, no-speech evidence, chunk energy, configuration and raw output are retained separately from speaker reconciliation. Unknown source origins or inconsistent capture timelines keep track-local timing in evidence rather than inventing meeting times. English-only model variants are currently offered.

Speakrs provides Community-1-style segmentation, WeSpeaker embeddings and clustering. Both backends produced identical diarization scores on the three development clips; Core ML was substantially faster. Its false speech on the short music-heavy `tucrg` clip made a Rust-only runtime an unsuitable default by itself.

The full FluidAudio package requires a newer Swift toolchain than this machine has. A small Apache-2.0 source subset at commit `1b0ed50d686873128a4129b3510ce3914c849bab` implements Nemotron inference independently of the installed MacParakeet app. It removes downloader APIs and unrelated code, and adapts Float16 conversion to the older SDK. The model algorithm and its 0.5 activity threshold are unchanged. Model installation, preprocessing, leases, persistence and timeline normalization remain in Rust. The helper has a parent-death watchdog. Core ML model requirements make macOS 14 the application baseline.

Nemotron preserves overlapping activity and has a maximum of eight anonymous speaker slots. It does not provide identity embeddings. `speakrs-community-1-coreml` supplies the separate embedding path. The selected Nemotron preset was fixed using development clips before the held-out recordings were run. Precise measured results and their limits are in [verification](verification.md).

Inference is bounded to two hours per operation. Whisper decoding is chunked; Nemotron's Rust adapter currently decodes a bounded whole interval to PCM, writes its own scratch file, and the Swift helper maps it and streams one-second inputs. This bounds but does not eliminate long-file memory use. Failures preserve source audio and useful scratch/recovery evidence. Model leases reject concurrent installation/removal/inference rather than overcommitting the machine. Models are loaded per operation; separate CLI processes do not share in-memory models. GPU/ANE utilization, sustained thermal behavior and the M3 laptop remain unmeasured.

Weight licenses are distinct from runtime source licenses. [Whisper conversions](https://huggingface.co/ggerganov/whisper.cpp) declare MIT. The pinned [Nemotron conversion](https://huggingface.co/FluidInference/nemotron-3-diarization-coreml) card declares OpenMDW-1.1. [Community-1](https://huggingface.co/pyannote/speaker-diarization-community-1) and the [WeSpeaker wrapper/weights](https://huggingface.co/pyannote/wespeaker-voxceleb-resnet34-LM) declare CC BY 4.0; the [speakrs export card](https://huggingface.co/avencera/speakrs-models) carries upstream provenance and access-condition notes. No upstream account agreement was accepted and no weights are redistributed in this repository. See [third-party attribution](../THIRD_PARTY.md).
