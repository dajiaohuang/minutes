#!/usr/bin/env python3
"""Explicitly download and extract only the twelve pinned public word XMLs."""
import argparse
import hashlib
import json
from pathlib import Path
from urllib.parse import urlsplit
import zipfile

from fetch import download


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--policy', type=Path, required=True)
    parser.add_argument('--directory', type=Path, required=True)
    args = parser.parse_args()
    policy = json.loads(args.policy.read_text())
    url = urlsplit(policy['source'])
    if (policy['task'] != 'oracle-word-attribution' or url.scheme != 'https'
            or url.hostname != 'groups.inf.ed.ac.uk' or policy['license'] != 'CC-BY-4.0'):
        raise ValueError('expected pinned public AMI annotations')
    admitted = []
    for item in policy['files']:
        name = item['name']
        if Path(name).name != name or not name.endswith('.words.xml'):
            raise ValueError('plain word XML filename required')
        target = args.directory / 'word-annotations' / name
        if not target.resolve().is_relative_to(args.directory.resolve()):
            raise ValueError('output escapes owned directory')
        if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest() != item['sha256']:
            raise ValueError('existing annotation differs; preserve and select fresh directory')
        admitted.append((item, target))
    args.directory.mkdir(parents=True, exist_ok=True)
    archive = args.directory / 'ami_public_manual_1.6.2.zip'
    if archive.exists() and hashlib.sha256(archive.read_bytes()).hexdigest() != policy['sha256']:
        raise ValueError('existing archive differs; preserve and select fresh directory')
    download(policy['source'], archive, policy['sha256'])
    with zipfile.ZipFile(archive) as zipped:
        for item, target in admitted:
            info = zipped.getinfo('words/' + item['name'])
            if info.file_size > 4 * 1024**2:
                raise ValueError('XML exceeds 4 MiB budget')
            data = zipped.read(info)
            if hashlib.sha256(data).hexdigest() != item['sha256']:
                raise ValueError('XML differs from frozen hash')
            target.parent.mkdir(parents=True, exist_ok=True)
            if not target.exists():
                with target.open('xb') as output:
                    output.write(data)
    (args.directory / 'WORD_ATTRIBUTION.txt').write_text(policy['attribution'] + '\n' + policy['license'] + '\n' + policy['source'] + '\n')
    print('Only pinned public word annotations verified; no inference or scoring run.')


if __name__ == '__main__':
    main()
