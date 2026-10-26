//! Desktop audio intent is explicit and applied to a session-local config.
//! Never persist these route overrides over the user's CLI/source preferences.
use minutes_core::capture::RecordingIntent;
use minutes_core::{CaptureMode, Config};

pub const CHOICE_REQUIRED: &str = "Choose meeting audio in Minutes: microphone only, or microphone + system audio. Recording has not started.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSource {
    Microphone,
    MicrophoneAndSystem,
}

impl AudioSource {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "microphone" => Ok(Self::Microphone),
            "microphone-and-system" => Ok(Self::MicrophoneAndSystem),
            _ => Err(CHOICE_REQUIRED.into()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::MicrophoneAndSystem => "microphone-and-system",
        }
    }
}

pub fn saved_choice(config: &Config) -> Result<Option<AudioSource>, String> {
    if let Some(value) = config.recording.desktop_audio_source.as_deref() {
        return AudioSource::parse(value).map(Some);
    }
    // An existing explicit call-source configuration is already an informed
    // choice. Preserve it; plain CPAL/default microphone is not such a choice.
    Ok(config
        .recording
        .sources
        .as_ref()
        .and_then(|sources| sources.call.as_deref())
        .filter(|source| !source.trim().is_empty())
        .map(|_| AudioSource::MicrophoneAndSystem))
}

pub fn microphone(config: &Config) -> Option<String> {
    config
        .recording
        .sources
        .as_ref()
        .and_then(|sources| sources.voice.clone())
        .or_else(|| config.recording.device.clone())
        .filter(|name| !name.trim().eq_ignore_ascii_case("default"))
        .and_then(|name| minutes_core::capture::canonicalize_input_device_setting(&name))
}

pub fn native_route_allowed(config: &Config, automatic: bool) -> bool {
    // Keep an explicit in-process backend, including the Process Tap workaround
    // already used by people affected by #286. It has its own permission route.
    automatic
        || config.recording.capture_backend == "cpal"
        || config
            .recording
            .sources
            .as_ref()
            .and_then(|sources| sources.call.as_deref())
            .is_none_or(|source| source.trim().is_empty())
}

/// A single loopback input may pass the CLI's system-audio preflight, but it
/// cannot satisfy the desktop's explicit promise to record both sources.
pub fn validate_route(
    config: &Config,
    intent: RecordingIntent,
    native_available: bool,
    microphone_permission: minutes_core::macos_permissions::MacPermissionStatus,
) -> Result<(), String> {
    if intent != RecordingIntent::Call || native_available {
        return Ok(());
    }
    if microphone_permission == minutes_core::macos_permissions::MacPermissionStatus::Denied {
        return Err("Microphone + system audio requires microphone access. Recording has not started. Allow Minutes in System Settings > Privacy & Security > Microphone, then try again.".into());
    }
    if config
        .recording
        .sources
        .as_ref()
        .and_then(|sources| sources.call.as_deref())
        .is_none_or(|source| source.trim().is_empty())
    {
        return Err("Microphone + system audio needs native call capture or separate microphone and system-audio sources. Recording has not started. Set up a call source in Capture settings, or choose Microphone only for a room meeting.".into());
    }
    Ok(())
}

