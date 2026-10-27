#!/usr/bin/env python3
"""Reproducible synthetic speaker-word pipeline diagnostic from public speech.

Six different LibriSpeech utterances from three labeled voices are interleaved.
An overlap variant mixes adjacent clips by one second. Utterance spans are not
speech/word alignments and must not be used as DER or timestamp ground truth.
All inference and scoring uses the shared application CLI. No network calls.
"""
import argparse
import array
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import wave

from public_fixtures import prepare_librispeech

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--fixtures", type=Path, default=Path(".local/public/librispeech"))
parser.add_argument("--root", type=Path, required=True)
parser.add_argument("--cli", type=Path, default=Path("target/debug/parakeet"))
parser.add_argument("--models-source", type=Path)
parser.add_argument("--word-timestamps", action="store_true", help="Evaluate experimental model-estimated words and same-ASR segment baseline")
parser.add_argument("--run", action="store_true", help="Explicitly create an isolated DB, clone verified models and run locally")
args = parser.parse_args()
if (args.root / "parakeet.db").exists():
    raise SystemExit("Use a fresh isolated root; existing databases are not modified by this preparer")
voices = prepare_librispeech(args.fixtures)[:3]
clips = [(voice["speaker"], voice["heldout"][turn]) for turn in range(2) for voice in voices]
ids = {entry["utterance"] for _, entry in clips}
transcripts = {}
with tarfile.open(args.fixtures / "dev-clean.tar.gz", mode="r|gz") as archive:
    for member in archive:
        if member.isfile() and member.name.endswith(".trans.txt") and member.size < 1024 * 1024:
            for line in archive.extractfile(member).read().decode().splitlines():
                key, text = line.split(" ", 1)
                if key in ids:
                    transcripts[key] = text
decoded = []
for speaker, entry in clips:
    path = Path(entry["path"])
    if hashlib.sha256(path.read_bytes()).hexdigest() != entry["sha256"]:
        raise SystemExit("Public fixture no longer matches its manifest")
    samples = array.array("h", subprocess.check_output([
        "ffmpeg", "-nostdin", "-v", "error", "-i", str(path), "-ac", "1", "-ar", "16000", "-f", "s16le", "-",
    ], timeout=30))
    if sys.byteorder != "little":
        samples.byteswap()
    decoded.append((speaker, entry, samples))
args.root.mkdir(parents=True, exist_ok=True)
prepared = []
for variant, gap_samples in [("sequential", 12_800), ("overlap", -16_000)]:
    samples = []
    placements = []
    texts = {voice["speaker"]: [] for voice in voices}
    for speaker, entry, audio in decoded:
        start = 0 if not samples else len(samples) + gap_samples
        end = start + len(audio)
        if end > len(samples):
            samples.extend([0.] * (end - len(samples)))
        for index, value in enumerate(audio, start):
            samples[index] += value * .5
        placements.append({"speaker": speaker, "utterance": entry["utterance"], "source_sha256": entry["sha256"], "start_sample": start, "end_sample": end})
        texts[speaker].append(transcripts[entry["utterance"]])
    pcm = array.array("h", (round(max(-32768, min(32767, value))) for value in samples))
    if sys.byteorder != "little":
        pcm.byteswap()
    audio_path = args.root / f"{variant}.wav"
    if audio_path.exists():
        raise SystemExit(f"Refusing to overwrite an existing fixture: {audio_path}")
    with wave.open(str(audio_path), "wb") as output:
        output.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
        output.writeframes(pcm.tobytes())
    audio_hash = hashlib.sha256(audio_path.read_bytes()).hexdigest()
    reference = {
        "dataset": "Synthetic six-turn/three-voice mixture of LibriSpeech dev-clean (OpenSLR12 CC BY4)",
        "labels": "synthetic", "audio_sha256": audio_hash,
        "streams": [{"speaker": speaker, "text": " ".join(text)} for speaker, text in texts.items()],
        "provenance": {"archive_md5": "42e2234ba48799c1f50f24a7926300a1", "variant": variant, "sample_rate": 16000, "per_clip_gain": .5, "gap_samples": gap_samples, "placements": placements, "speech_or_word_alignment_truth": False},
    }
    reference_path = args.root / f"{variant}.reference.json"
    reference_path.write_text(json.dumps(reference, indent=2))
    prepared.append((variant, audio_path, reference_path, audio_hash))
print("Prepared two synthetic conversations; source words/voices are public labels, not verified word timestamps.", flush=True)
if not args.run:
    raise SystemExit(0)
