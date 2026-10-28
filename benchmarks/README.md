# Reproducing the small public probes

Use the pinned models described in [audio-models](../docs/audio-models.md). Network steps below download public data only; all inference is local. Outputs, audio, model clones and private receipts belong under ignored `.local/`. These scripts are diagnostic subsets, not whole-corpus benchmark claims.

## Diarization

Obtain the official annotations from `https://github.com/joonson/voxconverse` (version 0.3) at `.references/voxconverse`. Inspect the dataset's CC BY 4.0 notice and source-media conditions. The preparer downloads only selected public WAV entries using HTTPS ZIP ranges.

```sh
python3 scripts/prepare_voxconverse.py --split dev
python3 scripts/prepare_voxconverse.py --split test --output .local/public/voxconverse-heldout
target/debug/parakeet --data-root .local/app benchmark run .local/public/voxconverse/suite.json
```

Each suite's `models` array determines the candidates; use a copied suite to compare `nemotron3-fast128`, `nemotron3-offline` or `speakrs-community-1-coreml`. Fix development choices before running the test suite. The scorer defaults here to zero collar with overlap included. Reports record input/reference hashes and model/runtime/configuration. They remain in the selected root's `evaluations/` directory.

## ASR, non-speech and identification

Download the official LibriSpeech dev-clean archive from `https://www.openslr.org/resources/12/dev-clean.tar.gz` to `.local/public/librispeech/dev-clean.tar.gz`. Official corpus/license: OpenSLR 12, CC BY 4.0. Expected archive MD5: `42e2234ba48799c1f50f24a7926300a1`. The common fixture helper checks the archive, bounds extraction and selects deterministic, disjoint utterances from six dataset speakers.

```sh
python3 benchmarks/prepare_asr.py
target/debug/parakeet --data-root .local/app benchmark run .local/public/librispeech/asr-development-suite.json
# Record model choice before evaluating this separate suite:
target/debug/parakeet --data-root .local/app benchmark run .local/public/librispeech/asr-heldout-suite.json
target/debug/parakeet --data-root .local/app benchmark run .local/public/non-speech/suite.json
python3 benchmarks/recognition-librispeech.py --backend coreml
python3 benchmarks/recognition-librispeech.py --backend cpu
```

The identification probe requires the corresponding speakrs model installed in `.local/app/models`; it uses an independent APFS clone and database under its result directory, preserving personal model selection. Dataset IDs serve as fixture identities. It enrolls two disjoint examples for each of three speakers, tunes score on designated utterances, then evaluates separate known/unfamiliar utterances. Do not use held-out results to retune the policy and continue calling them held-out.

`recognition-public.py` is an older strict-threshold regression probe. Its recall assertion currently fails and is deliberately retained; see [verification](../docs/verification.md). A calibrated result is not evidence that the strict gate passed.

ASR suites declare orthographic normalization explicitly. Non-speech suites use an empty reference and report false recognized words rather than undefined WER as zero. The checked-in [numeric summary](results/2026-10-27-public-summary.json) excludes local paths, raw speech, vectors and personal labels. Full run receipts have the precise evidence needed to inspect a result.

## Expanded public evaluation and runtime reuse

```sh
python3 scripts/prepare_voxconverse.py --split test --count 20 --output .local/expanded-eval/voxconverse
python3 benchmarks/prepare_asr_expanded.py --archive .local/public/librispeech/dev-clean.tar.gz --output .local/expanded-eval/asr
```

The VoxConverse filter yields 16 entries, including the earlier three. Set the copied diarization suite's `models` to `["nemotron3-fast128"]` to reproduce the fixed-preset run. Run each ASR model separately in a copied one-model suite to measure reuse across all 24 utterances. Use an explicitly provisioned isolated root with deliberately installed/verified model copies, then `benchmark run SUITE.json`; no benchmark implicitly installs weights. The [expanded numeric report](results/2026-10-27-expanded-public.json) includes hashes, errors, counts and resource measurements; no audio/transcript text is included.

The opt-in Rust cache probe requires `PARAKEET_MODEL_PROBE_ROOT` and `PARAKEET_MODEL_PROBE_AUDIO` as **absolute paths** to disposable model copies and a public audio file. Run `scripts/cargo test -p parakeet-infrastructure --test model_cache -- --ignored --nocapture`. It temporarily removes/restores the copy's verification receipt, checks stable output and waits 75 seconds for idle eviction. Do not point it at a live personal model root. It performs no downloads. Ordinary workspace tests skip this environment-dependent probe; its separate execution is recorded in verification.

