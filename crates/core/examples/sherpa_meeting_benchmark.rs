//! File-only benchmark of the production meeting transcription path.
//! Run on public/synthetic fixtures, with the same arguments on baseline and candidate.
//! cargo run -p minutes-core --no-default-features --features engine-sherpa \
//!   --example sherpa_meeting_benchmark -- MODEL_ROOT AUDIO.wav [AUDIO.wav ...]
#[cfg(feature = "engine-sherpa")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use minutes_core::{config::Config, transcribe};
    use std::{path::PathBuf, time::Instant};

    tracing_subscriber::fmt()
        .with_env_filter("minutes_core=info")
        .with_writer(std::io::stderr)
        .init();
    let mut args = std::env::args().skip(1);
    let model_root = PathBuf::from(args.next().ok_or("expected model root and WAV paths")?);
    let paths: Vec<_> = args.map(PathBuf::from).collect();
    if paths.is_empty() {
        return Err("expected at least one WAV path".into());
    }
    let mut config = Config::default();
    config.transcription.engine = "sherpa".into();
    config.transcription.model_path = model_root;
    config.transcription.min_words = 1;
    config.transcription.noise_reduction = false;
    let mut reports = Vec::new();
    for path in paths {
        let started = Instant::now();
        let result = transcribe::transcribe_meeting(&path, &config)?;
        reports.push(serde_json::json!({
            "audio": path,
            "elapsed_ms": started.elapsed().as_millis(),
            "audio_seconds": result.stats.audio_duration_secs,
            "raw_segments": result.stats.raw_segments,
            "final_words": result.stats.final_words,
            "text": result.text,
        }));
    }
    println!("{}", serde_json::to_string_pretty(&reports)?);
    Ok(())
}

#[cfg(not(feature = "engine-sherpa"))]
fn main() {
    eprintln!("requires --features engine-sherpa");
    std::process::exit(2);
}
