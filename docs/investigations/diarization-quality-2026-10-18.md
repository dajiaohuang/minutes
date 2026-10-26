# Diarization quality: October 18, 2026

Minutes must preserve what people said, separate their voices over a complete
recording, and abstain when there is insufficient evidence for a name. Capture
and the original WAV remain independent of optional diarization.

## What the research changes

The most useful reference for our current local engine is the
[pyannote powerset paper](https://arxiv.org/abs/2310.13025) and the
[current upstream pipeline](https://github.com/pyannote/pyannote-audio/blob/main/src/pyannote/audio/pipelines/speaker_diarization.py).
Powerset classes represent individual speakers and pairs speaking together.
Upstream retains those local masks, extracts per-speaker embeddings, clusters
across windows, and reconstructs speech activity. Local speaker indices are
permutation invariant: channel 1 in one window is not a persistent identity.
The upstream extractor can exclude overlapping frames from embedding evidence.

[Community-1](https://huggingface.co/pyannote/speaker-diarization-community-1)
is an open, offline pipeline with improved speaker counting and an additional
exclusive diarization output for alignment with transcript timestamps. Its
published benchmarks cover the complete upstream pipeline, including VBx/PLDA
clustering; using its exported segmentation and embedding networks in Minutes
does not reproduce that pipeline or inherit its scores. The model card uses
zero collar and includes overlap. Its CC-BY-4.0 weights require attribution;
the original download also has an access acknowledgement. Our existing ONNX
exports are a separately distributed conversion, not a new upstream runtime.

[Nemotron 3 Diarization](https://huggingface.co/nvidia/Nemotron-3-Diarization),
released September 23, 2026, is a serious streaming challenger. It supports up
to eight speakers and carries speaker information between chunks with an
arrival-order cache. Its recommended lowest input buffering is 320 ms; that
is not end-to-end latency. It has a different architecture and OpenMDW-1.1
license, and does not replace Minutes' named voice embeddings. A native port
would need measured CPU/Apple Silicon/Intel/Windows behavior, cancellation,
long-call identity continuity, and a separate voice-recognition policy. It is
not a drop-in replacement justified by a leaderboard number.

[Precision-3](https://www.pyannote.ai/blog/precision-3), released in September
2026, is a useful commercial reference, with frame-level speaker probabilities
and sensitivity controls. API/on-premise availability does not make it an
open-source, offline, redistributable Minutes default. No meeting audio is
sent to a provider by this work.

## Defects in our pipeline

The old ONNX integration collapsed all non-silent classes into a speech boolean.
It merged regions across short pauses before embedding extraction and could
assign an embedding from the beginning of a long, mixed region to every voice
inside that region. A separate frame-clock error accumulated across ten-second
windows. Repetition cleanup could also delete distinct numbered utterances.
The clock and cleanup defects are fixed separately in #1152 and #1151.

## Implementation decision

Community-1 receives a speaker-aware path:

- Decode all seven powerset classes into three local activity tracks per window.
- Keep paired speaker activity during overlap in the raw diarization result.
- Extract embeddings from that speaker's exclusive frames, bounded by a ten-second
  model window. Require 1.5 seconds of clean speech for identity evidence.
- Cluster all reliable observations globally by merging the most similar
  compatible centroids first, weighted by clean speech duration. Different
  local voices in one window cannot collapse through transitive merging.
- Short or overlap-only tracks remain UNKNOWN; they do not inherit another
  person's identity because that person happened to speak nearby.
- Transcript alignment uses an exclusive view. Ambiguous simultaneous voices
  remain UNKNOWN rather than arbitrarily attributing their words to one person.
- Keep model-specific voice-profile compatibility, cancellation, finite output
  validation, the two-hour audio budget, 100,000 region budget, and 64-result-cluster cap. Global clustering permits at most 2,160 observations
  and 2,160 squared queued pairs, using fallible queue allocation and cancellation
  during construction and merging.

Legacy CAM++ retains its existing speech-region compatibility path plus the
clock fix. A development fixture regressed with the new track path under CAM++,
so it is not enabled for that model. Neither the model default nor recognition
thresholds change. This is not the complete upstream Community-1/VBx pipeline.

CLI renderer qualification also covers a separate alignment boundary: an ASR
segment can span simultaneous voices or consecutive turns. Both text and JSON
must return UNKNOWN when their interval intersects different raw speaker labels,
independent of raw segment order. Tests cover overlap, sequential turns, one
supported voice, absent evidence, invalid intervals and unchanged words.
This renderer correction does not change model predictions or the paired DER
results below.

The downstream audit also distinguishes explicit UNKNOWN evidence from an absent
observation. Saved-meeting onset/dominance heuristics preserve the former,
including UNKNOWN reached through the existing boundary tolerance. Enrollment
refuses inconsistent speaker counts or multiple embeddings before touching the
voice-profile database. CLI renderer and enrollment contracts run explicitly on
all three main CI platforms. The model workflow keeps the immutable clock-fixed
comparator after the new pipeline lands; its already-seen meetings are regression
controls, not a new holdout for these downstream corrections.

## Evaluation and admission

The first development set consists of direct Apple TTS WAVs: two Dutch voices
with 350 ms and 1.5 s gaps, two English voices, and three English voices with
some overlap. These test turn preservation and timing, not human conversational
quality. Known voice schedules have approximate energy-trimmed boundaries.
The first 300 seconds of AMI ES2004a is also development data.

The first admission used full AMI IS1009a and TS3003a Array1-01 meetings.
An online clustering candidate improved weighted DER by 4.38 points, below the
predeclared five-point minimum; IS1009a improved from 44.60% to 33.10% and
TS3003a regressed from 27.08% to 27.53%. It was rejected. These recordings are
now development data. The replacement uses bounded, cancellable global centroid
agglomeration, addressing the general arrival-order defect without threshold
tuning. A regression with three acoustic vectors demonstrates that an early
ambiguous voice must join its strongest compatible observation rather than the
first acceptable template.

The global candidate is frozen before evaluating AMI EN2002a and EN2002b, from
a different speaker group. These two recordings share speakers with each other;
they are not independent population samples. The gate thresholds below remain
unchanged. Audio and annotations are
[openly licensed CC-BY-4.0 by the AMI project](https://groups.inf.ed.ac.uk/ami/).
Use the only-words RTTMs from pyannote/AMI-diarization-setup commit
67c2d539286e89f68952d5dcf83912bd9f01dfae. Record the actual audio/model/executable
hashes and source delta. This is a small engineering holdout, not independently
blinded certification or evidence that the model never trained on AMI.

Before either admission, the thresholds are:
weighted DER improves by at least five percentage points against the clock-fixed
baseline; no recording regresses by more than three percentage points; each
candidate run has real-time factor below 0.25 on this Linux CPU; and all source
regression tests pass. DER uses one optimal mapping per complete recording,
zero collar and overlap included. UNKNOWN is excluded from attributed hypotheses
and its duration is reported separately, so abstention cannot become a reference
speaker through optimal remapping. Record speech misses, false alarms, confusion,
and speaker count as well as the aggregate. A failure prevents enabling the new
path in a release. No threshold tuning on these two holdout outputs.

A model-default change needs broader, separately collected consented/licensed
human speech across languages, devices, cross-talk, short interjections, similar
voices, and headphone capture. It also needs native latency/memory and named
recognition qualification. No current result establishes that promotion.

## Observed local admission result

The frozen global candidate passed the unchanged regression gate on EN2002a and
EN2002b. The source was built before rebasing the implementation onto main;
its frozen Rust file hashes match original commit
dfa3c3c2459dda9a40a9c2b1c93a93322bcd2735. The baseline is clock-fixed
d67b0f7e101969f370e23acfdbe98d6a40898bde. Exact executable/model hashes and
harness timings are retained in the run receipt. The hosted workflow qualifies
the actual release candidate independently of these historical local binaries.

| Complete recording | Baseline DER | Global candidate DER | Candidate RTF | Candidate clusters / reference people |
| --- | ---: | ---: | ---: | ---: |
| EN2002a, 35.7 minutes | 59.60% | 38.72% | 0.063 | 17 / 4 |
| EN2002b, 29.8 minutes | 59.17% | 43.84% | 0.052 | 14 / 4 |
| Weighted by reference speaker-time | 59.42% | 40.94% | | |

The gain is 18.47 percentage points; neither file regressed. The earlier scorer
refused more than twelve labels, so the first scoring attempt stopped. Its
replacement uses exact bitmask assignment for small matrices and bounded cubic
Hungarian assignment above twelve, up to 65 labels (64 runtime speakers plus
abstention). It is checked against the small exact solver and a 64-label known
assignment. No audio, model, predictions or gate thresholds were changed to
resolve that scoring limitation.

These results expose serious remaining gaps rather than establish SOTA: speaker
counting is fragmented and substantial speech lacks reliable identity evidence.
The next useful comparison is full upstream window reconstruction and
model-calibrated PLDA/VBx against this portable ONNX path, plus the separate
streaming challenger. Evaluate speaker-count error, short-turn coverage and
named-identity false positives alongside DER on new consented/licensed voices.
These now-seen meetings cannot become a fresh holdout through another rerun.

## First candidate for the next comparison

The June entity-resolution plan already identified `speakrs`; refresh that lead
before implementing another clustering stack ourselves. Its
[current source, pinned to af8013f](https://github.com/avencera/speakrs/tree/af8013f95a53a37a554d8abc6e4ae2ab9a614be6),
checked October 18, describes a full Community-1-style pipeline with overlapping
windows, activity reconstruction, PLDA and VBx. Version 0.7 offers native CPU and
CoreML backends. Its Apache-2.0 implementation is a more direct candidate for
Minutes than a Python service or an Apple-only replacement. Native CPU uses a
different local weights bundle, so the existing ONNX pair is not a drop-in input.
Its author-reported benchmark scores are not reproduced Minutes results.

First compare this candidate against 0.30.4 on fresh licensed or consented
recordings, with the dataset, model hashes and admission thresholds frozen
before scoring. Report speaker-count error, short-turn coverage, overlap,
misses/false alarms/confusion and named-identity false positives, plus runtime,
peak memory and cancellation. Verify Windows and Intel behavior rather than
inferring support from the CPU backend name. Keep downloads explicit, audit
model redistribution/attribution, and preserve model-specific voice profiles.
If it passes, integrate behind an opt-in engine and qualify installed-app/CLI
parity before considering a default change. Recording and WAV preservation
remain isolated from this optional processing.

Score the words assigned to each speaker as well as raw speaker time. Add
cpWER/tcpWER with declared normalization and timing policy, and preserve
structured word/segment timestamps for safe alignment. The
[CHiME-7/8 review](https://arxiv.org/abs/2507.18161v2) shows that convincing
summaries can conceal substantial speaker-attributed transcription errors.
Summary quality alone cannot qualify Minutes. An ASR interval spanning several
voices must abstain until its words can be assigned with adequate timing
evidence; assigning the whole sentence to its first speaker is insufficient.

## Completed native comparison

The [full native comparison](full-native-diarization-2026-10-18.md) now has
source-bound Apple Silicon results on three additional AMI development groups:
weighted DER 28.06%→17.98%, candidate counts 4/4/4 for four reference speakers in
each meeting, and about 26 times real-time CPU processing. All predeclared gates
passed. A separate oracle-word diagnostic improved tcpWER 35.25%→29.78%; this
uses official words/timing and is not end-to-end ASR accuracy. Overlap remains
a substantial attribution gap. Native cancellation preserved the input WAV.

This advances the comparison step, not the broader promotion gates. Named
identity, multilingual/short-turn coverage, other native platforms, concurrent
live latency and installed app parity remain unqualified. The
[historical audit](diarization-history-and-live-latency-2026-10-18.md) also
establishes that old fragment merging targeted embedding stability; the recent
live-transcription responsiveness fixes are a separate path and should remain.
