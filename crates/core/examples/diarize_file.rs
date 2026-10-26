//! Offline model-only diarization example. No ASR, enrollment, capture or playback.
use minutes_core::{config::Config, diarize};
use std::{path::PathBuf, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 || !diarize::DIARIZATION_MODEL_NAMES.contains(&args[0].as_str()) {
        return Err(
            "usage: diarize_file legacy|community-1 MODELS_DIR AUTHORIZED_AUDIO.wav".into(),
        );
    }
    let mut config = Config::default();
    config.diarization.engine = "pyannote-rs".into();
    config.diarization.model = args[0].clone();
    config.diarization.model_path = PathBuf::from(&args[1]);
    let start = Instant::now();
    let result = diarize::diarize(&PathBuf::from(&args[2]), &config)
        .ok_or("diarization unavailable; no hypothesis emitted")?;
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "model": config.diarization.model,
            "embedding_model": diarize::embedding_model_for_config(&config).version,
            "processing_seconds": start.elapsed().as_secs_f64(),
            "segments": result.segments,
        }))?
    );
    Ok(())
}
