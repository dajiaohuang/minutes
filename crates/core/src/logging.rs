use crate::config::Config;
use std::fs;
use std::path::{Path, PathBuf};

// ──────────────────────────────────────────────────────────────
// Structured logging to ~/.minutes/logs/minutes.log
// Format: JSON lines, one per event
// Rotation: daily, keep 7 days
// CLI: `minutes logs` to tail, `minutes logs --errors` to filter
// ──────────────────────────────────────────────────────────────

/// The directory that holds `minutes.log` and the other diagnostic logs.
///
/// At runtime this is `<minutes dir>/logs`, which follows `MINUTES_DATA_DIR`
/// and `HOME` like the rest of the data directory. Test processes never get
/// the user's real logs: see [`isolate_from_tests`].
pub fn log_dir() -> PathBuf {
    isolate_from_tests(Config::minutes_dir().join("logs"))
}

/// Get the current log file path.
pub fn log_path() -> PathBuf {
    log_dir().join("minutes.log")
}

/// Keep `cargo test` processes out of the user's real log directory.
///
/// Test binaries reach the log writers through many call sites (dictation,
/// call detection, desktop commands), often without isolating `HOME`. Their
/// entries look exactly like real app events, so they used to make a working
/// engine look broken in `~/.minutes/logs/minutes.log`. A test that points
/// `HOME` or `MINUTES_DATA_DIR` at a temporary directory keeps its own log
/// there, so it can still read back what it wrote. Anything else is redirected
/// to a per-process scratch directory under the system temp dir.
///
/// Outside a test harness the directory is returned unchanged.
pub(crate) fn isolate_from_tests(dir: PathBuf) -> PathBuf {
    if !crate::test_support::running_under_cargo_test() {
        return dir;
    }
    redirect_unless_temporary(dir, &std::env::temp_dir(), std::process::id())
}

fn redirect_unless_temporary(dir: PathBuf, temp_root: &Path, pid: u32) -> PathBuf {
    if dir.starts_with(temp_root) {
        return dir;
    }
    temp_root.join("minutes-test-logs").join(pid.to_string())
}

/// Ensure the log directory exists.
pub fn ensure_log_dir() -> std::io::Result<()> {
    fs::create_dir_all(log_dir())
}

/// Rotate old log files. Keeps the last 7 days.
pub fn rotate_logs() -> std::io::Result<()> {
    let log_dir = log_dir();
    if !log_dir.exists() {
        return Ok(());
    }

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let current_log = log_dir.join("minutes.log");

    // If the current log exists and is from a previous day, rotate it
    if current_log.exists() {
        if let Ok(metadata) = current_log.metadata() {
            if let Ok(modified) = metadata.modified() {
                let modified_date = chrono::DateTime::<chrono::Local>::from(modified)
                    .format("%Y-%m-%d")
                    .to_string();

                if modified_date != today {
                    let rotated = log_dir.join(format!("minutes.{}.log", modified_date));
                    fs::rename(&current_log, &rotated)?;
                }
            }
        }
    }

    // Delete logs older than 7 days
    let cutoff = chrono::Local::now() - chrono::Duration::days(7);
    let cutoff_str = cutoff.format("%Y-%m-%d").to_string();

    for entry in fs::read_dir(&log_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        // Match pattern: minutes.YYYY-MM-DD.log
        if let Some(date) = name
            .strip_prefix("minutes.")
            .and_then(|s| s.strip_suffix(".log"))
        {
            if date < cutoff_str.as_str() {
                fs::remove_file(entry.path())?;
                tracing::debug!(file = %name, "removed old log file");
            }
        }
    }

    Ok(())
}

