#!/usr/bin/env python3
"""Explicitly fetch only the public, hashed full-pipeline experiment inputs."""
import argparse
import json
from pathlib import Path
from urllib.parse import urlsplit

from fetch import download
from run import digest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--directory', type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text())
    if manifest.get('task') != 'full-native-diarization' or manifest.get('schema_version') != 1:
        raise ValueError('expected frozen full-native-diarization manifest')
    groups = [('audio', item) for row in manifest['recordings'] for item in row['files']]
    groups += [(label + '-models', item) for label, model in manifest['models'].items()
               for item in model['files']]
    admitted = []
    for group, item in groups:
        url = urlsplit(item['url'])
        if url.scheme != 'https' or url.hostname not in {
                'groups.inf.ed.ac.uk', 'raw.githubusercontent.com', 'huggingface.co'}:
            raise ValueError('only the pinned public dataset/model hosts are admitted')
        target = args.directory / group / item['name']
        if not target.resolve().is_relative_to(args.directory.resolve()):
            raise ValueError('fixture path escapes owned directory')
        if target.exists() and digest(target) != item['sha256']:
            raise ValueError('existing file differs; preserve it and select a fresh directory')
        admitted.append((item, target))
    for item, target in admitted:
        target.parent.mkdir(parents=True, exist_ok=True)
        download(item['url'], target, item['sha256'])
    (args.directory / 'ATTRIBUTION.txt').write_text(
        'AMI Project, AMI Meeting Corpus, https://groups.inf.ed.ac.uk/ami/, CC-BY-4.0.\n'
        'RTTM conversion pyannote/AMI-diarization-setup67c2d539.\n'
        + manifest['models']['baseline']['attribution'] + '\n'
        + manifest['models']['candidate']['provenance'] + '\n')
    print('Pinned public inputs verified. Inference and model selection were not run.')


if __name__ == '__main__':main()
