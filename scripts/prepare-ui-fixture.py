#!/usr/bin/env python3
"""Create synthetic transcript artifacts for import through the normal application.

No audio, identities or private transcript text is included. No database writes.
"""
import argparse
import hashlib
import json
from pathlib import Path

WORDS = ('audio source timeline segment speaker unknown review confirmed recording local '
         'model words transcript correction evidence overlap system microphone sample '
         'window keyboard navigation search playback import export title meeting '.split())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--segments', type=int, default=2000)
    args = parser.parse_args()
    if not 40 <= args.segments <= 20000:
        parser.error('segments must be between 40 and 20000')
    args.directory.mkdir(parents=True, mode=0o700, exist_ok=False)
    manifest = {'meeting': {'id': f'parakeet-ui-performance-v1-{args.segments}',
                 'title': 'UI performance fixture', 'createdAt': '2026-01-01T00:00:00Z'}}
    segments = []
    for index in range(args.segments):
        words = [WORDS[(index * 7 + word * 3) % len(WORDS)] for word in range(80 + index % 81)]
        text = f'Synthetic interval {index + 1}. ' + ' '.join(words) + '.'
        if index % 4 == 0:
            text += '\nA separate paragraph preserves a variable row height. No person is identified.'
        segments.append({'id': str(index), 'startMs': index*5000, 'endMs': index*5000+4500, 'text': text})
    payload = json.dumps({'transcriptSegments': segments}, ensure_ascii=False).encode()
    (args.directory/'manifest.json').write_text(json.dumps(manifest))
    (args.directory/'transcript.json').write_bytes(payload)
    print(json.dumps({'segments':len(segments),'transcript_sha256':hashlib.sha256(payload).hexdigest()}))


if __name__ == '__main__':
    main()
