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
