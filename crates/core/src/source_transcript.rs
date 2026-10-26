//! Final ASR for independently captured sources. Never deduplicate across
//! sources: an echo and two people saying the same words are not distinguishable
//! from text alone. Source provenance is not a person's identity.

use crate::diarize::{CaptureSource, DiarizationResult};
use crate::transcribe::{FilterStats, TranscribeResult};

#[derive(Debug, Clone)]
struct SourceLine {
    source: CaptureSource,
    timestamp: Option<String>,
    start: f64,
    end: f64,
    text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceTranscript {
    lines: Vec<SourceLine>,
}

fn source_label(source: CaptureSource) -> &'static str {
    match source {
        CaptureSource::Voice => "MICROPHONE",
        CaptureSource::System => "SYSTEM",
        _ => unreachable!("independent ASR accepts microphone and system only"),
    }
}

fn timestamp_seconds(timestamp: &str) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0;
    for component in timestamp.split(':') {
        let value = component.parse::<f64>().ok()?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        total = total * 60.0 + value;
        count += 1;
    }
    (count == 2 || count == 3).then_some(total)
}

fn source_lines(source: CaptureSource, result: &TranscribeResult) -> Vec<SourceLine> {
    let mut last_start = 0.0_f64;
    let mut lines: Vec<_> = result
        .text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let timed = line.strip_prefix('[').and_then(|rest| rest.split_once(']'));
            let parsed = timed.and_then(|(ts, text)| {
                timestamp_seconds(ts).map(|start| (ts.to_string(), start, text.trim().to_string()))
            });
            let (timestamp, start, text) = match parsed {
                Some((ts, start, text)) => (Some(ts), start, text),
                None => (None, last_start, line.to_string()),
            };
            last_start = start;
            SourceLine {
                source,
                timestamp,
                start,
                end: start,
                text,
            }
        })
        .collect();
    // Some engines/chunked paths expose only timestamped text. Infer an end
    // from the next line in this source, never the competing source's onset.
    for index in 0..lines.len() {
        let next = lines
            .get(index + 1)
            .map(|line| line.start)
            .unwrap_or(result.stats.audio_duration_secs);
        lines[index].end = next.max(lines[index].start);
    }
    // Retain precise timings when the engine supplied the same cleaned text.
    // Match occurrences in order so repeated utterances are not conflated.
    let mut segments = result.segments.iter();
    for line in &mut lines {
        if let Some(segment) = segments.find(|segment| {
            segment.text.trim() == line.text
                && segment.start.is_finite()
                && segment.end.is_finite()
                && segment.start >= 0.0
                && segment.end >= segment.start
                && (segment.start - line.start).abs() < 1.0
        }) {
            line.start = segment.start;
            line.end = segment.end;
        }
    }
    lines
}

pub(crate) fn merge_sources(
    voice: TranscribeResult,
    system: TranscribeResult,
) -> (TranscribeResult, SourceTranscript) {
    let mut lines = source_lines(CaptureSource::Voice, &voice);
    lines.extend(source_lines(CaptureSource::System, &system));
    // Stable sort retains both utterances at equal starts, and subsecond
    // timings where available. No flattening or cross-source text cleanup.
    lines.sort_by(|a, b| a.start.total_cmp(&b.start));
    let sources = SourceTranscript { lines };
    let text = sources.render();
    let a = voice.stats;
    let b = system.stats;
    let stats = FilterStats {
        audio_duration_secs: a.audio_duration_secs.max(b.audio_duration_secs),
        samples_after_silence_strip: a.samples_after_silence_strip + b.samples_after_silence_strip,
        raw_segments: a.raw_segments + b.raw_segments,
        skipped_no_speech: a.skipped_no_speech + b.skipped_no_speech,
        after_no_speech_filter: a.after_no_speech_filter + b.after_no_speech_filter,
        after_dedup: a.after_dedup + b.after_dedup,
        after_interleaved: a.after_interleaved + b.after_interleaved,
        after_script_filter: a.after_script_filter + b.after_script_filter,
        after_noise_markers: a.after_noise_markers + b.after_noise_markers,
        after_trailing_trim: a.after_trailing_trim + b.after_trailing_trim,
        rescued_no_speech: a.rescued_no_speech + b.rescued_no_speech,
        final_words: a.final_words + b.final_words,
    };
    let detected_language = match (voice.detected_language, system.detected_language) {
        (Some(a), Some(b)) if a == b => Some(a),
        (Some(language), None) | (None, Some(language)) => Some(language),
        _ => None,
    };
    (
        TranscribeResult {
            text,
            stats,
            segments: vec![],
            detected_language,
        },
        sources,
    )
}

