#!/usr/bin/env python3
"""Prepare 24 additional LibriSpeech voices; no download, inference, or tuning.

Uses the official dev-clean archive (OpenSLR 12, CC BY 4.0). Selection is the
first 8–30 second utterance for each eligible voice in archive order, excluding
the six voices used by the earlier probes. Only regular, bounded members are
read; archive paths are never extracted.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tarfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--archive", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
with args.archive.open("rb") as source:
    digest = hashlib.file_digest(source, "md5").hexdigest()
if digest != "42e2234ba48799c1f50f24a7926300a1":
    raise SystemExit("Official dev-clean archive MD5 does not match")
args.output.mkdir(parents=True, exist_ok=True)
excluded = {"84", "174", "251", "422", "652", "777"}
chosen = {}
transcripts = {}
with tarfile.open(args.archive, mode="r|gz") as archive:
    for member in archive:
        if not member.isfile():
            continue
        name = Path(member.name).name
        if name.endswith(".trans.txt") and member.size < 1024 * 1024:
            for line in archive.extractfile(member).read().decode().splitlines():
                key, text = line.split(" ", 1)
                transcripts[key] = text
        elif len(chosen) < 24 and re.fullmatch(r"\d+-\d+-\d+\.flac", name) and member.size < 4 * 1024 * 1024:
            speaker = name.split("-")[0]
            if speaker in excluded or speaker in chosen:
                continue
            path = args.output / name
            data = archive.extractfile(member).read()
            if path.exists():
                if path.read_bytes() != data:
                    raise SystemExit(f"Existing fixture differs from archive: {path}")
            else:
                path.write_bytes(data)
            duration = float(subprocess.check_output([
                "ffprobe", "-v", "error", "-show_entries", "format=duration",
                "-of", "default=nw=1:nk=1", str(path),
            ], timeout=30))
            if 8 <= duration <= 30:
                chosen[speaker] = {"id": path.stem, "audio": str(path.resolve()), "reference": None}
            else:
                path.unlink()
if len(chosen) != 24:
    raise SystemExit("Could not find the expected 24 voices")
for entry in chosen.values():
    entry["reference_text"] = transcripts[entry["id"]]
suite = {
    "id": "librispeech-24-new-voices",
    "dataset": "LibriSpeech dev-clean, OpenSLR12 CC BY4; archive MD5 " + digest
        + "; first24 eligible voices in archive order excluding the six earlier voices; first8-30sec utterance per voice",
    "split": "new-voice-heldout-fixed-models", "labels": "verified_public",
    "models": ["whisper-base.en", "whisper-small.en"],
    "entries": list(chosen.values()), "collar_ms": 0, "score_overlap": True,
    "word_normalization": "english_case_punctuation",
}
(args.output / "suite.json").write_text(json.dumps(suite, indent=2))
print("Prepared 24 new public voices; no inference or model selection performed.")
