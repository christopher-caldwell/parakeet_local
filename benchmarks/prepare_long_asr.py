"""Concatenate consecutive LibriSpeech dev-clean utterances into multi-minute clips.

Long clips exercise chunking, which single utterances never do. Usage:
  python3 benchmarks/prepare_long_asr.py ARCHIVE OUTPUT_DIR SPEAKER [SPEAKER ...]
Writes OUTPUT_DIR/suite.json for `parakeet benchmark run`. Audio stays under ignored paths.
"""
import json, os, random, subprocess, sys, tarfile

archive, out, *speakers = sys.argv[1:]
os.makedirs(out, exist_ok=True)
want, text, audio = set(speakers), {}, {}
with tarfile.open(archive) as tar:
    for member in tar:
        parts = member.name.split("/")
        if len(parts) < 4 or parts[2] not in want:
            continue
        if member.name.endswith(".trans.txt"):
            for line in tar.extractfile(member).read().decode().splitlines():
                key, value = line.split(" ", 1)
                text[key] = value
        elif member.name.endswith(".flac"):
            audio[parts[-1][:-5]] = tar.extractfile(member).read()
rng, entries = random.Random(7), []
for speaker in sorted(want):
    ids = sorted(k for k in audio if k.startswith(speaker + "-"))[:40]
    files = []
    for key in ids:
        path = f"{out}/{key}.flac"
        open(path, "wb").write(audio[key])
        files.append(path)
    command = ["ffmpeg", "-y", "-loglevel", "error"]
    graph = labels = ""
    for index, path in enumerate(files):
        command += ["-i", path]
        gap = rng.choice([0.3, 0.6, 1.0, 1.5])
        graph += f"[{index}:a]aresample=16000,apad=pad_dur={gap}[a{index}];"
        labels += f"[a{index}]"
    graph += f"{labels}concat=n={len(files)}:v=0:a=1[o]"
    wav = f"{out}/long-{speaker}.wav"
    subprocess.run(command + ["-filter_complex", graph, "-map", "[o]", "-ac", "1", wav], check=True)
    for path in files:
        os.remove(path)
    entries.append({"id": f"long-{speaker}", "audio": os.path.abspath(wav), "reference": None,
                    "reference_text": " ".join(text[k] for k in ids)})
json.dump({"id": "librispeech-long-concat",
           "dataset": "LibriSpeech dev-clean (OpenSLR 12, CC BY 4.0), 40 consecutive utterances per speaker joined with 0.3-1.5 s gaps",
           "split": "development", "labels": "verified_public", "models": ["whisper-small.en"],
           "collar_ms": 0, "score_overlap": True, "word_normalization": "english_case_punctuation",
           "entries": entries}, open(f"{out}/suite.json", "w"), indent=1)
