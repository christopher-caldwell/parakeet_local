# Focused human checkpoints

## 0. Pick the right transcript (about 10 minutes)

Measures which speech model is more accurate on your own meetings, without anyone typing a reference transcript. Two models transcribe the same recordings; where they disagree, a page plays a short clip and shows both versions in random order. You pick the one that matches what was said.

```sh
python3 benchmarks/make_review_page.py .local/review/result-tdt.json .local/review/result-lv3.json .local/review --items 60
open .local/review/review.html
```

Answer, press **Save my answers**, then `python3 benchmarks/score_review.py .local/review ~/Downloads/review-answers.json` prints how often each transcript was right (counts only). The page, clips and answers hold private speech and stay under `.local/`.

## 1. Speaker identification session (about 15 minutes)

No new recording is needed. In the desktop app: open a meeting that has been analyzed (Library → recording → **Name voices** tab; the Transcript tab shows a blue banner with a Name voices button while voices are waiting), then for each waiting voice:

1. Press **Play a clip**.
2. Type who it is (or tap a name you already gave) and press **Save name**, or press **Skip this voice**.

Saving names every line that voice speaks alone, leaves overlapping lines untouched, and keeps up to three clean turns as voice examples. **Undo** (listed under "Named in this recording") reverts the naming and removes exactly those examples. Repeat for the same person in other meetings so each person has examples from more than one recording and, ideally, both sources.

When two or more people have examples from at least three recordings, open **People** and press **Check how well my verified voices are recognized**. It tunes criteria on some recordings and scores them once on the rest. Read the numbers and, if they are acceptable, press **Use these criteria**, then turn on **Suggest names for voices** (Record a meeting → After you stop). Suggestions are always shown as unconfirmed.

Earlier checkpoint: three short clips (one example per person) were enrolled; their names and receipts stay in the private library and `.local/`, outside Git.

## 2. One controlled physical session

Approve microphone, Screen Recording, Accessibility and Input Monitoring from Privacy & AI (each prompts only when you press its button). Select the intended microphone, record both sources for a few minutes with someone speaking through the speakers, then check: playback and source alignment, that speech heard only through the speakers is not labelled Me, and that the silent-source and interruption warnings behave (mute the microphone once; unplug and replug it once).

Then try each configured hold shortcut on the laptop and on the external keyboard (including a modifier-only hold such as Right Option), cancellation, and insertion into the intended editor. Bluetooth/external microphone changes and the M3 laptop need real use.

Finish with a short real-meeting review of chronology, overlap and false local attribution. No remote AI profile needs to be enabled for any of these checks.
