# Speaker-word evaluation and experimental timing

Ordinary WER can be low while words are assigned to the wrong voices. The shared `ScoringService` now computes concatenated minimum-permutation WER (cpWER): chronological words within each anonymous voice form one stream, and a global minimum-cost one-to-one assignment compares predicted/reference streams. Missing streams incur deletions and extra streams incur insertions. This follows the [MeetEval metric/algorithm](https://github.com/fgnt/meeteval/blob/main/doc/algorithms.md); the Rust implementation was independently compared with installed **MeetEval 0.4.3 on 250 deterministic synthetic cases**. All total error counts and reference-word denominators matched, including empty/missing/extra streams. Named identity and word timestamp accuracy are separate concerns.

## Read-only scoring

```sh
target/debug/parakeet benchmark score-speakers reference.json prediction.json --normalization english_case_punctuation
```

Each file declares `dataset`, `labels`, `audio_sha256`, `streams` (distinct nonempty `speaker` plus already-concatenated chronological `text`) and optional `provenance`. References must declare `verified_public`, `verified_personal` or `synthetic`; unverified personal predictions are refused as ground truth. Both documents must declare the same valid audio hash. This checks their declared identity, not the existence or correctness of external ground truth. The adapter hashes the exact parsed bytes, bounds each file to 8 MiB and does not initialize storage. Domain scoring supports at most 64 streams per side, 20,000 words per side and 25 million pairwise word cells. Larger inputs return a structured error. Existing diarization-file scoring also now goes through this application-owned reader/service.

Normalization is explicitly exact whitespace or `english_case_punctuation`, with the same rules as ordinary WER. Empty reference speech has undefined cpWER, while error counts still report extra words. A stream labeled `__unattributed__` is an evaluation stream, so the permutation can map it to one reference voice; its separately reported unknown-word count must be inspected. The metric does not mean an unknown voice was recognized as a person. It has no time constraints and does not penalize absolute timestamp drift.

## Constructed public pipeline probe

`benchmarks/speaker_words_pipeline.py` makes six alternating turns from two different LibriSpeech utterances for each of three labeled voices. Audio is mono PCM16/16 kHz at 0.5 per-clip gain. One variant has 800 ms gaps; the other mixes adjacent clips by 1,000 ms. Reference words and source voices are known, but utterance spans include pauses and are **not word/speech-boundary ground truth**. No DER or timestamp-accuracy score is computed on these constructed spans. These are diagnostic mixtures, not natural meetings or held-out population accuracy.

Fixed Whisper base/small plus Nemotron fast initially produced segment-level cpWER of 50.00%/46.55% on the sequential fixture and 72.41%/65.52% on overlap. All outputs retain ambiguous/uncovered speech in an unattributed stream. These substantial errors expose a limitation that separate ASR/diarization scores did not establish.

## Opt-in model-estimated words

`transcribe ID --track TRACK --word-timestamps` requests Whisper's experimental token timing. The native Library has the same explicit action and a collapsed original-word-evidence view. Normal transcription/dictation defaults remain unchanged. Model metadata advertises this optional capability; unsupported model routes refuse it. The flag and algorithm identity are persisted and retry preserves the request. Old requests default to segment timing.

The adapter preserves raw token IDs, bytes, probabilities and native 10 ms timestamp values. Whitespace grouping combines decoded subwords, including split UTF-8; it never divides a segment duration equally among words. Invalid/zero-length/out-of-chunk ranges, tokens spanning multiple words or a text reconstruction mismatch leave affected words untimed. Original text, including legitimate repeats, is preserved. Domain reconciliation validates local bounds, recomputes common time from the saved authoritative track origin, preserves unknown source timing and only produces anonymous attribution. Uncertain speech, overlap or missing evidence stays Unknown. Original model words do not overwrite segment corrections or enroll voices.

On paired runs with the **same ASR and diarization output** for each attribution comparison (232 reference words per fixture):

| Variant | ASR | Segment cpWER | Estimated-word cpWER | Estimated-word errors |
|---|---|---:|---:|---:|
| Sequential | base | 50.00% | 35.34% | 82 |
| Sequential | small | 46.55% | 40.95% | 95 |
| Overlap | base | 72.41% | 30.17% | 70 |
| Overlap | small | 65.52% | 31.03% | 72 |

Unknown whitespace-word counts were 38/43/32/27, respectively. Improvement on these two fixtures does not validate timestamp precision, establish personal meeting quality, or imply overlap is easier. Substantial errors remain. The option stays experimental and off by default; no model/policy selection was retuned. [Numeric results](../benchmarks/results/2026-10-27-speaker-words.json) exclude speech text and local paths. Full receipts are under `.local/speaker-word-eval/`, `.local/speaker-word-estimated/` and `.local/word-timing/`.

The explicitly executed cache test also passes repeatable word evidence and switching back to unchanged segment output, as well as verification invalidation and 75-second idle expiry. Unit tests cover source/offset safety, untrusted common timestamps, untimed ranges, UTF-8, repeated words and ambiguous token boundaries. These checks do not establish real word alignment accuracy.
