//! Exercise the real setup command without inference or network downloads.
//! Size-only sparse fixtures satisfy setup's existing-file check; they are
//! deliberately not valid models and must never be handed to transcription.
use std::fs;
use std::process::{Command, Output};

struct Profile {
    root: tempfile::TempDir,
    config: std::path::PathBuf,
}

impl Profile {
    fn new(batch: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let models = root.path().join("models");
        fs::create_dir(&models).unwrap();
        for (model, bytes) in [("tiny", 71_u64 << 20), ("base", 136_u64 << 20)] {
            fs::File::create(models.join(format!("ggml-{model}.bin")))
                .unwrap()
                .set_len(bytes)
                .unwrap();
        }
        for vad in ["ggml-silero-v6.2.0.bin", "silero-vad-v6.2.0.onnx"] {
            fs::write(models.join(vad), b"existing setup fixture, not a model").unwrap();
        }
        let config = root.path().join("config.toml");
        fs::write(
            &config,
            format!(
                "# preserve my preferences\n[transcription]\nengine = 'whisper'\nmodel = '{batch}'\nmodel_path = {}\n[dictation]\nmodel = 'base'\n[live_transcript]\nmodel = 'base'\n[future_settings]\nkeep = true\n",
                serde_json::to_string(&models.to_string_lossy()).unwrap()
            ),
        ).unwrap();
        Self { root, config }
    }

    fn setup(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_minutes"))
            .arg("setup")
            .args(args)
            .env("MINUTES_CONFIG_PATH", &self.config)
            .env("MINUTES_HOME", self.root.path().join("home"))
            .env("MINUTES_DATA_DIR", self.root.path().join("data"))
            .env("XDG_CONFIG_HOME", self.root.path().join("xdg"))
            .env("MINUTES_MCP_AUTO_SETUP", "0")
            .output()
            .unwrap()
    }
}

#[test]
fn installing_a_companion_model_keeps_the_entire_config_byte_for_byte() {
    let profile = Profile::new("small");
    let before = fs::read(&profile.config).unwrap();
    let output = profile.setup(&["--model", "base", "--download-only"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&profile.config).unwrap(), before);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Downloading "));
}

#[test]
fn an_explicit_model_choice_changes_only_the_batch_selection() {
    let profile = Profile::new("small");
    let output = profile.setup(&["--model", "base"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(&profile.config).unwrap();
    let config = minutes_core::config::Config::load_from(&profile.config);
    assert_eq!(config.transcription.model, "base");
    assert_eq!(config.transcription.engine, "whisper");
    assert_eq!(config.dictation.model, "base");
    assert_eq!(config.live_transcript.model, "base");
    assert!(text.contains("[future_settings]\nkeep = true"));
    assert!(text.contains("# preserve my preferences"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--download-only"));
}

#[test]
fn plain_setup_preserves_existing_selections_including_explicit_tiny() {
    for selected in ["base", "tiny"] {
        let profile = Profile::new(selected);
        let before = fs::read(&profile.config).unwrap();
        let output = profile.setup(&[]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(&profile.config).unwrap(), before);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Downloading "));
    }
}

#[test]
fn malformed_config_is_not_repaired_or_replaced_by_install_only_setup() {
    let profile = Profile::new("small");
    fs::write(&profile.config, b"[transcription\nmodel = 'medium'").unwrap();
    let before = fs::read(&profile.config).unwrap();
    let output = profile.setup(&["--model", "base", "--download-only"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("Minutes config exists but is malformed"),
        "{stderr}"
    );
    assert_eq!(fs::read(&profile.config).unwrap(), before);
}
