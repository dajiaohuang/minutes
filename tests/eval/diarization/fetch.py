#!/usr/bin/env python3
"""Explicit download of the public, licensed fixtures/models pinned next to this file."""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import tempfile
import time
import urllib.request


def download_once(url, target, expected):
    if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest() == expected:
        return
    temporary = None
    try:
        with urllib.request.urlopen(url, timeout=120) as response, tempfile.NamedTemporaryFile(dir=target.parent, delete=False) as output:
            temporary = Path(output.name)
            digest, length = hashlib.sha256(), 0
            for block in iter(lambda: response.read(1024 * 1024), b''):
                length += len(block)
                if length > 100 * 1024 * 1024:
                    raise ValueError('public fixture download exceeded 100 MiB budget')
                digest.update(block)
                output.write(block)
        if digest.hexdigest() != expected:
            raise ValueError(f'{target.name}: downloaded hash differs from pinned manifest')
        os.replace(temporary, target)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def download(url, target, expected):
    for attempt in range(3):
        try:
            download_once(url, target, expected)
            return
        except (OSError, http.client.HTTPException):
            if attempt == 2:
                raise
            time.sleep(2 ** attempt)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    manifest = json.loads((root / 'ami-holdout.json').read_text())
    models = json.loads((root / 'models.json').read_text())
    audio_directory, model_directory = args.directory / 'audio', args.directory / 'models'
    audio_directory.mkdir(parents=True, exist_ok=True)
    model_directory.mkdir(parents=True, exist_ok=True)
    for item in manifest['recordings']:
        name = item['id']
        base = f'https://groups.inf.ed.ac.uk/ami/AMICorpusMirror/amicorpus/{name}/audio/'
        reference = 'https://raw.githubusercontent.com/pyannote/AMI-diarization-setup/67c2d539286e89f68952d5dcf83912bd9f01dfae/only_words/rttms/test/'
        download(base + item['audio'], audio_directory / item['audio'], item['sha256'])
        download(reference + item['reference'], audio_directory / item['reference'], item['reference_sha256'])
    for item in models['files']:
        download(item['url'], model_directory / item['name'], item['sha256'])
    (args.directory / 'ATTRIBUTION.txt').write_text(
        'AMI Meeting Corpus, AMI Project, https://groups.inf.ed.ac.uk/ami/, CC-BY-4.0.\n'
        'RTTM conversion: pyannote/AMI-diarization-setup commit 67c2d539286e89f68952d5dcf83912bd9f01dfae.\n'
        + models['attribution'] + '\n')
    print('Pinned public AMI audio, annotations and Community-1 models verified.')


if __name__ == '__main__':
    main()
