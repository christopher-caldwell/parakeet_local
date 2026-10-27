# Focused human checkpoints

The existing recordings have already been imported and processed. No new dictation is needed for the next step.

## First checkpoint completed: three short clips

The user identified [A](../.local/enrollment-checkpoint/candidate-A.wav), [B](../.local/enrollment-checkpoint/candidate-B.wav) and [C](../.local/enrollment-checkpoint/candidate-C.wav), approximately 22.4 seconds total. All three exact intervals were enrolled through the application service. The three profile names, stable IDs, explicit confirmation and embedding provenance remain in the private library and `.local/enrollment-checkpoint/` receipts, outside Git.

Only the exact confirmed intervals became verified examples. The full containing speaker clusters were not labeled or enrolled. A microphone source is not assumed to be the user. Clip hashes were checked before enrollment; original track-local ranges are recorded in `.local/enrollment-checkpoint/candidates.json`.

Twelve separate, nonoverlapping candidate intervals were compared locally, producing inspectable suggestion receipts. All remained Unknown under the explicit diagnostic policy (score 0.9, margin 0.05, two verified references). Each identity currently has one confirmed example. None of these predictions became enrollment data, and these unlabeled comparisons do not measure accuracy. One short sample is insufficient to certify a personal identity threshold. Future independent confirmed examples can support tuning; separate labeled material must remain held out for evaluation.

The GPUI Speakers screen also supports playing candidates, choosing/creating a profile, confirming enrollment and removing an incorrect example. The CLI exposes the same services. Comparison review, confirmation/replacement, rejection and undo are now available in GPUI. Anonymous cluster edits currently use CLI.

## Later: one controlled physical session

After enrollment work, use one short session to approve microphone access, select the intended physical microphone, record both sources, and check playback/source alignment. Then try the two configured hold shortcuts on the laptop and external keyboards, cancellation and insertion into the intended editor. Screen/audio, input monitoring and Accessibility preflights are already available; actual microphone approval and physical key delivery remain unverified.

Finish with a small real-meeting review of chronology, overlap and false local attribution. UI convenience and the M3 laptop need actual use. No remote AI profile needs to be enabled for any of these checks; choosing such a route is a separate explicit decision.
