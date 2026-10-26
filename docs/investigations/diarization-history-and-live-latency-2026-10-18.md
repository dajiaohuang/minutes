# Speaker flattening and live-transcription latency

Audit source: `main` at `eb602776d1d71f9f6b7cfa2c4c02ec22c6281d12`.
Reviewed the relevant October 4–18 changes, then followed the affected functions
back to their introduction. This distinguishes recorded rationale from an
inference about an author's intent.

## What was deliberate

The April 5 clustering change, [PR #71](https://github.com/silverstein/minutes/pull/71),
deliberately merged neighboring speech regions to obtain longer, more stable
speaker embeddings. It also skipped embeddings shorter than 1.5 seconds and
inherited short-segment labels from temporal neighbors. The review explicitly
discussed that heuristic's limitations on rapid back-and-forth speech.

This followed the April 3 fragmentation repair,
[`bf87ec1e`](https://github.com/silverstein/minutes/commit/bf87ec1ef753cffbb08bda3769739ab0fcea9e93):
phone audio quality shifts were splitting two people into many anonymous labels.
The objective was identification stability, not first-live-text latency.

The contributor later reported a 20-clip, 180-minute AMI/VoxConverse comparison
in PR #71. Those results favored its greedy-plus-merge clustering over a proposed
AHC/silhouette variant. This was a reported historical experiment, not a result
reproduced here. It shared already-extracted embeddings across alternatives,
so it could not establish that the preceding segmentation retained speaker
boundaries. Its clip IDs do not establish which named files were evaluated.

It would therefore be inaccurate to call every merge or the original work
pointless. Joining fragments of the same speaker is useful. Joining different
speakers before establishing their identities is the problematic assumption.

## Which information was lost

The original native integration used pyannote-rs's speech-region interface.
The April 5 direct ONNX replacement
[`d22619ae`](https://github.com/silverstein/minutes/commit/d22619aef5e6a8470ea3b7497decca0c4279dbe3)
corrected waveform normalization but retained a single speech/non-speech view:
every non-silence powerset class became `true`. That discarded separate local
speaker activity and overlap before identity embeddings were extracted.

The September 2 optional Community-1 integration
[`444560a2`](https://github.com/silverstein/minutes/commit/444560a2e182fe959a45ba448b2c00538cb6cebf)
adapted model tensor contracts while retaining that approximate pipeline. Using
the Community-1 weights therefore did not provide the full upstream pipeline.

Fewer merged regions can incidentally mean fewer embedding inferences. That is
an engineering consequence, not a documented live-latency justification in the
reviewed history. No evidence here establishes that losing speaker boundaries
was necessary to keep live transcription responsive.

## What the recent live changes actually did

| Change | Recorded purpose | Relationship to speaker flattening |
| --- | --- | --- |
| October 7, [#1110](https://github.com/silverstein/minutes/pull/1110) | Capture both sides when going Live. | Capture/source selection, not the offline local-speaker collapse. |
| October 16, [#1144](https://github.com/silverstein/minutes/pull/1144) | Avoid discarded partial-recognition passes, bound delayed audio work, retain recording identity. | Genuine responsiveness work in the live ASR/relay path; preserve it. |
| October 17, [#1146](https://github.com/silverstein/minutes/pull/1146) | Keep background transcription progressing, use one-thread Silero VAD, harden relay I/O. | Background scheduling and transport reliability; preserve it. |
| October 18, [#1153](https://github.com/silverstein/minutes/pull/1153) and [#1156](https://github.com/silverstein/minutes/pull/1156) | Retain Community-1 local speakers, cluster globally, preserve uncertainty downstream. | Repairs the offline attribution path without replacing the live ASR loop. |

The normal `live_transcript.rs` path emits text through its own recognition
workers. Its written lines currently have `speaker: None`. It does not call
the offline `segment_speech`/`merge_short_segments` path to produce immediate
words. The feature-disabled-by-default `streaming_diarize.rs` module describes
a separate delayed-label sidecar design; it has no production call sites in
this inspected revision. Describing that dormant design as an active shipped
live-label feature would be incorrect.

There is also a separate function named `normalize_live_transcript_text` that
joins timestamped ASR text into an utterance string. That formatting behavior
is not the model-speaker flattening discussed here.

## Consequence for the next comparison

Keep immediate live words and bounded capture consumers separate from optional
speaker refinement. A full-recording pipeline can improve later attribution
without becoming a prerequisite for displaying live text. This is an
architecture choice, not proof of zero resource contention: concurrent CPU/GPU
load and actual first-text latency still need measurement before live rollout.

Compare the pinned full native pipeline with released 0.30.4 before integration.
Do not remove useful same-speaker merging, undo the October live fixes, reuse
unqualified voice-profile embeddings, or promote a default on a small offline
result. The existing legacy compatibility path remains a deliberate boundary.
