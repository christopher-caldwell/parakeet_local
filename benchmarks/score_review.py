"""Tally review-answers.json from make_review_page.py: how often each transcript was right.

Usage: python3 benchmarks/score_review.py OUTPUT_DIR ANSWERS.json
Prints counts only (never any transcript text).
"""
import json, sys
out, answers = sys.argv[1:3]
key = json.load(open(f"{out}/key.json"))["flips"]
ans = json.load(open(answers))
count = {"a": 0, "b": 0, "both": 0, "neither": 0, "skip": 0}
for item, a in ans.items():
    c = a["choice"]
    if c in ("1", "2"):
        shown_b = (c == "1") == key[item]  # version 1 is B exactly when the card was flipped
        count["b" if shown_b else "a"] += 1
    else:
        count[c] += 1
decided = count["a"] + count["b"]
print(json.dumps({"answered": len(ans), **count,
                  "first_transcript_right_share_of_decided": round(count["a"] / decided, 3) if decided else None}))
