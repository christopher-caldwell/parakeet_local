# Focused human checkpoints

The existing recordings have already been imported and processed. No new dictation is needed for the next step.

## First: identify three short clips

Listen to [A](../.local/enrollment-checkpoint/candidate-A.wav), [B](../.local/enrollment-checkpoint/candidate-B.wav) and [C](../.local/enrollment-checkpoint/candidate-C.wav). Together they contain approximately 22.4 seconds. Reply with a name for each recognizable voice, or **Unknown**. If a clip contains multiple people or is unclear, leave it Unknown.

Only the exact confirmed intervals will become verified local examples. Reusing a name assigns examples to one stable profile; Unknown saves no example. A microphone source is not assumed to be the user. Clip hashes and original track-local ranges are recorded in `.local/enrollment-checkpoint/candidates.json`; no names or embeddings are committed to Git.

After receiving labels, the agent can enroll those intervals, propose additional independent examples and inspect cross-recording matches. One short sample is insufficient to certify an identity threshold. Unverified matching predictions remain predictions, and tuning material must stay separate from final evaluation.

The GPUI Speakers screen also supports playing candidates, choosing/creating a profile, confirming enrollment and removing an incorrect example. The CLI exposes the same services. Detailed suggestion/cluster edits currently use CLI.

## Later: one controlled physical session

After enrollment work, use one short session to approve microphone access, select the intended physical microphone, record both sources, and check playback/source alignment. Then try the two configured hold shortcuts on the laptop and external keyboards, cancellation and insertion into the intended editor. Screen/audio, input monitoring and Accessibility preflights are already available; actual microphone approval and physical key delivery remain unverified.

Finish with a small real-meeting review of chronology, overlap and false local attribution. UI convenience and the M3 laptop need actual use. No remote AI profile needs to be enabled for any of these checks; choosing such a route is a separate explicit decision.