if args.models_source is None:
    raise SystemExit("--run requires explicitly selected --models-source")
models = ["whisper-base.en", "whisper-small.en", "nemotron3-fast128"]
(args.root / "models").mkdir(exist_ok=True)
for model in models:
    subprocess.run(["/bin/cp", "-cRp", str(args.models_source / model), str(args.root / "models" / model)], check=True)

def call(*command):
    result = subprocess.run([str(args.cli.resolve()), "--data-root", str(args.root), *map(str, command)], capture_output=True, text=True, timeout=600)
    envelope = json.loads(result.stdout)
    if result.returncode:
        raise RuntimeError(envelope)
    return envelope["data"]

call("db", "create")
call("db", "migrate")
for model in models:
    if not call("models", "verify", model)["verified"]:
        raise RuntimeError("Model clone verification failed")
results = []
for variant, audio_path, reference_path, audio_hash in prepared:
    recording = call("meetings", "import", audio_path)["recordings"][0]
    track = call("meetings", "show", recording)["tracks"][0]["id"]
    diarization = call("transcribe", recording, "--track", track, "--model", "nemotron3-fast128")
    for model in models[:2]:
        asr = call("transcribe", recording, "--track", track, "--model", model, *(["--word-timestamps"] if args.word_timestamps else []))
        detail = call("meetings", "show", recording)
        attributes = {row["segment_id"]: row for row in detail["attributions"]}
        streams = {}
        unknown_words = 0
        # The application already orders these segments by audio time. Only its
        # conservative single-speaker attribution is eligible for a voice stream.
        for segment in detail["segments"]:
            assert segment["run_id"] == asr["id"]
            attribution = attributes[segment["id"]]
            if attribution["state"] == "anonymous_model_attribution" and not segment["evidence"].get("uncertain_speech", False):
                speaker = attribution["anonymous_speakers"][0]
            else:
                speaker = "__unattributed__"
                unknown_words += len(segment["text"].split())
            streams.setdefault(speaker, []).append(segment["text"])
        prediction = {
            "dataset": "Local model output on synthetic public conversation", "labels": "model_prediction", "audio_sha256": audio_hash,
            "streams": [{"speaker": speaker, "text": " ".join(text)} for speaker, text in streams.items()],
            "provenance": {"asr_run": asr["id"], "asr_model": model, "asr_revision": asr["model_revision"], "diarization_run": diarization["id"], "diarization_model": diarization["model"], "diarization_revision": diarization["model_revision"], "attribution": "shared application segment reconciliation; no word-time interpolation", "unattributed_whitespace_words": unknown_words},
        }
        prediction_path = args.root / f"{variant}.{model}.prediction.json"
        prediction_path.write_text(json.dumps(prediction, indent=2))
        report = call("benchmark", "score-speakers", reference_path, prediction_path)
        segment_report = report
        if args.word_timestamps:
            streams = {}
            unknown_words = 0
            timed_words = 0
            assert len(detail["word_attributions"]) == sum(len(s["text"].split()) for s in detail["segments"])
            for word in detail["word_attributions"]:
                attribution = word["attribution"]
                speaker = attribution["anonymous_speakers"][0] if attribution["state"] == "anonymous_model_attribution" else "__unattributed__"
                streams.setdefault(speaker, []).append(word["word"]["text"])
                unknown_words += speaker == "__unattributed__"
                timed_words += word["word"]["start_ms"] is not None
            prediction["streams"] = [{"speaker": speaker, "text": " ".join(text)} for speaker, text in streams.items()]
            prediction["provenance"].update({"attribution": "shared application experimental word reconciliation; no timestamp interpolation", "unattributed_whitespace_words": unknown_words, "timed_whitespace_words": timed_words})
            prediction_path = args.root / f"{variant}.{model}.estimated-words.prediction.json"
            prediction_path.write_text(json.dumps(prediction, indent=2))
            report = call("benchmark", "score-speakers", reference_path, prediction_path)
        results.append({"variant": variant, "asr_model": model, "experimental_word_timestamps": args.word_timestamps, "report": report, "same_asr_segment_report": segment_report, "asr_metrics": asr["result"]["metrics"], "diarization_metrics": diarization["result"]["metrics"]})
        (args.root / "pipeline-report.json").write_text(json.dumps(results, indent=2))
        print(variant, model, "cpWER", report["score"]["cpwer"], "unattributed words", unknown_words, flush=True)
