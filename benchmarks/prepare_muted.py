"""Synthetic stand-ins for a muted or idle microphone: tracks with no speech at all.

Usage: python3 benchmarks/prepare_muted.py OUTPUT_DIR   (needs numpy, soundfile)
Any word a model prints for these is a false word. Writes suite.json (empty references).
"""
import json, os, sys
import numpy as np, soundfile as sf

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
sr, seconds = 16000, 180
rng = np.random.default_rng(20261028)
n = sr * seconds
t = np.arange(n) / sr
db = lambda x: 10 ** (x / 20)

def lowpass(x, k):
    return np.convolve(x, np.ones(k) / k, mode="same")

def clicks(rate):
    x = np.zeros(n)
    for at in np.cumsum(rng.exponential(1 / rate, int(seconds * rate * 2))):
        i = int(at * sr)
        if i + 480 < n:
            x[i:i + 480] += rng.normal(0, 0.3, 480) * np.exp(-np.arange(480) / 60)
    return x

hum = sum(np.sin(2 * np.pi * 60 * h * t) / h for h in (1, 2, 3))
breath = lowpass(rng.normal(0, 1, n), 8) * (0.5 + 0.5 * np.sin(2 * np.pi * 0.25 * t)) ** 2
tracks = {
    "digital-silence": np.zeros(n),
    "self-noise-minus60": rng.normal(0, db(-60), n),
    "room-noise-minus45": lowpass(rng.normal(0, 1, n), 4) * db(-45) * 3,
    "keyboard-clicks": clicks(3) * db(-14) + rng.normal(0, db(-60), n),
    "mains-hum-minus40": hum * db(-40) + rng.normal(0, db(-60), n),
    "breathing-minus35": breath * db(-35) + rng.normal(0, db(-60), n),
    "fan-noise-minus35": lowpass(rng.normal(0, 1, n), 12) * db(-35) * 4,
}
entries = []
for name, x in tracks.items():
    path = os.path.abspath(f"{out}/{name}.wav")
    sf.write(path, np.clip(x, -1, 1).astype(np.float32), sr, subtype="PCM_16")
    entries.append({"id": name, "audio": path, "reference": None, "reference_text": ""})
json.dump({"id": "muted-mic-probe", "dataset": "Generated non-speech tracks, 3 min, mono 16 kHz, seed 20261028",
           "split": "diagnostic", "labels": "synthetic", "models": ["whisper-small.en"], "collar_ms": 0,
           "score_overlap": True, "word_normalization": "english_case_punctuation", "entries": entries},
          open(f"{out}/suite.json", "w"), indent=1)