impl SourceTranscript {
    fn render(&self) -> String {
        self.lines
            .iter()
            .map(|line| {
                let label = source_label(line.source);
                match &line.timestamp {
                    Some(ts) => format!("[{label} {ts}] {}\n", line.text),
                    None => format!("[{label}] {}\n", line.text),
                }
            })
            .collect()
    }

    /// Only source-aware diarization has the reserved local SPEAKER_0 vs
    /// system-label contract. An ordinary mixed-audio cluster number cannot
    /// establish source membership. Even source-aware labels must cover the
    /// whole utterance unambiguously; otherwise keep the capture-source label.
    pub(crate) fn apply_speakers(&self, transcript: &str, result: &DiarizationResult) -> String {
        let rendered: Vec<_> = transcript.lines().collect();
        if rendered.len() != self.lines.len() || !result.source_aware {
            return transcript.to_string();
        }
        self.lines
            .iter()
            .zip(rendered)
            .map(|(line, rendered)| {
                let Some((_, text)) = rendered.split_once(']') else {
                    return format!("{rendered}\n");
                };
                let Some(ts) = &line.timestamp else {
                    return format!("{rendered}\n");
                };
                let overlapping: Vec<_> = result
                    .segments
                    .iter()
                    .filter(|segment| {
                        segment.start.is_finite()
                            && segment.end.is_finite()
                            && segment.start < line.end
                            && segment.end > line.start
                    })
                    .collect();
                if overlapping
                    .iter()
                    .any(|segment| segment.speaker == "UNKNOWN")
                {
                    return format!("{rendered}\n");
                }
                let mut candidates: Vec<_> = overlapping
                    .into_iter()
                    .filter(|segment| match line.source {
                        CaptureSource::Voice => segment.speaker == "SPEAKER_0",
                        CaptureSource::System => segment.speaker != "SPEAKER_0",
                        _ => false,
                    })
                    .collect();
                candidates.sort_by(|a, b| a.start.total_cmp(&b.start));
                let Some(first) = candidates.first() else {
                    return format!("{rendered}\n");
                };
                if first.start > line.start + 0.5 {
                    return format!("{rendered}\n");
                }
                let mut end = first.end;
                for candidate in &candidates[1..] {
                    if candidate.speaker != first.speaker || candidate.start > end + 0.5 {
                        return format!("{rendered}\n");
                    }
                    end = end.max(candidate.end);
                }
                if end < line.end - 0.5 {
                    return format!("{rendered}\n");
                }
                format!(
                    "[{} {ts}] [{}] {}\n",
                    first.speaker,
                    source_label(line.source),
                    text.trim()
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diarize::SpeakerSegment;
    use crate::transcribe::TranscriptSegment;

    fn result(text: &str, duration: f64) -> TranscribeResult {
        TranscribeResult {
            text: text.into(),
            stats: FilterStats {
                audio_duration_secs: duration,
                final_words: text.split_whitespace().count(),
                ..Default::default()
            },
            segments: vec![],
            detected_language: None,
        }
    }

    #[test]
    fn keeps_overlapping_words_and_repeated_responses_from_both_sources() {
        let (merged, _) = merge_sources(
            result("[0:02] keep annual billing\n[0:04] yes\n", 8.0),
            result(
                "[0:01] proposal\n[0:02] send it Tuesday\n[0:04] yes\n",
                10.0,
            ),
        );
        assert_eq!(merged.text, "[SYSTEM 0:01] proposal\n[MICROPHONE 0:02] keep annual billing\n[SYSTEM 0:02] send it Tuesday\n[MICROPHONE 0:04] yes\n[SYSTEM 0:04] yes\n");
        assert_eq!(
            merged.stats.audio_duration_secs, 10.0,
            "wall time is not the sum of the tracks"
        );
    }

    #[test]
    fn orders_subsecond_segments_without_requiring_structured_timing_from_every_engine() {
        let mut voice = result("[0:02] local\n", 4.0);
        voice.segments.push(TranscriptSegment {
            start: 2.8,
            end: 3.0,
            text: "local".into(),
        });
        let mut system = result("[0:02] remote\n", 4.0);
        system.segments.push(TranscriptSegment {
            start: 2.1,
            end: 2.5,
            text: "remote".into(),
        });
        let (merged, _) = merge_sources(voice, system);
        assert_eq!(
            merged.text,
            "[SYSTEM 0:02] remote\n[MICROPHONE 0:02] local\n"
        );
    }

    #[test]
    fn preserves_long_timestamps_untimed_text_and_one_empty_source() {
        let (merged, _) = merge_sources(
            result("", 5.0),
            result("[1:02:03.5] later\nuntimed suffix\n", 4000.0),
        );
        assert!(merged.text.contains("[SYSTEM 1:02:03.5] later\n"));
        assert!(merged.text.contains("[SYSTEM] untimed suffix\n"));
        assert!(!merged.text.contains("MICROPHONE"));
    }

    fn source_diarization() -> DiarizationResult {
        DiarizationResult {
            source_aware: true,
            segments: vec![
                SpeakerSegment {
                    speaker: "SPEAKER_0".into(),
                    start: 0.0,
                    end: 5.0,
                },
                SpeakerSegment {
                    speaker: "SPEAKER_2".into(),
                    start: 0.0,
                    end: 5.0,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn attributes_simultaneous_source_utterances_without_collapsing_them() {
        let (merged, sources) = merge_sources(
            result("[0:01] my words\n", 4.0),
            result("[0:01] their words\n", 4.0),
        );
        let text = sources.apply_speakers(&merged.text, &source_diarization());
        assert_eq!(
            text,
            "[SPEAKER_0 0:01] [MICROPHONE] my words\n[SPEAKER_2 0:01] [SYSTEM] their words\n"
        );
    }

    #[test]
    fn never_assigns_a_source_from_an_ordinary_cluster_number() {
        let (merged, sources) = merge_sources(
            result("[0:01] local\n", 4.0),
            result("[0:01] remote\n", 4.0),
        );
        let mut diarization = source_diarization();
        diarization.source_aware = false;
        assert_eq!(
            sources.apply_speakers(&merged.text, &diarization),
            merged.text
        );
    }

    #[test]
    fn abstains_when_multiple_system_speakers_overlap_or_an_utterance_crosses_a_turn() {
        let (merged, sources) =
            merge_sources(result("", 4.0), result("[0:01] several people\n", 4.0));
        let mut diarization = source_diarization();
        diarization.segments.push(SpeakerSegment {
            speaker: "SPEAKER_3".into(),
            start: 2.0,
            end: 4.0,
        });
        assert_eq!(
            sources.apply_speakers(&merged.text, &diarization),
            merged.text
        );
        diarization
            .segments
            .retain(|segment| segment.speaker == "SPEAKER_0");
        assert_eq!(
            sources.apply_speakers(&merged.text, &diarization),
            merged.text,
            "system audio is never assigned to the microphone speaker"
        );
    }

    #[test]
    fn retains_normalized_text_when_source_attribution_is_added() {
        let (merged, sources) = merge_sources(result("[0:01] matt\n", 4.0), result("", 4.0));
        let normalized = merged.text.replace("matt", "Mat");
        assert!(sources
            .apply_speakers(&normalized, &source_diarization())
            .contains("Mat"));
    }

    #[test]
    fn conflicting_detected_languages_do_not_establish_one_meeting_language() {
        let mut voice = result("[0:00] hello", 4.0);
        voice.detected_language = Some("en".into());
        let mut system = result("[0:00] bonjour", 4.0);
        system.detected_language = Some("fr".into());
        assert_eq!(merge_sources(voice, system).0.detected_language, None);
    }
}
