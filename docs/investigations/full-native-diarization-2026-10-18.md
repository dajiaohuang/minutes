# Full native speaker pipeline comparison

The full Speakrs CPU pipeline improved anonymous speaker attribution against
Minutes 0.30.4's optional Community-1 path on all three selected AMI recordings.
This supports building an opt-in, isolated refinement integration. It does not
support changing the default or claiming general state-of-the-art accuracy.

## Source and protocol

- Baseline: released Minutes 0.30.4,
  [`cfaccda1`](https://github.com/silverstein/minutes/commit/cfaccda1f5175bcfb4d78d28a99160e6b336ba1f),
  `diarize_file community-1`, built `--release --locked --no-default-features --features diarize`.
  This is the opt-in Community-1 comparator, not the legacy default engine.
- Candidate: unmodified Speakrs `diarize_wav` at
  [`af8013f`](https://github.com/avencera/speakrs/tree/af8013f95a53a37a554d8abc6e4ae2ab9a614be6),
  package 0.7.0, built `--release --locked --no-default-features --features cpu`.
  Uses the full reconstruction/PLDA/VBx pipeline with default settings and
  one-second step; no oracle speaker count or tuning. The online/download feature
  is disabled. All 579 source blobs were checked against the pinned Git tree.
- Rust 1.95.0, native Apple Silicon Mac with 128 GiB RAM, 16 logical CPUs. Both
  optimized probes ran on the same machine. Thread settings are hints, not an
  equal-work microbenchmark: the baseline ONNX sessions use one intra/inter thread.
- Public licensed AMI `Array1-01` audio and pinned `only_words` RTTMs: IS1008a,
  TS3004a and IB4001, 67.835 minutes in three AMI development groups. Two scenario
  meetings, one unstructured. No private recordings, microphone or playback.
- Manifest, model hashes, thresholds and runner frozen before predictions in
  `13b790ca`. Candidate weights pinned to HF revision
  `67f963b6a1d94fcd4678948237b3cc8bd69bf1d8`. Code is Apache-2.0; converted
  weights refer to upstream terms and lack a separate converter license field.
  This evaluation does not authorize model redistribution or profile reuse.
- These files were fresh to this comparison. PR71's historical numbered AMI
  examples cannot be matched to filenames, so prior project overlap cannot be
  excluded. No model-training independence or blinded selection claim.

## Anonymous speaker time

Zero-collar DER, overlap included, globally optimal speaker mapping. `UNKNOWN`
is excluded from identity mapping. Lower error is better.

| Recording | Duration | 0.30.4 DER | Full pipeline DER | Predicted speakers, baseline → candidate / reference | Processing seconds, baseline → candidate |
| --- | ---: | ---: | ---: | ---: | ---: |
| IS1008a | 15.73min | 15.06% | 9.95% | 8 → 4 / 4 | 42.01 → 35.88 |
| TS3004a | 22.42min | 34.48% | 21.20% | 14 → 4 / 4 | 45.72 → 50.60 |
| IB4001 | 29.68min | 30.44% | 19.92% | 9 → 4 / 4 | 65.16 → 68.33 |
| Weighted by reference speaker time | 67.84min | **28.06%** | **17.98%** | | |

All six predeclared gates passed: at least 5 percentage points weighted gain,
no recording regressing over 3 points, count within one and no count-error
regression, real-time factor at most 0.25, and peak child RSS at most 4 GiB.
Candidate RTF was 0.0376–0.0384 (about 26 times real time); actual kernel-reported
peak was 579,174,400 bytes, versus up to 268,845,056 bytes for the baseline.

The gain is not uniform across error types. On IB4001, missed speaker time
fell 340.61→118.59 seconds, while false alarms rose 47.62→88.52 and speaker
confusion rose 92.01→107.10. All three files had more false-alarm speaker time.
A better aggregate does not mean every attribution behavior is better.

## Speaker assignment to oracle words

A separate policy was frozen in `047195d2` after DER was known but before word
scoring. Official AMI manual words/timings were shared by both engines; only
predicted speaker activity changed. MeetEval 0.4.3 provides cpWER and tcpWER,
with a 0.5 second tcpWER collar. The checked-in policy specifies exact normalization,
interval intersection and abstention behavior. This measures attribution with
oracle words and timing, **not ASR or end-to-end transcription accuracy**.

| Recording | Timed reference tokens | Baseline cpWER → candidate | Baseline tcpWER → candidate |
| --- | ---: | ---: | ---: |
| IS1008a | 2,529 | 16.45% → 12.22% | 17.04% → 12.26% |
| TS3004a | 3,067 | 39.19% → 32.18% | 41.57% → 34.14% |
| IB4001 | 4,690 | 39.30% → 34.67% | 40.94% → 36.38% |
| Word-weighted | **10,286** | **33.65% → 28.41%** | **35.25% → 29.78%** |

All 1,889 excluded XML word entries had zero-duration spans (386/594/909 by file).
Some XML entries normalize to more than one token. Excluding these entries
limits coverage and must not be represented as scoring every spoken word.

The conservative attribution rule abstains on any intersecting competing
identity or UNKNOWN activity; these words count as attribution deletions rather
than letting UNKNOWN become a free optimally mapped speaker. Actual transcript
text is not deleted. Abstentions fell 2,857→2,518, still 24.5% of scored tokens
for the candidate. Of its abstentions, 2,449 were competing-speaker intervals and
69 had no predicted activity. This exposes a remaining overlap/alignment problem:
fuller diarization alone does not establish a safe final word-attribution UX.
No promotion gate was added or retuned from this descriptive follow-up.

## Process isolation and cancellation

The offline supervisor uses a separate process group, minimal environment,
nice 10, a 15 minute wall deadline, one-hour admitted-audio ceiling, a 4 GiB sampled
RSS watchdog and a 16 MiB kernel output-file limit. The RSS guard is not a kernel
memory reservation. Per-child peak memory is measured separately with `wait4`.

All 17 scorer/supervisor/word tests passed on the native Mac, including actual
worker-descendant cancellation, deadline/RSS termination, output limiting,
provider-environment exclusion, global permutation, all-unknown deletion penalty
and time-constrained word matching. Linux passed 15 with the two optional
MeetEval tests skipped. Installing MeetEval's compiled extensions on the
resource-constrained VM failed with SIGTERM; the actual metrics ran successfully
in an isolated Mac Python 3.12.13 environment.

Hosted macOS CI subsequently exposed an already-reaped process-group cleanup
race. Cleanup now accepts a denied signal only when process metadata confirms
no live group member remains; denied termination of a live member still fails.
The additional regression control is included in the 17 tests above. This changes
supervision cleanup only; the frozen `13b790ca` model report retains its original
runner hash, model configuration and predictions.

Separately, the actual native candidate on IB4001 was cancelled after one second;
it exited by SIGKILL after 1.280 seconds with the WAV SHA256 unchanged. The exact
internal inference phase was not instrumented. This proves this offline worker
can be terminated without changing its input, not that installed live capture
has been qualified. The normal installed Minutes/Minutes Dev apps were untouched. A repeat with the
cleanup correction terminated in 1.269 seconds and preserved the same input hash.

## Integration recommendation and remaining gates

Preserve the October live ASR latency fixes. As documented in the
[history audit](diarization-history-and-live-latency-2026-10-18.md), the older
flattening was a speaker-quality approximation, not a prerequisite for immediate
live text. Keep immediate words independent of optional later speaker refinement.

Next, implement an opt-in supervised full-pipeline helper with explicitly pinned
local models, its own embedding namespace, duration/observation/resource bounds,
and cancellation that cannot interrupt recording or WAV preservation. Do not
reuse existing voice profiles merely because embedding dimensions match.

Before promotion, qualify Linux, Windows and Intel native behavior; concurrent
live first-text latency and resource contention; actual ASR word timing and
turn/text preservation; short interjections, similar voices, multilingual/Dutch
and headphone-call cases; named-identity false positives; and signed installed
app/CLI parity. This comparison has no named enrollment, live capture, UI or
cross-platform candidate acceptance. Existing four-platform Minutes regression
controls are a separate test of the shipped approximation.

## Reproduction receipts

The [README](../../tests/eval/diarization/README.md) contains explicit download,
build and scoring instructions. Hash-bound public numeric reports:

- [Frozen model manifest](../../tests/eval/diarization/full-native-ami.json)
- [Native DER/count/resource report](../../tests/eval/diarization/reports/2026-10-18/full-native.json)
- [Frozen word policy](../../tests/eval/diarization/oracle-word-policy.json)
- [Oracle-word report](../../tests/eval/diarization/reports/2026-10-18/oracle-words.json)
- [Actual native cancellation receipt](../../tests/eval/diarization/reports/2026-10-18/native-cancellation.json)
- [Cancellation with corrected cleanup](../../tests/eval/diarization/reports/2026-10-18/native-cancellation-cleanup.json)

Raw predictions, annotations, models and local build logs remain private task
artifacts on the evaluation host; they are not bundled into Minutes or this PR.
