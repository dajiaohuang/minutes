#!/usr/bin/env python3
"""Descriptive oracle-word attribution check; never an ASR accuracy result.

Reference text/timing is shared by both engines. Ambiguous or missing predicted
activity abstains; its words remain in reference and count as attribution loss.
"""
import argparse
from collections import Counter
from dataclasses import asdict
import importlib.metadata
import json
import math
from pathlib import Path
import xml.etree.ElementTree as ET

from full_pipeline import read_rttm, verified_files
from run import digest
from metrics import words


def read_words(path, file_id, speaker, duration):
    result, excluded = [], Counter()
    for node in ET.parse(path).getroot().iter():
        if node.tag.rsplit('}', 1)[-1] != 'w':
            continue
        try:
            start, end = float(node.attrib['starttime']), float(node.attrib['endtime'])
        except (KeyError, ValueError):
            excluded['missing_or_invalid_time'] += 1
            continue
        if not (math.isfinite(start) and math.isfinite(end) and 0 <= start < end <= duration):
            excluded['nonpositive_nonfinite_or_out_of_file'] += 1
            continue
        tokens = words(''.join(node.itertext()))
        if not tokens:
            excluded['empty_after_normalization'] += 1
        for token in tokens:
            result.append(dict(session_id=file_id, speaker=speaker, start_time=start,
                               end_time=end, words=token))
    return result, dict(excluded)


def attribute(reference, segments, duration):
    for segment in segments:
        start, end, speaker = segment['start'], segment['end'], segment['speaker']
        if (not isinstance(speaker, str) or not speaker or not math.isfinite(start)
                or not math.isfinite(end) or not 0 <= start < end <= duration):
            raise ValueError('invalid predicted activity')
    hypothesis, abstained = [], Counter()
    for word in reference:
        active = {s['speaker'] for s in segments
                  if s['start'] < word['end_time'] and s['end'] > word['start_time']}
        if not active:
            abstained['no_activity'] += 1
        elif 'UNKNOWN' in active:
            abstained['unknown_activity'] += 1
        elif len(active) != 1:
            abstained['multiple_speakers'] += 1
        else:
            hypothesis.append(dict(word, speaker=next(iter(active))))
    return hypothesis, dict(abstained)


def score_words(reference, hypothesis, collar):
    if importlib.metadata.version('meeteval') != '0.4.3':
        raise ValueError('this diagnostic pins MeetEval 0.4.3')
    from meeteval.io import SegLST
    from meeteval.wer.wer.cp import cp_word_error_rate
    from meeteval.wer.wer.time_constrained import time_constrained_minimum_permutation_word_error_rate
    ref, hyp = SegLST(reference), SegLST(hypothesis)
    cp = cp_word_error_rate(ref, hyp)
    tcp = time_constrained_minimum_permutation_word_error_rate(
        ref, hyp, collar=collar, reference_pseudo_word_level_timing='full_segment',
        hypothesis_pseudo_word_level_timing='full_segment')
    return {name: dict(asdict(value), error_rate=float(value.error_rate))
            for name, value in [('cpwer', cp), ('tcpwer', tcp)]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('manifest', 'word-policy', 'annotations', 'predictions', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text())
    policy = json.loads(args.word_policy.read_text())
    if policy['task'] != 'oracle-word-attribution' or policy['meeteval_version'] != '0.4.3':
        raise ValueError('wrong word policy')
    if policy['tcpwer_collar_seconds'] != 0.5:
        raise ValueError('this frozen diagnostic uses a 0.5-second collar')
    verified_files(args.annotations, policy['files'])
    model_report = args.predictions / 'report.json'
    if json.loads(model_report.read_text())['manifest_sha256'] != digest(args.manifest):
        raise ValueError('predictions belong to a different model/input policy')
    rows, input_hashes = [], {}
    for row in manifest['recordings']:
        reference, exclusions = [], {}
        for speaker in 'ABCD':
            name = f"{row['id']}.{speaker}.words.xml"
            if name not in {f['name'] for f in policy['files']}:
                raise ValueError('missing pinned reference participant')
            data, excluded = read_words(args.annotations / name, row['id'], speaker, row['duration'])
            reference.extend(data)
            exclusions[speaker] = excluded
        reference.sort(key=lambda w: (w['start_time'], w['end_time'], w['speaker']))
        if not reference:
            raise ValueError('empty lexical reference')
        for engine in ('baseline', 'candidate'):
            path = args.predictions / f"{row['id']}-{engine}.stdout"
            input_hashes[path.name] = digest(path)
            text = path.read_text()
            segments = (json.loads(text) if engine == 'baseline' else
                        read_rttm(text, row['id'], row['duration']))['segments']
            hypothesis, abstained = attribute(reference, segments, row['duration'])
            rows.append(dict(id=row['id'], engine=engine, reference_words=len(reference),
                             attributed_words=len(hypothesis), abstentions=abstained,
                             excluded_xml_nodes=exclusions,
                             **score_words(reference, hypothesis, policy['tcpwer_collar_seconds'])))
    aggregate = {}
    for engine in ('baseline', 'candidate'):
        selected = [r for r in rows if r['engine'] == engine]
        aggregate[engine] = {}
        for metric in ('cpwer', 'tcpwer'):
            errors = sum(r[metric]['errors'] for r in selected)
            length = sum(r[metric]['length'] for r in selected)
            aggregate[engine][metric] = dict(errors=errors, length=length, error_rate=errors / length)
        aggregate[engine]['abstained_words'] = sum(sum(r['abstentions'].values()) for r in selected)
    report = dict(task=policy['task'], qualification=policy['qualification'],
                  policy_sha256=digest(args.word_policy), manifest_sha256=digest(args.manifest),
                  runner_sha256=digest(Path(__file__)), model_report_sha256=digest(model_report),
                  prediction_sha256=input_hashes, meeteval_version=importlib.metadata.version('meeteval'),
                  rows=rows, aggregates=aggregate)
    with args.output.open('x') as output:
        json.dump(report, output, indent=2, default=str)
        output.write('\n')
    print(json.dumps(aggregate, indent=2))


if __name__ == '__main__':
    main()
