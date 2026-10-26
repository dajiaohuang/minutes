#!/usr/bin/env python3
"""Offline, model-only paired diarization evaluation. No capture, ASR or uploads.

Run executables built from the same examples/diarize_file.rs harness at two
revisions. Public fixture downloads are explicit; this runner never downloads.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys
import time
import wave

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'v02'))
from metrics import diarization_error


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def reference_segments(path, duration):
    segments = []
    for line in path.read_text().splitlines():
        if not line.strip() or line.startswith('#'):
            continue
        fields = line.split()
        if len(fields) != 10 or fields[0] != 'SPEAKER':
            raise ValueError('expected ten-column SPEAKER RTTM')
        start, length = float(fields[3]), float(fields[4])
        end = min(duration, start + length)
        if not all(math.isfinite(v) for v in (start, length)) or start < 0 or length <= 0:
            raise ValueError('invalid RTTM interval')
        if start < duration:
            segments.append({'start': start, 'end': end, 'speaker': fields[7]})
    if not segments:
        raise ValueError('reference must contain speaker intervals')
    return segments


def score(reference, hypothesis, duration):
    segments = hypothesis['segments']
    # Validate *all* intervals before filtering. UNKNOWN is abstention, never a
    # free label the global mapper may turn into a known reference identity.
    diarization_error([], segments, duration)
    known = [s for s in segments if s['speaker'] != 'UNKNOWN']
    unknown = [s for s in segments if s['speaker'] == 'UNKNOWN']
    return {**diarization_error(reference, known, duration),
            'predicted_speakers': len({s['speaker'] for s in known}),
            'reference_speakers': len({s['speaker'] for s in reference}),
            'unknown_speaker_seconds': sum(s['end'] - s['start'] for s in unknown)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--audio-directory', type=Path, required=True)
    parser.add_argument('--models', type=Path, required=True)
    parser.add_argument('--model', choices=['legacy', 'community-1'], default='community-1')
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--baseline-revision', required=True)
    parser.add_argument('--candidate-revision', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text())
    if manifest.get('schema_version') != 1 or manifest.get('task') != 'diarization':
        raise ValueError('expected diarization manifest version 1')
    fixtures = manifest['recordings']
    if not fixtures or len({r['id'] for r in fixtures}) != len(fixtures):
        raise ValueError('nonempty, unique fixture IDs required')
    gates = manifest['gates']
    required = {'minimum_der_improvement', 'maximum_recording_regression', 'maximum_real_time_factor'}
    if set(gates) != required or any(isinstance(v, bool) or not isinstance(v, (int, float))
            or not math.isfinite(v) or v < 0 for v in gates.values()):
        raise ValueError('invalid predeclared gates')
    validated = []
    # Verify every fixture and its authorization before executing either binary.
    for fixture in fixtures:
        authorization = fixture.get('authorization', {})
        if authorization.get('basis') not in ('license', 'consent') or not authorization.get('reference'):
            raise ValueError('fixture requires a license or consent reference')
        if fixture.get('split') != 'holdout':
            raise ValueError('this admission runner accepts holdout fixtures only')
        audio = args.audio_directory / fixture['audio']
        reference = args.audio_directory / fixture['reference']
        if digest(audio) != fixture['sha256'] or digest(reference) != fixture['reference_sha256']:
            raise ValueError(f"{fixture['id']}: fixture hash mismatch")
        with wave.open(str(audio)) as recording:
            duration = recording.getnframes() / recording.getframerate()
        if not 0 < duration <= 7200 or abs(duration - fixture['duration']) > 1e-6:
            raise ValueError('fixture duration does not match manifest/budget')
        validated.append((fixture, audio, reference_segments(reference, duration), duration))
    args.output.mkdir(parents=True, exist_ok=True)
    model_files = (['segmentation-community-1.onnx', 'embedding-community-1.onnx']
                   if args.model == 'community-1' else ['segmentation-3.0.onnx', 'wespeaker_en_voxceleb_CAM++.onnx'])
    model_hashes = {name: digest(args.models / name) for name in model_files}
    executables = {'baseline': args.baseline.resolve(), 'candidate': args.candidate.resolve()}
    executable_hashes = {key: digest(value) for key, value in executables.items()}
    if executable_hashes['baseline'] == executable_hashes['candidate']:
        raise ValueError('baseline and candidate executables are identical; no comparison was run')
    rows = []
    for fixture, audio, reference, duration in validated:
        for label, executable in executables.items():
            start = time.monotonic()
            # The subprocess receives only the admitted file and public model
            # directory. Its stderr remains local, outside the report.
            with (args.output / f"{fixture['id']}-{label}.json").open('w') as output, (args.output / f"{fixture['id']}-{label}.stderr").open('w') as errors:
                subprocess.run([str(executable), args.model, str(args.models.resolve()), str(audio.resolve())],
                               check=True, timeout=900, stdin=subprocess.DEVNULL, stdout=output, stderr=errors)
            elapsed = time.monotonic() - start
            if digest(executable) != executable_hashes[label]:
                raise ValueError('executable changed during evaluation')
            hypothesis = json.loads((args.output / f"{fixture['id']}-{label}.json").read_text())
            rows.append({'id': fixture['id'], 'candidate': label, 'audio_seconds': duration,
                         'processing_seconds': elapsed, 'real_time_factor': elapsed / duration,
                         **score(reference, hypothesis, duration)})
            print(f"{fixture['id']} {label}: DER={rows[-1]['der']:.2%}, RTF={elapsed / duration:.3f}", flush=True)
    aggregates = {}
    for label in executables:
        selected = [r for r in rows if r['candidate'] == label]
        speaker_time = sum(r['reference_speaker_seconds'] for r in selected)
        errors = sum(r[k] for r in selected for k in ('missed_speaker_seconds', 'false_alarm_speaker_seconds', 'confusion_speaker_seconds'))
        aggregates[label] = {'der': errors / speaker_time, 'reference_speaker_seconds': speaker_time}
    pairs = {r['id']: {} for r in rows}
    for row in rows:
        pairs[row['id']][row['candidate']] = row
    improvement = aggregates['baseline']['der'] - aggregates['candidate']['der']
    passed = (improvement >= gates['minimum_der_improvement']
              and all(p['candidate']['der'] <= p['baseline']['der'] + gates['maximum_recording_regression'] for p in pairs.values())
              and all(p['candidate']['real_time_factor'] <= gates['maximum_real_time_factor'] for p in pairs.values()))
    report = {'schema_version': 1, 'manifest_sha256': digest(args.manifest), 'model': args.model,
              'baseline_revision': args.baseline_revision, 'candidate_revision': args.candidate_revision,
              'executable_sha256': executable_hashes, 'model_sha256': model_hashes,
              'platform': sys.platform, 'provenance': manifest.get('provenance', {}),
              'timing_source': 'harness-wall-clock', 'rows': rows, 'aggregates': aggregates,
              'gate': {'passed': passed, 'thresholds': gates, 'der_improvement': improvement},
              'qualification': 'Specified files only. Not a model-default promotion, native UI/capture qualification, named-voice test, or general SOTA claim.'}
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report['gate']))
    return 0 if passed else 2


if __name__ == '__main__':
    raise SystemExit(main())