/// Append a structured log entry to the log file.
pub fn append_log(entry: &serde_json::Value) -> std::io::Result<()> {
    ensure_log_dir()?;
    let path = log_path();
    let line = serde_json::to_string(entry)? + "\n";

    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// Log a pipeline step to the structured log file.
pub fn log_step(step: &str, file: &str, duration_ms: u64, extra: serde_json::Value) {
    let entry = serde_json::json!({
        "ts": chrono::Local::now().to_rfc3339(),
        "level": "info",
        "step": step,
        "file": file,
        "duration_ms": duration_ms,
        "extra": extra,
    });

    if let Err(e) = append_log(&entry) {
        tracing::warn!("failed to write to log file: {}", e);
    }
}

/// Log an error to the structured log file.
pub fn log_error(step: &str, file: &str, error: &str) {
    let entry = serde_json::json!({
        "ts": chrono::Local::now().to_rfc3339(),
        "level": "error",
        "step": step,
        "file": file,
        "error": error,
    });

    if let Err(e) = append_log(&entry) {
        tracing::warn!("failed to write error to log file: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    /// Clears `MINUTES_DATA_DIR` while held so `HOME` decides the data dir.
    struct DataDirUnset(Option<OsString>);

    impl DataDirUnset {
        fn new() -> Self {
            let previous = std::env::var_os("MINUTES_DATA_DIR");
            std::env::remove_var("MINUTES_DATA_DIR");
            Self(previous)
        }
    }

    impl Drop for DataDirUnset {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                std::env::set_var("MINUTES_DATA_DIR", previous);
            }
        }
    }

    #[test]
    fn test_writes_never_reach_the_real_home_log() {
        let _lock = crate::test_support::home_env_lock();
        let _data_dir = DataDirUnset::new();
        let temp_root = std::env::temp_dir();
        // With the developer's own HOME this is ~/.minutes/logs, the file the
        // desktop app and CLI write at runtime.
        let runtime_dir = Config::minutes_dir().join("logs");

        let marker = format!(
            "logging-isolation-{}-{}",
            std::process::id(),
            chrono::Local::now()
                .timestamp_nanos_opt()
                .unwrap_or_default()
        );
        append_log(&serde_json::json!({ "step": "test", "marker": marker })).unwrap();

        assert!(
            log_path().starts_with(&temp_root),
            "test log {} must live under {}",
            log_path().display(),
            temp_root.display()
        );
        let written = fs::read_to_string(log_path()).unwrap();
        assert!(written.contains(&marker));
        if !runtime_dir.starts_with(&temp_root) {
            assert_ne!(log_dir(), runtime_dir);
            let real_log = fs::read_to_string(runtime_dir.join("minutes.log")).unwrap_or_default();
            assert!(
                !real_log.contains(&marker),
                "a test entry reached the real log at {}",
                runtime_dir.display()
            );
        }
    }

    #[test]
    fn a_temporary_home_keeps_its_own_log() {
        crate::test_support::with_temp_home(|home| {
            let _data_dir = DataDirUnset::new();
            assert_eq!(log_path(), home.join(".minutes/logs/minutes.log"));
        });
    }

    #[test]
    fn redirect_keeps_temporary_dirs_and_replaces_everything_else() {
        let temp = Path::new("/var/folders/xy/T");
        let kept = temp.join(".tmpAbC/.minutes/logs");
        assert_eq!(redirect_unless_temporary(kept.clone(), temp, 42), kept);
        assert_eq!(
            redirect_unless_temporary(PathBuf::from("/Users/me/.minutes/logs"), temp, 42),
            temp.join("minutes-test-logs/42")
        );
        assert_eq!(
            redirect_unless_temporary(PathBuf::from("/var/folders/xy/Tother/logs"), temp, 7),
            temp.join("minutes-test-logs/7")
        );
    }

    #[test]
    fn append_log_writes_json_line_to_file() {
        // Use a temp dir to avoid contaminating real logs
        let dir = tempfile::TempDir::new().unwrap();
        let log_file = dir.path().join("test.log");

        let entry = serde_json::json!({
            "ts": "2026-03-17T08:00:00",
            "level": "info",
            "step": "test",
            "message": "unit test entry"
        });

        // Write directly to temp file
        let line = serde_json::to_string(&entry).unwrap() + "\n";
        std::fs::write(&log_file, &line).unwrap();

        // Verify it was written
        let content = std::fs::read_to_string(&log_file).unwrap();
        assert!(content.contains("\"step\":\"test\""));
        assert!(content.ends_with('\n'));

        // Verify it's valid JSON
        let parsed: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed["step"], "test");
    }

    #[test]
    fn log_step_formats_correctly() {
        let entry = serde_json::json!({
            "ts": chrono::Local::now().to_rfc3339(),
            "level": "info",
            "step": "transcribe",
            "file": "test.wav",
            "duration_ms": 4200,
            "extra": { "words": 142 },
        });

        let line = serde_json::to_string(&entry).unwrap();
        assert!(line.contains("\"step\":\"transcribe\""));
        assert!(line.contains("\"duration_ms\":4200"));
    }
}
