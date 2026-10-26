#!/usr/bin/env python3
"""Explicit offline comparison of the Minutes probe and a pinned speakrs example.

POSIX only. No capture, enrollment, uploads, model downloads or default changes.
The input manifest and model hashes must be frozen before collecting predictions.
"""
import argparse
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time
import wave

if os.name == 'posix':
    import resource

from run import digest, reference_segments, score

FILE_LIMIT = 16 * 1024**2


def read_rttm(text, file_id, duration):
    segments = []
    for line in text.splitlines():
        if not line.strip() or line.startswith('#'):
            continue
        fields = line.split()
        if len(fields) != 10 or fields[0] != 'SPEAKER' or fields[1] != file_id:
            raise ValueError('invalid RTTM record or mismatched recording ID')
        start, length = float(fields[3]), float(fields[4])
        end = start + length
        if (not all(math.isfinite(x) for x in (start, length, end))
                or start < 0 or length <= 0 or end > duration + 0.05):
            raise ValueError('invalid or out-of-file RTTM interval')
        segments.append({'start': start, 'end': min(end, duration), 'speaker': fields[7]})
    return {'segments': segments}


def group_rss_bytes(group):
    # Metadata only; report just this owned process group, not other host tasks.
    output = subprocess.check_output(['ps', '-axo', 'pid=,pgid=,rss='], text=True)
    return sum(int(row[2]) * 1024 for line in output.splitlines()
               if len(row := line.split()) == 3 and int(row[1]) == group)


def kill_group(group):
    try:
        os.killpg(group, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except PermissionError:
        # Darwin may report EPERM after the root was reaped while only dead
        # descendants remain. Never suppress a denied signal to a live member.
        output = subprocess.check_output(['ps', '-axo', 'pgid=,stat='], text=True)
        if any(int(row[0]) == group and not row[1].startswith('Z')
               for line in output.splitlines() if len(row := line.split()) == 2):
            raise


def supervise(command, output, wall_seconds=900, rss_bytes=4 * 1024**3,
              cancel_after=None, poll_seconds=0.2):
    """Kernel file-size limit, sampled group RSS guard, deadline and group kill.

    RSS sampling is a watchdog, not a kernel memory reservation. wait4 reports
    the child's actual peak separately; shared pages can inflate group RSS.
    """
    if os.name != 'posix' or not hasattr(os, 'wait4'):
        raise ValueError('this experiment requires POSIX process-group supervision')
    if wall_seconds <= 0 or rss_bytes <= 0:
        raise ValueError('positive resource budgets required')
    env = {k: os.environ[k] for k in ('PATH', 'TMPDIR', 'LANG', 'LC_ALL', 'SYSTEMROOT')
           if k in os.environ}
    env.update(RAYON_NUM_THREADS='2', OMP_NUM_THREADS='2',
               OPENBLAS_NUM_THREADS='2', VECLIB_MAXIMUM_THREADS='2',
               MINUTES_MCP_AUTO_SETUP='0', RUST_LOG='warn')
    started = time.monotonic()
    reason, sampled_peak, usage = None, 0, None
    with output.with_suffix('.stdout').open('xb') as stdout, output.with_suffix('.stderr').open('xb') as stderr:
        child = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '_worker',
                                  json.dumps(command)], stdin=subprocess.DEVNULL,
                                 stdout=stdout, stderr=stderr, env=env, start_new_session=True)
        try:
            try:
                os.setpriority(os.PRIO_PROCESS, child.pid, 10)
            except (AttributeError, ProcessLookupError, PermissionError):
                pass
            while True:
                pid, status, usage = os.wait4(child.pid, os.WNOHANG)
                if pid:
                    child.returncode = os.waitstatus_to_exitcode(status)
                    break
                elapsed = time.monotonic() - started
                rss = group_rss_bytes(child.pid)
                sampled_peak = max(sampled_peak, rss)
                if cancel_after is not None and elapsed >= cancel_after:
                    reason = 'cancelled'
                elif elapsed >= wall_seconds:
                    reason = 'wall_deadline'
                elif rss > rss_bytes:
                    reason = 'sampled_group_rss_budget'
                if reason:
                    kill_group(child.pid)
                    _, status, usage = os.wait4(child.pid, 0)
                    child.returncode = os.waitstatus_to_exitcode(status)
                    break
                time.sleep(poll_seconds)
        finally:
            # Also terminate descendants left behind after a successful root exit.
            kill_group(child.pid)
            if child.returncode is None:
                child.wait()
    kernel_peak = usage.ru_maxrss * (1 if sys.platform == 'darwin' else 1024)
    return {'exit': child.returncode, 'reason': reason,
            'processing_seconds': time.monotonic() - started,
            'sampled_group_peak_rss_bytes': sampled_peak,
            'kernel_child_peak_rss_bytes': kernel_peak,
            'rss_enforcement': 'sampled watchdog; not a kernel memory limit'}


