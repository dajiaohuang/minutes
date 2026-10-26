//! Compatibility path for CAM++ model sets, retained until a measured replacement
//! meets their non-regression gate. Community-1 uses local speaker tracks.
use super::*;
const MAX_EMBEDDING_SECONDS: usize = 60;

#[cfg(feature = "diarize")]
pub(super) fn diarize_legacy_speech_regions(
    audio_path: &Path,
    config: &Config,
    cancellation: &DiarizationCancellation,
) -> Result<DiarizationResult, Box<dyn std::error::Error>> {
    cancellation.check()?;
    let model_dir = &config.diarization.model_path;
    let model_info = diarization_model_for_config(config);
    let seg_model = model_dir.join(model_info.segmentation_filename);
    let emb_info = embedding_model_for_config(config);
    let emb_model = model_dir.join(emb_info.filename);

    if !seg_model.exists() {
        return Err(format!(
            "Segmentation model not found at {}. Run `minutes setup --diarization` to download.",
            seg_model.display()
        )
        .into());
    }
    if !emb_model.exists() {
        return Err(format!(
            "Embedding model not found at {}. Run `minutes setup --diarization` to download.",
            emb_model.display()
        )
        .into());
    }

    let inference_deadline = OrtInferenceDeadline::new(cancellation.clone())?;
    let (f32_samples, sample_rate) = load_audio(audio_path, cancellation)?;
    let mut f32_samples = zeroize::Zeroizing::new(f32_samples);
    cancellation.check()?;

    tracing::info!(
        f32_samples = f32_samples.len(),
        sample_rate = sample_rate,
        "audio loaded for diarization"
    );

    // Step 1: Segment speech using the ONNX model directly with properly
    // normalized f32 input. We bypass pyannote_rs::get_segments because it
    // casts i16 to f32 without dividing by 32768, feeding the model values
    // in [-32768, 32767] when it expects [-1.0, 1.0].
    let mut speech_segments = segment_legacy_speech(
        &f32_samples,
        sample_rate,
        &seg_model,
        model_info,
        inference_deadline.options(),
    )?;

    // If the model found no speech, the audio may be too quiet (e.g. MacBook
    // built-in mic with peaks as low as 0.0004). Normalize to a usable level
    // and retry — this avoids hardcoding a sensitivity threshold.
    if speech_segments.is_empty() {
        let peak = f32_samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        const TARGET_PEAK: f32 = 0.5;

        if peak > 0.0 && peak < TARGET_PEAK {
            let gain = TARGET_PEAK / peak;
            tracing::info!(
                peak = format!("{:.6}", peak),
                gain = format!("{:.1}x", gain),
                "no speech detected — retrying with normalized audio"
            );
            for s in f32_samples.iter_mut() {
                *s = (*s * gain).clamp(-1.0, 1.0);
            }
            cancellation.check()?;
            speech_segments = segment_legacy_speech(
                &f32_samples,
                sample_rate,
                &seg_model,
                model_info,
                inference_deadline.options(),
            )?;
        }
    }

    tracing::info!(
        segments = speech_segments.len(),
        "speech segmentation complete"
    );

    // Step 2: Extract speaker embeddings and cluster.
    //
    // We replace pyannote-rs's EmbeddingManager with our own clustering
    // that maintains running-average speaker templates. EmbeddingManager
    // only stores the first segment's embedding per speaker, which causes
    // over-segmentation (one person → multiple speakers) as the reference
    // embedding becomes unrepresentative over time.
    let mut extractor = BoundedEmbeddingExtractor::new(&emb_model, emb_info)?;
    let threshold = config.diarization.threshold;

    // Merge adjacent speech segments that are separated by short gaps.
    // The segmentation model often splits continuous speech into many
    // tiny fragments; merging them produces longer, more stable segments
    // for embedding extraction.
    let speech_segments = merge_short_segments(speech_segments, sample_rate);

    tracing::info!(
        segments = speech_segments.len(),
        "speech segments after merge"
    );

    // speaker_templates[i] = (running average embedding, segment count)
    let mut speaker_templates: Vec<(Vec<f32>, usize)> = Vec::new();
    // Per-segment: which speaker index was assigned
    let mut seg_speaker_ids: Vec<usize> = Vec::new();
    // Reliable embeddings parallel to `speech_segments`. Short segments stay
    // `None` and are never allowed to vote on an identity decision.
    let mut seg_embeddings: Vec<Option<(Vec<f32>, f64)>> = Vec::new();

    // Minimum samples for reliable embedding extraction (~1.5s at 16kHz).
    // Shorter segments produce unstable embeddings that corrupt clustering.
    let min_embed_samples = (sample_rate as f64 * 1.5) as usize;

    for seg in &speech_segments {
        cancellation.check()?;
        let embed_end = seg.end_sample.min(
            seg.start_sample
                .saturating_add(sample_rate as usize * MAX_EMBEDDING_SECONDS),
        );
        let seg_i16 = zeroize::Zeroizing::new(
            f32_samples[seg.start_sample..embed_end]
                .iter()
                .map(|&sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect::<Vec<_>>(),
        );

        // Skip too-short segments for clustering; they still appear in the
        // transcript but inherit the nearest speaker label.
        if seg_i16.len() < min_embed_samples {
            seg_speaker_ids.push(usize::MAX); // sentinel: inherit later
            seg_embeddings.push(None);
            continue;
        }

        let raw_embedding = extractor.compute(seg_i16.as_slice(), inference_deadline.options())?;

        // L2-normalize so every segment contributes equally to the
        // average direction, regardless of the model's output magnitude.
        let embedding = l2_normalize(&raw_embedding);
        let embedding_seconds = (embed_end - seg.start_sample) as f64 / sample_rate as f64;
        seg_embeddings.push(Some((embedding.clone(), embedding_seconds)));

        // Find best matching speaker by cosine similarity
        let mut best_id = None;
        let mut best_sim = threshold;
        for (id, (template, _)) in speaker_templates.iter().enumerate() {
            let sim = crate::voice::cosine_similarity(&embedding, template);
            if sim > best_sim {
                best_sim = sim;
                best_id = Some(id);
            }
        }

        let speaker_id = match best_id {
            Some(id) => {
                let (ref mut template, ref mut count) = speaker_templates[id];
                let old_count = *count as f32;
                let new_count = old_count + 1.0;
                for (i, val) in embedding.iter().enumerate() {
                    template[i] = (template[i] * old_count + val) / new_count;
                }
                *count += 1;
                id
            }
            None => {
                if speaker_templates.len() >= MAX_SPEAKER_TEMPLATES {
                    return Err("diarization speaker-template budget exceeded".into());
                }
                let id = speaker_templates.len();
                speaker_templates.push((embedding, 1));
                id
            }
        };

        seg_speaker_ids.push(speaker_id);
    }

    // Merge pass: if two speaker templates are similar enough, merge them.
    // This catches cases where early segments created separate speakers
    // that converged as more data came in.
    //
    // The merge threshold is set to max(threshold - 0.05, 0.3) to avoid
    // merging genuinely different speakers. The 0.3 floor prevents overly
    // aggressive merging when the user sets a low diarization threshold.
    let merge_threshold = (threshold - 0.05).max(0.3);
    let num_templates = speaker_templates.len();
    let mut merge_map: Vec<usize> = (0..num_templates).collect();

    for i in 0..num_templates {
        for j in (i + 1)..num_templates {
            if merge_map[j] != j {
                continue; // already merged
            }
            let ri = merge_map[i]; // canonical id for i
            let sim =
                crate::voice::cosine_similarity(&speaker_templates[ri].0, &speaker_templates[j].0);
            if sim > merge_threshold {
                tracing::info!(
                    from = j,
                    into = ri,
                    similarity = format!("{:.4}", sim),
                    "merging speaker clusters"
                );
                merge_map[j] = ri;
            }
        }
    }

    // Resolve transitive merges (e.g. 3→2→1 becomes 3→1, 2→1).
    // Loop bound prevents infinite loops if merge_map is ever inconsistent.
    for i in 0..num_templates {
        let mut root = merge_map[i];
        let mut steps = 0;
        while merge_map[root] != root && steps < num_templates {
            root = merge_map[root];
            steps += 1;
        }
        merge_map[i] = root;
    }

    // Assign compact labels (SPEAKER_1, SPEAKER_2, ...) to canonical IDs
    let mut canonical_to_label: std::collections::HashMap<usize, String> =
        std::collections::HashMap::new();
    let mut next_label = 1usize;
    for &canonical in &merge_map {
        canonical_to_label.entry(canonical).or_insert_with(|| {
            let label = format!("SPEAKER_{}", next_label);
            next_label += 1;
            label
        });
    }

    // Build segments with merged labels.
    // Segments that were too short for embedding extraction (sentinel usize::MAX)
    // inherit the label of the nearest non-skipped segment.
    let mut segments = Vec::new();

    // First pass: resolve labels for non-skipped segments
    let resolved_labels: Vec<Option<String>> = seg_speaker_ids
        .iter()
        .map(|&raw_id| {
            if raw_id == usize::MAX {
                None
            } else {
                let canonical_id = merge_map[raw_id];
                Some(canonical_to_label[&canonical_id].clone())
            }
        })
        .collect();

    // Forward pass: fill skipped segments by inheriting from the nearest
    // *temporal* neighbor (not acoustic). A short segment between two different
    // speakers gets the label of whichever speaker was most recent, not
    // whichever it sounds like. This is an acceptable tradeoff: extracting
    // embeddings from <1.5s segments produces unreliable results, and temporal
    // proximity is a reasonable heuristic for meeting-style audio.
    let mut last_known_label: Option<String> = None;
    let mut final_labels: Vec<String> = Vec::with_capacity(resolved_labels.len());
    for label in &resolved_labels {
        if let Some(l) = label {
            last_known_label = Some(l.clone());
        }
        final_labels.push(last_known_label.clone().unwrap_or_else(|| "UNKNOWN".into()));
    }
    // Backward pass: fix leading skipped segments (before any known label)
    if let Some(first_known) = resolved_labels.iter().find_map(|l| l.as_ref()) {
        for label in &mut final_labels {
            if label == "UNKNOWN" {
                *label = first_known.clone();
            } else {
                break;
            }
        }
    }

    for (idx, seg) in speech_segments.iter().enumerate() {
        segments.push(SpeakerSegment {
            speaker: final_labels[idx].clone(),
            start: seg.start,
            end: seg.end,
        });
    }

    let mut speaker_embedding_segments: std::collections::HashMap<
        String,
        Vec<SpeakerEmbeddingSegment>,
    > = std::collections::HashMap::new();
    for (idx, evidence) in seg_embeddings.into_iter().enumerate() {
        let Some((embedding, embedding_seconds)) = evidence else {
            continue;
        };
        speaker_embedding_segments
            .entry(final_labels[idx].clone())
            .or_default()
            .push(SpeakerEmbeddingSegment {
                embedding,
                embedding_seconds,
                speech_seconds: speech_segments[idx]
                    .end_sample
                    .saturating_sub(speech_segments[idx].start_sample)
                    as f64
                    / sample_rate as f64,
            });
    }

    // Rebuild final speaker embeddings by weighted-averaging merged templates.
    // Each template is weighted by its segment count so a template built from
    // 50 segments contributes proportionally more than one from 2 segments.
    let mut speaker_embeddings: std::collections::HashMap<String, Vec<f32>> =
        std::collections::HashMap::new();
    let mut speaker_total_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (raw_id, (template, count)) in speaker_templates.iter().enumerate() {
        let canonical_id = merge_map[raw_id];
        let label = canonical_to_label[&canonical_id].clone();
        let entry = speaker_embeddings
            .entry(label.clone())
            .or_insert_with(|| vec![0.0f32; template.len()]);
        for (i, val) in template.iter().enumerate() {
            entry[i] += val * (*count as f32);
        }
        *speaker_total_counts.entry(label).or_insert(0) += count;
    }
    for (label, embedding) in speaker_embeddings.iter_mut() {
        let total = *speaker_total_counts.get(label).unwrap_or(&1) as f32;
        for val in embedding.iter_mut() {
            *val /= total;
        }
    }

    let num_speakers = speaker_embeddings.len();

    tracing::info!(
        raw_clusters = num_templates,
        merged_speakers = num_speakers,
        threshold = threshold,
        merge_threshold = format!("{:.3}", merge_threshold),
        "speaker clustering complete"
    );

    Ok(DiarizationResult {
        segments,
        num_speakers,
        system_dominant_ratio: 0.0,
        voice_dominant_ratio: 0.0,
        degraded_capture: None,
        from_stems: false,
        source_aware: false,
        speaker_embeddings,
        speaker_embedding_segments,
    })
}

/// A detected speech region with sample-level boundaries for embedding extraction.
#[cfg(any(feature = "diarize", test))]
#[derive(Clone)]
struct SpeechSegment {
    start: f64,
    end: f64,
    start_sample: usize,
    end_sample: usize,
}

// Model frames cover slightly less than each independent ten-second input
// window. Anchor frame offsets to that window, rather than carrying its frame
// count forward as an audio clock (which accumulates seconds of drift).
#[cfg(any(feature = "diarize", test))]
struct SpeechFrameTimeline {
    sample_rate: u32,
    total_samples: usize,
    speech_start: Option<usize>,
    next_offset: usize,
    segments: Vec<SpeechSegment>,
}

#[cfg(any(feature = "diarize", test))]
impl SpeechFrameTimeline {
    const FRAME_HOP: usize = 270;
    const FRAME_START: usize = 721;

    fn new(sample_rate: u32, total_samples: usize) -> Self {
        Self {
            sample_rate,
            total_samples,
            speech_start: None,
            next_offset: 0,
            segments: Vec::new(),
        }
    }

    fn push_frame(
        &mut self,
        window_start: usize,
        frame_index: usize,
        speech: bool,
    ) -> Result<(), &'static str> {
        let offset = window_start
            .saturating_add(Self::FRAME_START)
            .saturating_add(frame_index.saturating_mul(Self::FRAME_HOP))
            .min(self.total_samples);
        self.next_offset = offset
            .saturating_add(Self::FRAME_HOP)
            .min(self.total_samples);
        // Padded model input is not recorded audio and cannot start a segment.
        if offset == self.total_samples {
            return Ok(());
        }
        if speech {
            self.speech_start.get_or_insert(offset);
        } else {
            self.end_speech(offset)?;
        }
        Ok(())
    }

    fn end_speech(&mut self, end: usize) -> Result<(), &'static str> {
        if let Some(start) = self.speech_start.take() {
            if end > start {
                if self.segments.len() >= MAX_DIARIZATION_SEGMENTS {
                    return Err("diarization segment budget exceeded");
                }
                self.segments.push(SpeechSegment {
                    start: start as f64 / self.sample_rate as f64,
                    end: end as f64 / self.sample_rate as f64,
                    start_sample: start,
                    end_sample: end,
                });
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<SpeechSegment>, &'static str> {
        self.end_speech(self.next_offset)?;
        Ok(self.segments)
    }
}

/// Merge speech segments that are separated by gaps shorter than `max_gap`
/// and ensure all resulting segments are at least `min_dur` long by absorbing
/// tiny neighbours. This reduces over-fragmentation from the frame-level
/// segmentation model, producing longer segments with more stable embeddings.
#[cfg(feature = "diarize")]
fn merge_short_segments(segments: Vec<SpeechSegment>, sample_rate: u32) -> Vec<SpeechSegment> {
    if segments.is_empty() {
        return segments;
    }

    let max_gap_samples = (sample_rate as f64 * 0.3) as usize; // 300ms gap tolerance
    let min_dur_samples = (sample_rate as f64 * 0.5) as usize; // 0.5s minimum

    // Cap gap tolerance for short segments so they don't absorb across long pauses.
    let max_short_gap_samples = (sample_rate as f64 * 1.0) as usize; // 1s ceiling

    let mut merged: Vec<SpeechSegment> = Vec::new();
    let mut current = segments[0].clone();

    for seg in segments.iter().skip(1) {
        let gap = seg.start_sample.saturating_sub(current.end_sample);
        let current_dur = current.end_sample.saturating_sub(current.start_sample);

        let should_merge = gap <= max_gap_samples
            || (current_dur < min_dur_samples && gap <= max_short_gap_samples);

        if should_merge {
            current.end = seg.end;
            current.end_sample = seg.end_sample;
        } else {
            merged.push(current);
            current = seg.clone();
        }
    }
    merged.push(current);

    tracing::debug!(
        before = segments.len(),
        after = merged.len(),
        "merged adjacent speech segments"
    );

    merged
}

/// Run the segmentation ONNX model directly with properly normalised f32 audio.
///
/// pyannote-rs's `get_segments` has a bug: it casts raw i16 samples to f32
/// (`x as f32`) without dividing by 32768, so the model receives values in
/// [-32768, 32767] instead of the [-1.0, 1.0] it was trained on. This causes
/// the model to classify all frames as non-speech for typical microphone input.
///
/// This function mirrors the same sliding-window logic but feeds the model
/// correctly normalised f32 waveform data.
/// Stage the final, short window of a recording into a reusable scratch buffer,
/// zero-padded out to the model's fixed window length.
///
/// Compiled for the `diarize` feature, which supplies its only production
/// caller, and for tests regardless of features: this is pure buffer handling
/// with no ONNX involvement, so gating it on the feature alone would put it out
/// of reach of the default test command.
#[cfg(feature = "diarize")]
fn segment_legacy_speech(
    samples: &[f32],
    sample_rate: u32,
    model_path: &Path,
    model_info: &DiarizationModelInfo,
    run_options: &ort::session::run_options::RunOptions,
) -> Result<Vec<SpeechSegment>, Box<dyn std::error::Error>> {
    use ndarray::{Array1, ArrayViewD, Axis, IxDyn};
    use ort::session::builder::GraphOptimizationLevel;
    use ort::session::Session;

    let mut session = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(1)?
        .with_inter_threads(1)?
        .commit_from_file(model_path)?;

    let actual_input = session
        .inputs
        .first()
        .map(|input| input.name.as_str())
        .ok_or("segmentation model has no input")?;
    if actual_input != model_info.segmentation_input_name {
        return Err(format!(
            "segmentation model input mismatch: expected '{}', found '{}'",
            model_info.segmentation_input_name, actual_input
        )
        .into());
    }
    if !session
        .outputs
        .iter()
        .any(|output| output.name == model_info.segmentation_output_name)
    {
        return Err(format!(
            "segmentation model output mismatch: expected '{}'",
            model_info.segmentation_output_name
        )
        .into());
    }

    // Both supported exports use a ten-second waveform and the frame geometry
    // encoded by SpeechFrameTimeline (16.875 ms hop, 45 ms initial offset).
    let window_size = (sample_rate as usize) * 10;
    let mut timeline = SpeechFrameTimeline::new(sample_rate, samples.len());
    let mut final_window = zeroize::Zeroizing::new(vec![0.0f32; window_size]);

    for window_start in (0..samples.len()).step_by(window_size) {
        let window_end = (window_start + window_size).min(samples.len());
        let window = if window_end - window_start == window_size {
            &samples[window_start..window_end]
        } else {
            fill_partial_window(
                &mut final_window,
                window_size,
                &samples[window_start..window_end],
            );
            final_window.as_slice()
        };

        let array = Array1::from_iter(window.iter().copied());
        let array = array.view().insert_axis(Axis(0)).insert_axis(Axis(1));

        let inputs = ort::inputs![ort::value::TensorRef::from_array_view(array.into_dyn())
            .map_err(|e| format!("tensor prep: {e:?}"))?];

        let ort_outs = session.run_with_options(inputs, run_options)?;
        let ort_out = ort_outs
            .get(model_info.segmentation_output_name)
            .ok_or("segmentation model output tensor is missing")?;
        let ort_out = ort_out
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("tensor extract: {e:?}"))?;

        let (shape, data) = ort_out;
        let shape_slice: Vec<usize> = (0..shape.len()).map(|i| shape[i] as usize).collect();
        let view = ArrayViewD::<f32>::from_shape(IxDyn(&shape_slice), data)
            .map_err(|e| format!("ndarray shape: {e}"))?;

        let mut frame_index = 0;
        for row in view.outer_iter() {
            for sub_row in row.axis_iter(Axis(0)) {
                let max_index = sub_row
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(i, _)| i)
                    .unwrap_or(0);

                timeline.push_frame(window_start, frame_index, max_index != 0)?;
                frame_index += 1;
            }
        }
    }

    Ok(timeline.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn segmentation_windows_keep_absolute_audio_time_over_many_minutes() {
        const RATE: u32 = 16_000;
        let mut timeline = SpeechFrameTimeline::new(RATE, RATE as usize * 640);
        // The actual exports emit 589 frames for each ten-second input.
        // Treating 589 * 270 samples as ten seconds drifts every window.
        for window in 0..64 {
            for frame in 0..589 {
                timeline
                    .push_frame(window * 160_000, frame, (100..200).contains(&frame))
                    .unwrap();
            }
        }
        let segments = timeline.finish().unwrap();
        assert_eq!(segments.len(), 64);
        for (window, segment) in segments.iter().enumerate() {
            assert_eq!(segment.start_sample, window * 160_000 + 721 + 100 * 270);
            assert_eq!(segment.end_sample, window * 160_000 + 721 + 200 * 270);
            assert_eq!(segment.start, segment.start_sample as f64 / RATE as f64);
        }
    }

    #[test]
    fn segmentation_padding_cannot_extend_or_create_speech_after_the_audio() {
        let total_samples = 163_000;
        let mut timeline = SpeechFrameTimeline::new(16_000, total_samples);
        for frame in 0..589 {
            timeline.push_frame(0, frame, false).unwrap();
        }
        for frame in 0..589 {
            timeline.push_frame(160_000, frame, true).unwrap();
        }
        let segments = timeline.finish().unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_sample, 160_721);
        assert_eq!(segments[0].end_sample, total_samples);
        assert_eq!(segments[0].end, total_samples as f64 / 16_000.0);

        let mut padding_only = SpeechFrameTimeline::new(16_000, 160_100);
        padding_only.push_frame(160_000, 0, true).unwrap();
        assert!(padding_only.finish().unwrap().is_empty());
    }

    #[test]
    fn segmentation_speech_continues_across_a_window_boundary() {
        let mut timeline = SpeechFrameTimeline::new(16_000, 320_000);
        for frame in 0..589 {
            timeline.push_frame(0, frame, frame >= 580).unwrap();
        }
        for frame in 0..589 {
            timeline.push_frame(160_000, frame, frame < 10).unwrap();
        }
        let segments = timeline.finish().unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_sample, 721 + 580 * 270);
        assert_eq!(segments[0].end_sample, 160_721 + 10 * 270);
    }

    // ── merge_short_segments tests ──────────────────────────────

    #[cfg(feature = "diarize")]
    fn make_seg(start_s: f64, end_s: f64, sr: u32) -> SpeechSegment {
        SpeechSegment {
            start: start_s,
            end: end_s,
            start_sample: (start_s * sr as f64) as usize,
            end_sample: (end_s * sr as f64) as usize,
        }
    }

    #[cfg(feature = "diarize")]
    #[test]
    fn merge_short_segments_empty_input() {
        let result = merge_short_segments(vec![], 16000);
        assert!(result.is_empty());
    }

    #[cfg(feature = "diarize")]
    #[test]
    fn merge_short_segments_single_segment() {
        let segs = vec![make_seg(0.0, 2.0, 16000)];
        let result = merge_short_segments(segs, 16000);
        assert_eq!(result.len(), 1);
        assert!((result[0].start - 0.0).abs() < 1e-6);
        assert!((result[0].end - 2.0).abs() < 1e-6);
    }

    #[cfg(feature = "diarize")]
    #[test]
    fn merge_short_segments_merges_small_gaps() {
        // Two segments 200ms apart → should merge (300ms tolerance)
        let segs = vec![make_seg(0.0, 1.0, 16000), make_seg(1.2, 2.0, 16000)];
        let result = merge_short_segments(segs, 16000);
        assert_eq!(result.len(), 1);
        assert!((result[0].end - 2.0).abs() < 1e-6);
    }

    #[cfg(feature = "diarize")]
    #[test]
    fn merge_short_segments_preserves_large_gaps() {
        // Two segments 2s apart → should NOT merge
        let segs = vec![make_seg(0.0, 1.0, 16000), make_seg(3.0, 4.0, 16000)];
        let result = merge_short_segments(segs, 16000);
        assert_eq!(result.len(), 2);
    }

    #[cfg(feature = "diarize")]
    #[test]
    fn merge_short_segments_short_segment_respects_gap_ceiling() {
        // A short segment (0.3s) followed by another 1.5s away.
        // Even though the first is <0.5s (min_dur), the gap exceeds the 1s
        // ceiling so they should NOT merge.
        let segs = vec![make_seg(0.0, 0.3, 16000), make_seg(1.8, 3.0, 16000)];
        let result = merge_short_segments(segs, 16000);
        assert_eq!(
            result.len(),
            2,
            "short segment should not absorb across >1s gap"
        );
    }

    #[cfg(feature = "diarize")]
    #[test]
    fn merge_short_segments_short_segment_merges_within_ceiling() {
        // A short segment (0.3s) followed by another 0.8s away.
        // First is <0.5s and gap is <1s ceiling → should merge.
        let segs = vec![make_seg(0.0, 0.3, 16000), make_seg(1.1, 2.0, 16000)];
        let result = merge_short_segments(segs, 16000);
        assert_eq!(
            result.len(),
            1,
            "short segment should absorb within 1s ceiling"
        );
    }

    #[cfg(feature = "diarize")]
    #[test]
    fn merge_short_segments_all_below_min_duration() {
        // All segments are very short. They should chain-merge until they
        // hit the gap ceiling.
        let segs = vec![
            make_seg(0.0, 0.1, 16000),
            make_seg(0.2, 0.3, 16000),
            make_seg(0.4, 0.5, 16000),
            // 3s gap — exceeds ceiling
            make_seg(3.5, 3.6, 16000),
        ];
        let result = merge_short_segments(segs, 16000);
        assert_eq!(
            result.len(),
            2,
            "chain of short segments should merge, but not across 3s gap"
        );
        assert!((result[0].end - 0.5).abs() < 1e-6);
        assert!((result[1].start - 3.5).abs() < 1e-6);
    }
}
