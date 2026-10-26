# Apple Speech Scope

This document describes the **current shipped scope** of Minutes' experimental
Apple Speech integration. It is intentionally practical and user-facing.

If you want the benchmark evidence that informed this experiment, see
[`docs/designs/apple-speech-benchmark-2026-04-22.md`](designs/apple-speech-benchmark-2026-04-22.md).

## Current product scope

Apple Speech is selectable, opt-in, for **standalone live transcript** in a
trusted signed Minutes app on macOS 26 or later. Whisper stays the default.
Dictation, recording-sidecar live, and batch transcription still resolve an
Apple Speech preference to sealed local Whisper.

The product gate (`apple_speech_private_audio_transport_supported`) is true
only when the running app can bind its embedded worker authority: Developer ID
distribution, the integrity manifest matching the worker cdhash embedded at
build time, and the peer-requirement XPC API. Source, ad-hoc, and symlinked CLI
processes stay on Whisper. Dictation has its own closed switch
(`apple_speech_dictation_selectable`), following the activation plan's surface
order of live transcript, then batch, then dictation.

If Apple Speech fails during a live session (worker crash, Speech error, or an
unsupported runtime), that utterance is transcribed by Whisper and the session
stays on the next engine for the rest of its life.

The transport replaces the pathname handoff with a dedicated service embedded
in the app. Minutes sends bounded 16 kHz mono sample bytes over a nonce-bound,
one-request XPC connection. The service constructs an `AVAudioPCMBuffer`
directly; it accepts no audio pathname, executable, environment, or attachment
and creates no named plaintext utterance file.

- the parent requires the service's exact CodeDirectory hash and Team identity
  before the content-free handshake
- the service reciprocally authenticates the signed Minutes parent before
  accepting any sample frame
- source, ad-hoc, missing-service, altered-service, and unconfirmed-exit cases
  stay on Whisper
- the App-Sandbox-only service has immutable process, address-space, byte, and
  wall-clock ceilings and exits after one terminal response

A signed build must still prove its exact SHA on macOS 26, including the
hostile same-UID open-holder regression. A positive source compile, unsigned
build, or operating-system capability probe is not that proof.

The exact candidate includes a fixed-input, non-product
`--apple-speech-transport-acceptance` route. It accepts no caller audio or path.
It requires the product gate to be open for standalone live transcript and
closed for dictation, and prints both verdicts for the harness to check. After the separately gated signing
job has destroyed its credentials, a no-secret successor job can invoke that
route from the signed app while a same-UID process watches and holds newly
created temporary files. The resulting content-free receipt is acceptance
evidence only after that protected workflow actually runs for the exact SHA.

On real macOS hardware, the separate fixed-input
`--apple-speech-runtime-acceptance` route closes the analyzer confound without
opening the product gate. It embeds the checked-in public speech fixture in the
parent, sends its bounded PCM bytes through the authenticated XPC worker, and
emits only content-free timing and count metrics. It accepts no caller audio,
path, locale, or executable and runs under the same same-UID open-holder
watcher. The harness must run from a repo-shaped directory, because it reads
the public fixture from `crates/assets/demo.wav` relative to `scripts/`.

## Fallback behavior

Standalone live transcript with Apple Speech selected falls back in this order:

1. Apple Speech
2. Parakeet, if separately selectable
3. Whisper

Dictation and recording-sidecar live resolve an Apple Speech preference to
Whisper. No fallback branch creates a named plaintext Apple Speech WAV.

## Shadow measurement

`transcription.apple_speech_shadow = true` runs Apple Speech next to Whisper on
live sessions that ship Whisper text, and logs content-free comparison rows to
`~/.minutes/logs/minutes.log`. A session whose shipped engine is Apple Speech
has no Whisper line to score against, so it logs
`apple_speech_shadow_disabled` with that reason instead.

## What Apple Speech does not do today

Apple Speech does **not** currently:

- replace the recording-sidecar live path used during `minutes record`
- transcribe dictation
- replace post-recording batch transcription or watcher processing
- become selectable from the desktop settings transcription-engine picker
- install missing Speech assets on demand (activation plan Phase 2)

## Related docs

- Benchmark evidence:
  [`docs/designs/apple-speech-benchmark-2026-04-22.md`](designs/apple-speech-benchmark-2026-04-22.md)
- Parakeet setup and scope:
  [`docs/architecture/parakeet.md`](parakeet.md)