def verified_files(directory, files):
    for item in files:
        path = (directory / item['name']).resolve()
        if not path.is_relative_to(directory.resolve()) or digest(path) != item['sha256']:
            raise ValueError('file path/hash differs from frozen manifest')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('manifest', 'audio-directory', 'baseline', 'candidate',
                 'baseline-models', 'candidate-models', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text())
    if manifest.get('schema_version') != 1 or manifest.get('task') != 'full-native-diarization':
        raise ValueError('expected frozen full-native-diarization manifest')
    if not manifest['recordings'] or len({r['id'] for r in manifest['recordings']}) != len(manifest['recordings']):
        raise ValueError('nonempty unique recording IDs required')
    expected_gates = {'minimum_der_improvement', 'maximum_recording_regression',
                      'maximum_speaker_count_error', 'maximum_real_time_factor',
                      'maximum_peak_rss_bytes'}
    if (set(manifest['gates']) != expected_gates or any(isinstance(v, bool)
            or not isinstance(v, (int, float)) or not math.isfinite(v) or v < 0
            for v in manifest['gates'].values())):
        raise ValueError('invalid predeclared gates')
    validated = []
    for row in manifest['recordings']:
        if not re.fullmatch(r'[A-Za-z0-9_-]+', row['id']) or len(row['files']) != 2:
            raise ValueError('safe recording ID and paired WAV/RTTM required')
        if row['authorization']['basis'] != 'license' or not row['authorization']['reference']:
            raise ValueError('explicit public recording license required')
        verified_files(args.audio_directory, row['files'])
        audio = args.audio_directory / row['files'][0]['name']
        with wave.open(str(audio)) as wav:
            duration = wav.getnframes() / wav.getframerate()
            if (wav.getframerate(), wav.getnchannels(), wav.getsampwidth()) != (16000, 1, 2):
                raise ValueError('admit only mono 16kHz signed16 WAV')
        if not 0 < duration <= 3600 or abs(duration - row['duration']) > 1e-6:
            raise ValueError('duration differs from manifest or one-hour experiment budget')
        reference = reference_segments(args.audio_directory / row['files'][1]['name'], duration)
        validated.append((row, audio, reference, duration))
    for label, directory in [('baseline', args.baseline_models), ('candidate', args.candidate_models)]:
        verified_files(directory, manifest['models'][label]['files'])
    executable_hashes = {label: digest(getattr(args, label)) for label in ('baseline', 'candidate')}
    if executable_hashes['baseline'] == executable_hashes['candidate']:
        raise ValueError('distinct engine executables required')
    args.output.mkdir(parents=True, exist_ok=False)
    rows = []
    for fixture, audio, reference, duration in validated:
        for label in ('baseline', 'candidate'):
            engine = getattr(args, label).resolve()
            command = ([str(engine), 'community-1', str(args.baseline_models.resolve()), str(audio.resolve())]
                       if label == 'baseline' else
                       [str(engine), str(args.candidate_models.resolve()), str(audio.resolve()), fixture['id']])
            path = args.output / (fixture['id'] + '-' + label)
            receipt = supervise(command, path)
            if receipt['exit'] or receipt['reason']:
                (args.output / 'failure.json').write_text(json.dumps({'id': fixture['id'], 'engine': label, **receipt}, indent=2))
                raise RuntimeError('worker failed; local bounded receipt retained')
            if digest(engine) != executable_hashes[label] or digest(audio) != fixture['files'][0]['sha256']:
                raise ValueError('engine or admitted audio changed during inference')
            text = path.with_suffix('.stdout').read_text()
            hypothesis = json.loads(text) if label == 'baseline' else read_rttm(text, fixture['id'], duration)
            measured = score(reference, hypothesis, duration)
            row = {'id': fixture['id'], 'engine': label, 'audio_seconds': duration,
                   **receipt, 'real_time_factor': receipt['processing_seconds'] / duration,
                   **measured}
            row['speaker_count_error'] = abs(row['predicted_speakers'] - row['reference_speakers'])
            rows.append(row)
            print(json.dumps(row), flush=True)
    aggregates = {}
    for label in ('baseline', 'candidate'):
        group = [r for r in rows if r['engine'] == label]
        denominator = sum(r['reference_speaker_seconds'] for r in group)
        aggregates[label] = {'der': sum(r[k] for r in group for k in
            ('missed_speaker_seconds', 'false_alarm_speaker_seconds', 'confusion_speaker_seconds')) / denominator}
    pairs = {r['id']: {} for r in rows}
    for row in rows:
        pairs[row['id']][row['engine']] = row
    gates = manifest['gates']
    improvement = aggregates['baseline']['der'] - aggregates['candidate']['der']
    checks = {
        'weighted_der_gain': improvement >= gates['minimum_der_improvement'],
        'no_large_recording_regression': all(p['candidate']['der'] <= p['baseline']['der'] + gates['maximum_recording_regression'] for p in pairs.values()),
        'speaker_count_within_one': all(p['candidate']['speaker_count_error'] <= gates['maximum_speaker_count_error'] for p in pairs.values()),
        'speaker_count_no_regression': all(p['candidate']['speaker_count_error'] <= p['baseline']['speaker_count_error'] for p in pairs.values()),
        'runtime': all(p['candidate']['real_time_factor'] <= gates['maximum_real_time_factor'] for p in pairs.values()),
        'memory': all(p['candidate']['kernel_child_peak_rss_bytes'] <= gates['maximum_peak_rss_bytes'] for p in pairs.values()),
    }
    report = {'schema_version': 1, 'manifest_sha256': digest(args.manifest),
              'runner_sha256': digest(Path(__file__)), 'platform': sys.platform,
              'executable_sha256': executable_hashes, 'sources': manifest['sources'],
              'provenance': manifest['provenance'], 'rows': rows, 'aggregates': aggregates,
              'gate': {'passed': all(checks.values()), 'checks': checks,
                       'der_improvement': improvement, 'thresholds': gates},
              'qualification': 'Specified public files and CPU settings only. No default-model, named-identity, word-error, live-latency, UI or capture qualification.'}
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report['gate']), flush=True)
    return 0 if report['gate']['passed'] else 2


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '_worker':
        if os.name != 'posix':
            raise SystemExit('POSIX-only experimental worker')
        resource.setrlimit(resource.RLIMIT_FSIZE, (FILE_LIMIT, FILE_LIMIT))
        command = json.loads(sys.argv[2])
        os.execve(command[0], command, os.environ)
    raise SystemExit(main())