pub fn prepare(
    config: &mut Config,
    mode: CaptureMode,
    requested: Option<RecordingIntent>,
) -> Result<RecordingIntent, String> {
    let intent = if mode == CaptureMode::QuickThought {
        if requested.is_some_and(|intent| intent != RecordingIntent::Memo) {
            return Err("Quick thoughts only support microphone memo capture.".into());
        }
        RecordingIntent::Memo
    } else if let Some(intent) = requested {
        intent
    } else {
        match saved_choice(config)?.ok_or(CHOICE_REQUIRED)? {
            AudioSource::Microphone => RecordingIntent::Room,
            AudioSource::MicrophoneAndSystem => RecordingIntent::Call,
        }
    };

    // Match the displayed input for both native capture and CPAL fallback.
    config.recording.device = microphone(config);
    if intent == RecordingIntent::Call {
        // "Both" must never inherit an older opt-in to degraded mic-only calls.
        config.recording.allow_degraded_call_capture = false;
        if let Some(sources) = config.recording.sources.as_mut() {
            if sources.voice.is_none() {
                sources.voice = config.recording.device.clone();
            }
        }
    } else {
        config.recording.sources = None;
        config.recording.capture_backend = "cpal".into();
        config.recording.auto_call_intent = false;
    }
    Ok(intent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minutes_core::config::SourcesConfig;

    #[test]
    fn fresh_manual_meeting_requires_an_audio_choice() {
        assert_eq!(
            prepare(&mut Config::default(), CaptureMode::Meeting, None),
            Err(CHOICE_REQUIRED.into())
        );
    }

    #[test]
    fn manual_system_capture_does_not_need_detection_or_allow_degraded() {
        let mut config = Config::default();
        config.recording.desktop_audio_source = Some("microphone-and-system".into());
        config.recording.allow_degraded_call_capture = true;
        assert_eq!(
            prepare(&mut config, CaptureMode::Meeting, None).unwrap(),
            RecordingIntent::Call
        );
        assert!(!config.recording.allow_degraded_call_capture);
    }

    #[test]
    fn microphone_choice_removes_system_routes_only_from_session_copy() {
        let mut saved = Config::default();
        saved.recording.desktop_audio_source = Some("microphone".into());
        saved.recording.capture_backend = "core-audio-tap".into();
        saved.recording.sources = Some(SourcesConfig {
            voice: Some("Studio Mic".into()),
            call: Some("auto".into()),
        });
        let mut session = saved.clone();
        assert_eq!(
            prepare(&mut session, CaptureMode::Meeting, None).unwrap(),
            RecordingIntent::Room
        );
        assert_eq!(session.recording.device.as_deref(), Some("Studio Mic"));
        assert!(session.recording.sources.is_none());
        assert_eq!(session.recording.capture_backend, "cpal");
        assert_eq!(
            saved.recording.sources.unwrap().call.as_deref(),
            Some("auto")
        );
        assert_eq!(saved.recording.capture_backend, "core-audio-tap");
    }

    #[test]
    fn existing_explicit_call_source_is_preserved_without_new_preference() {
        let mut config = Config::default();
        config.recording.sources = Some(SourcesConfig {
            voice: Some("Studio Mic".into()),
            call: Some("BlackHole 2ch".into()),
        });
        assert_eq!(
            prepare(&mut config, CaptureMode::Meeting, None).unwrap(),
            RecordingIntent::Call
        );
        assert_eq!(
            config.recording.sources.unwrap().call.as_deref(),
            Some("BlackHole 2ch")
        );
    }

    #[test]
    fn an_explicit_default_voice_source_uses_the_system_microphone() {
        let mut config = Config::default();
        config.recording.device = Some("Old USB Mic".into());
        config.recording.sources = Some(SourcesConfig {
            voice: Some("default".into()),
            call: Some("auto".into()),
        });
        assert_eq!(microphone(&config), None);
        prepare(
            &mut config,
            CaptureMode::Meeting,
            Some(RecordingIntent::Call),
        )
        .unwrap();
        assert_eq!(config.recording.device, None);
    }

    #[test]
    fn explicit_process_tap_keeps_its_backend_and_selected_microphone() {
        let mut config = Config::default();
        config.recording.capture_backend = "core-audio-tap".into();
        config.recording.device = Some("Studio Mic".into());
        config.recording.sources = Some(SourcesConfig {
            voice: None,
            call: Some("auto".into()),
        });
        prepare(&mut config, CaptureMode::Meeting, None).unwrap();
        assert!(!native_route_allowed(&config, false));
        assert!(native_route_allowed(&config, true));
        assert_eq!(config.recording.capture_backend, "core-audio-tap");
        assert_eq!(
            config.recording.sources.as_ref().unwrap().voice.as_deref(),
            Some("Studio Mic")
        );
        config.recording.capture_backend = "cpal".into();
        assert!(native_route_allowed(&config, false));
    }

    #[test]
    fn explicit_call_entry_points_override_a_saved_room_preference() {
        let mut config = Config::default();
        config.recording.desktop_audio_source = Some("microphone".into());
        assert_eq!(
            prepare(
                &mut config,
                CaptureMode::Meeting,
                Some(RecordingIntent::Call)
            )
            .unwrap(),
            RecordingIntent::Call
        );
    }

    #[test]
    fn explicit_room_and_memo_entry_points_never_capture_system_audio() {
        for (mode, intent) in [
            (CaptureMode::Meeting, Some(RecordingIntent::Room)),
            (CaptureMode::QuickThought, None),
        ] {
            let mut config = Config::default();
            config.recording.desktop_audio_source = Some("microphone-and-system".into());
            config.recording.sources = Some(SourcesConfig {
                voice: None,
                call: Some("auto".into()),
            });
            prepare(&mut config, mode, intent).unwrap();
            assert!(config.recording.sources.is_none());
            assert_eq!(config.recording.capture_backend, "cpal");
        }
    }

    #[test]
    fn unknown_saved_choice_never_defaults_to_microphone() {
        let mut config = Config::default();
        config.recording.desktop_audio_source = Some("future-choice".into());
        assert!(prepare(&mut config, CaptureMode::Meeting, None).is_err());
    }

    #[test]
    fn a_single_loopback_cannot_satisfy_the_two_source_desktop_choice() {
        use minutes_core::macos_permissions::MacPermissionStatus as Permission;
        let mut config = Config::default();
        config.recording.device = Some("BlackHole 2ch".into());
        assert!(
            validate_route(&config, RecordingIntent::Call, false, Permission::Granted).is_err()
        );
        assert!(validate_route(&config, RecordingIntent::Call, true, Permission::Granted).is_ok());
        assert!(validate_route(&config, RecordingIntent::Room, false, Permission::Granted).is_ok());
        config.recording.sources = Some(SourcesConfig {
            voice: Some("Studio Mic".into()),
            call: Some("BlackHole 2ch".into()),
        });
        assert!(validate_route(&config, RecordingIntent::Call, false, Permission::Granted).is_ok());
        assert!(validate_route(&config, RecordingIntent::Call, false, Permission::Denied).is_err());
    }
}
