#!/usr/bin/env python3
"""Compare the Rust cpWER CLI against explicitly installed MeetEval 0.4.3.

Use an isolated Python environment. No installation, downloads or real speech.
"""
import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import random
import subprocess
import tempfile

import meeteval

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--cli", type=Path, default=Path("target/debug/parakeet"))
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
version = importlib.metadata.version("meeteval")
if version != "0.4.3":
    raise SystemExit("This oracle receipt is pinned to meeteval 0.4.3")
randomizer = random.Random(20261027)
cases = []
for _ in range(250):
    case = []
    for prefix in ["reference", "prediction"]:
        case.append([
            {"speaker": f"{prefix}-{i}", "text": " ".join(randomizer.choices(["one", "two", "three", "four", "five"], k=randomizer.randrange(15)))}
            for i in range(randomizer.randrange(6))
        ])
    cases.append(case)
with tempfile.TemporaryDirectory(prefix="parakeet-cpwer-") as directory:
    root = Path(directory)
    for number, (reference, prediction) in enumerate(cases):
        for name, streams in [("reference", reference), ("prediction", prediction)]:
            (root / f"{name}.json").write_text(json.dumps({
                "dataset": "synthetic random oracle", "labels": "synthetic" if name == "reference" else "model_prediction",
                "audio_sha256": "ab" * 32, "streams": streams,
            }))
        completed = subprocess.run([
            str(args.cli.resolve()), "--data-root", str(root / "must-remain-absent"),
            "benchmark", "score-speakers", str(root / "reference.json"), str(root / "prediction.json"), "--normalization", "exact",
        ], capture_output=True, text=True, check=True)
        actual = json.loads(completed.stdout)["data"]["score"]
        expected = meeteval.wer.cp_word_error_rate([s["text"] for s in reference], [s["text"] for s in prediction])
        if actual["errors"] != expected.errors or actual["reference_words"] != expected.length:
            raise AssertionError((number, actual, expected))
        assert not (root / "must-remain-absent").exists()
report = {
    "oracle": "meeteval", "version": version, "seed": 20261027, "cases": len(cases),
    "case_sha256": hashlib.sha256(json.dumps(cases, sort_keys=True).encode()).hexdigest(),
    "all_error_counts_and_denominators_match": True,
    "includes_empty_missing_extra_permuted_and_contaminated_streams": True,
    "application_storage_not_initialized": True,
}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report))
