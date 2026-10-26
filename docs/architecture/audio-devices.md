# Audio Device Setup Guide

For desktop meetings, choose **Meeting audio** before recording: **Microphone only** for a room meeting, or **Microphone + system audio** for a computer call. Minutes remembers the choice. The microphone picker selects your input device; it does not add the other side of a call by itself.

## Quick reference

| Scenario | Input device | Setup needed |
|----------|-------------|--------------|
| In-person meeting | Built-in Microphone | None |
| External USB mic (Blue Yeti, Rode, etc.) | Your mic name | Plug in, select in System Settings > Sound > Input |
| Zoom / Teams / Meet call (desktop app, macOS 15+) | Microphone + system audio | Grant Screen Recording + Microphone permission; call detection is optional |
| Zoom / Meet / Teams call (CLI, macOS 14.4+) | Core Audio Process Tap + microphone | Configure both backend and call source (see below) |
| Computer call with loopback devices | Separate microphone + BlackHole 2ch | Install BlackHole + Multi-Output Device (see below) |
| iPhone voice memo | N/A (file-based) | Configure folder watcher |

## Capturing system audio (Zoom, Meet, Teams)

**Desktop app (recommended, macOS 15+): you do not need BlackHole.** The desktop app records both your microphone and the call's audio natively using ScreenCaptureKit, with no virtual audio device. To use it:

1. Grant Minutes **Screen Recording** and **Microphone** permission in System Settings > Privacy & Security.
2. Select **Meeting**, choose **Microphone + system audio**, and click **Start**. This works without a detected call or an open call banner. System audio includes other apps that play sound, so silence notifications if needed.
3. During capture, check the separate microphone and call-audio indicators. If native capture cannot start, Minutes stops with an error; it does not silently switch to microphone-only recording.

Call detection is an optional convenience: Zoom, Microsoft Teams desktop and Webex are enabled by default. Google Meet and Teams-in-browser detection are opt-in in Settings and may require Automation permission. The banner's **Record** action explicitly starts call capture, regardless of the saved room-meeting choice. Tray and shortcut Meeting starts use the saved choice; a new installation asks you to choose before starting. Memos always use the microphone.

**CLI on macOS 14.4+: Core Audio Process Tap.** The CLI has an independent native system-audio backend. Configure both keys:

```toml
[recording]
capture_backend = "core-audio-tap"

[recording.sources]
voice = "default"
call = "auto"
```

Then run `minutes record --call`. Process Tap uses system-audio access rather than the desktop helper's Screen Recording route. See [Core Audio Process Tap](core-audio-process-tap.md) for platform and permission details. The desktop Meeting choice does not change CLI defaults.

**Loopback fallback:** BlackHole is an alternative for older macOS or an explicit loopback setup. To retain both sides, configure separate voice and call sources rather than selecting BlackHole as your only input.

### 1. Install BlackHole

```bash
brew install blackhole-2ch
```

### 2. Create a Multi-Output Device

This lets you hear the call AND record it simultaneously:

1. Open **Audio MIDI Setup** (Spotlight → type "Audio MIDI Setup")
2. Click the **+** button at bottom-left → **Create Multi-Output Device**
3. Check both:
   - Your speakers/headphones (e.g., "MacBook Pro Speakers" or "AirPods Pro")
   - **BlackHole 2ch**
4. Make sure your speakers/headphones are listed FIRST (drag to reorder — the first device is what you hear)
5. Right-click the Multi-Output Device → **Use This Device For Sound Output**

### 3. Configure Minutes

Configure a microphone and a separate BlackHole call source:

```toml
# ~/.config/minutes/config.toml
[recording]
capture_backend = "cpal"

[recording.sources]
voice = "default"
call = "BlackHole 2ch"
```

Or pass it per-recording:
```bash
minutes record --call --title "Team standup"
```

### 4. After the call

Switch your output back to normal speakers/headphones. The Multi-Output Device routes audio through both outputs, which can add slight latency for music/video.

## iPhone voice memos (auto-import)

Use the folder watcher to auto-process voice memos synced from iPhone via iCloud:

```toml
# ~/.config/minutes/config.toml
[watch]
input_dir = "~/Library/Mobile Documents/com~apple~Voicememos/Recordings/"
```

Then: `minutes watch`

Files are moved to `~/meetings/memos/processed/` after transcription.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| Empty transcript | Wrong input device selected | Check System Settings → Sound → Input |
| Only your voice, not the call | Microphone-only route selected | Choose Microphone + system audio in the desktop app; for CLI, configure a separate call source |
| Echo in recording | Recording both mic + system audio | Mute your mic in the call app if you only want call audio |
| BlackHole disappeared | macOS update removed it | `brew reinstall blackhole-2ch` |
| Recording is mono/low quality | Source audio is compressed VoIP | Normal — VoIP audio is always lower quality than in-person |
| Notification sounds in transcript | System sounds captured via BlackHole | Turn on Do Not Disturb during recording |