## Long-clip chunking

```sh
python3 benchmarks/prepare_long_asr.py .local/public/librispeech/dev-clean.tar.gz .local/long-asr 1272 1462 2035
target/release/parakeet --data-root .local/long-asr-root benchmark run .local/long-asr/suite.json
```

Single utterances never reach chunking; these multi-minute joins do. [Result](results/2026-10-28-long-clip-chunking.json): parity with the old fixed windows, not a gain.

## Meeting-like tracks and model comparison

```sh
# AMI headset audio (CC BY 4.0): https://huggingface.co/datasets/edinburghcstr/ami, ihm/test-00000-of-00004.parquet
python3 benchmarks/prepare_ami.py .local/ami/test-00000.parquet .local/ami
```

Run a copied suite once per `models` entry (`whisper-small.en`, `-medium.en`, `-large-v3-turbo`, `-large-v3`). [Result](results/2026-10-28-whisper-models.json).

## Muted or idle microphone

```sh
python3 benchmarks/prepare_muted.py .local/muted
```

Any word printed for these tracks is false. [Result](results/2026-10-28-muted-mic.json): small.en prints none; large-v3 still prints a few words on breathing-like noise.

## Speaker-attributed word diagnostics

`benchmark score-speakers REFERENCE.json PREDICTION.json` computes anonymous-stream cpWER through the shared application service without database initialization. [Input/protocol and limitations](../docs/speaker-word-evaluation.md) are explicit. For external scorer verification, deliberately install `meeteval==0.4.3` in an isolated Python environment and run `python benchmarks/check_speaker_wer.py --output .local/speaker-wer-oracle.json` using that interpreter. The fixed seed generates 250 cases; no speech/network inference is involved.

```sh
# Fresh independent roots; explicit APFS clones of already-installed verified models:
python3 benchmarks/speaker_words_pipeline.py --root .local/speaker-word-eval --models-source .local/app/models --run
python3 benchmarks/speaker_words_pipeline.py --root .local/speaker-word-estimated --models-source .local/app/models --word-timestamps --run
```

The preparer refuses an existing database/audio target, makes six different public utterances into two synthetic conversations, and records source placements/hashes. `--word-timestamps` also records the same-ASR segment-attribution baseline, avoiding a recognition-change confound. No words/voices are inferred as reference truth from model predictions. Utterance placements cannot serve as word/speech-alignment ground truth. The small results remain poor and must not be generalized to meetings. No production model choice or personal threshold is silently changed.

## Native UI observations

The [native performance guide](../docs/ui-performance.md) describes opt-in release builds, phase markers, isolated data snapshots, native wheel interactions and macOS resource accounting. `scripts/ui-performance.py` records process/frame observations; it does not drive the UI or create a database. `scripts/prepare-ui-fixture.py` generates the deterministic long transcript. The [numeric report](results/2026-10-27-native-ui.json) contains aggregate results without private recording content. Keep raw traces and process paths under ignored local storage.

## Decoder memory and meeting-workflow probes

[Workflow behavior](../docs/meeting-workflow.md) and the [numeric report](results/2026-10-27-meeting-workflow.json) separate actual system capture/recovery, real local model regressions, SQLite failure injection and synthetic decoder resource measurements. Original recordings are never moved to inject failures; use newly generated copies under an isolated root.

The ignored `audio::decoder_tests::decoder_resource_probe` requires explicit `PARAKEET_PCM_PROBE_INPUT`, `PARAKEET_PCM_PROBE_MODE=memory|disk`, `PARAKEET_PCM_PROBE_MS`, and a new `PARAKEET_PCM_PROBE_OUTPUT` in disk mode. It decodes only; no model downloads or inference. Generate silence with `ffmpeg -f lavfi -i anullsrc=r=16000:cl=mono -t 1800 -c:a flac <new-file.flac>`. Compile with `scripts/cargo test -p parakeet-infrastructure --lib --no-run --message-format=json`, then select that command's emitted test executable. Run it directly with `--ignored --exact audio::decoder_tests::decoder_resource_probe --nocapture`, separately for each mode. Measure the executable with `/usr/bin/time -l`; measuring the Cargo command would include build work. Output and raw resource logs belong under ignored `.local/`. These preparation-stage RSS observations do not measure Core ML or all filesystem-cache memory.
