//! Local powerset speaker tracks. Local channel numbers are never global IDs.
use std::collections::BTreeSet;
use std::ops::Range;

pub(super) const FRAME_HOP: usize = 270;
pub(super) const FRAME_START: usize = 721;

#[derive(Debug)]
pub(super) struct LocalSpeakerTrack {
    pub window: usize,
    pub regions: Vec<Range<usize>>,
    // Only frames with exactly this speaker active may identify a voice.
    pub clean_regions: Vec<Range<usize>>,
}

#[derive(Clone)]
pub(super) struct Observation {
    pub embedding: Vec<f32>,
    pub window: usize,
    pub seconds: f64,
}

pub(super) struct Clustered {
    pub roots: Vec<usize>,
    pub embeddings: Vec<Vec<f32>>,
}

#[derive(PartialEq)]
struct Pair {
    similarity: f32,
    left: usize,
    right: usize,
    left_revision: u32,
    right_revision: u32,
}
impl Eq for Pair {}
impl PartialOrd for Pair {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Pair {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.similarity
            .total_cmp(&other.similarity)
            .then(other.left.cmp(&self.left))
            .then(other.right.cmp(&self.right))
            .then(self.left_revision.cmp(&other.left_revision))
            .then(self.right_revision.cmp(&other.right_revision))
    }
}

/// Offline centroid agglomeration: merge the strongest compatible pair first,
/// rather than committing each arriving observation to an early template.
/// Maximum observations follows the two-hour, ten-second, three-local-slot
/// input contract. The pair queue is bounded, fallibly allocated and checked
/// for cancellation throughout construction and merging.
pub(super) fn cluster_embeddings(
    observations: &[Observation],
    threshold: f32,
    max_clusters: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Clustered, &'static str> {
    const MAX_OBSERVATIONS: usize = 2160;
    const MAX_PAIRS: usize = MAX_OBSERVATIONS * MAX_OBSERVATIONS;
    let count = observations.len();
    if count > MAX_OBSERVATIONS {
        return Err("diarization observation budget exceeded");
    }
    if !threshold.is_finite()
        || observations.iter().any(|o| {
            o.embedding.is_empty()
                || !o.seconds.is_finite()
                || o.seconds <= 0.0
                || o.embedding.iter().any(|v| !v.is_finite())
                || o.embedding.len() != observations[0].embedding.len()
        })
    {
        return Err("invalid diarization clustering evidence");
    }
    let mut roots: Vec<_> = (0..count).collect();
    let mut active = vec![true; count];
    let mut revisions = vec![0_u32; count];
    let mut embeddings: Vec<_> = observations.iter().map(|o| o.embedding.clone()).collect();
    let mut weights: Vec<_> = observations.iter().map(|o| o.seconds).collect();
    let mut windows: Vec<BTreeSet<usize>> = observations
        .iter()
        .map(|o| BTreeSet::from([o.window]))
        .collect();
    let mut pairs = std::collections::BinaryHeap::new();
    let queue =
        |pairs: &mut std::collections::BinaryHeap<Pair>, pair: Pair| -> Result<(), &'static str> {
            if pair.similarity <= threshold {
                return Ok(());
            }
            if pairs.len() >= MAX_PAIRS {
                return Err("diarization pair budget exceeded");
            }
            if pairs.len() == pairs.capacity() {
                pairs
                    .try_reserve(1024.min(MAX_PAIRS - pairs.len()))
                    .map_err(|_| "diarization pair allocation refused")?;
            }
            pairs.push(pair);
            Ok(())
        };
    for i in 0..count {
        if cancelled() {
            return Err("diarization cancelled");
        }
        for j in i + 1..count {
            if observations[i].window == observations[j].window {
                continue;
            }
            queue(
                &mut pairs,
                Pair {
                    similarity: crate::voice::cosine_similarity(&embeddings[i], &embeddings[j]),
                    left: i,
                    right: j,
                    left_revision: 0,
                    right_revision: 0,
                },
            )?;
        }
    }
    while let Some(pair) = pairs.pop() {
        if cancelled() {
            return Err("diarization cancelled");
        }
        let (i, j) = (pair.left, pair.right);
        if !active[i]
            || !active[j]
            || pair.left_revision != revisions[i]
            || pair.right_revision != revisions[j]
            || !windows[i].is_disjoint(&windows[j])
        {
            continue;
        }
        let total = weights[i] + weights[j];
        for dimension in 0..embeddings[i].len() {
            embeddings[i][dimension] = ((embeddings[i][dimension] as f64 * weights[i]
                + embeddings[j][dimension] as f64 * weights[j])
                / total) as f32;
        }
        weights[i] = total;
        active[j] = false;
        revisions[i] += 1;
        let other = std::mem::take(&mut windows[j]);
        windows[i].extend(other);
        for root in &mut roots {
            if *root == j {
                *root = i;
            }
        }
        for k in 0..count {
            if k == i || !active[k] || !windows[i].is_disjoint(&windows[k]) {
                continue;
            }
            let (left, right) = (i.min(k), i.max(k));
            queue(
                &mut pairs,
                Pair {
                    similarity: crate::voice::cosine_similarity(
                        &embeddings[left],
                        &embeddings[right],
                    ),
                    left,
                    right,
                    left_revision: revisions[left],
                    right_revision: revisions[right],
                },
            )?;
        }
    }
    if active.iter().filter(|&&value| value).count() > max_clusters {
        return Err("diarization speaker-template budget exceeded");
    }
    Ok(Clustered { roots, embeddings })
}

fn append_range(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
    if range.is_empty() {
        return;
    }
    if let Some(last) = ranges.last_mut() {
        if last.end == range.start {
            last.end = range.end;
            return;
        }
    }
    ranges.push(range);
}

/// The seven classes encode silence, three singletons, then three pairs.
/// Retain both speakers during overlap, but never feed mixed frames to an
/// embedding. The clock resets to the real start of each model input window.
pub(super) fn decode_window(
    window: usize,
    total_samples: usize,
    classes: &[usize],
    remaining_regions: usize,
) -> Result<Vec<LocalSpeakerTrack>, &'static str> {
    const MASKS: [u8; 7] = [0, 1, 2, 4, 3, 5, 6];
    let mut tracks: Vec<_> = (0..3)
        .map(|_| LocalSpeakerTrack {
            window,
            regions: Vec::new(),
            clean_regions: Vec::new(),
        })
        .collect();
    let mut regions = 0;
    for (index, &class) in classes.iter().enumerate() {
        let mask = *MASKS
            .get(class)
            .ok_or("invalid diarization powerset class")?;
        let start = window
            .saturating_add(FRAME_START)
            .saturating_add(index.saturating_mul(FRAME_HOP))
            .min(total_samples);
        let end = start.saturating_add(FRAME_HOP).min(total_samples);
        for (speaker, track) in tracks.iter_mut().enumerate() {
            if mask & (1 << speaker) == 0 || start == end {
                continue;
            }
            let before = track.regions.len() + track.clean_regions.len();
            append_range(&mut track.regions, start..end);
            if mask.count_ones() == 1 {
                append_range(&mut track.clean_regions, start..end);
            }
            regions += track.regions.len() + track.clean_regions.len() - before;
            if regions > remaining_regions {
                return Err("diarization segment budget exceeded");
            }
        }
    }
    tracks.retain(|track| !track.regions.is_empty());
    Ok(tracks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(embedding: &[f32], window: usize) -> Observation {
        Observation {
            embedding: embedding.to_vec(),
            window,
            seconds: 3.0,
        }
    }

    #[test]
    fn local_speakers_cannot_collapse_even_with_identical_embeddings() {
        let result = cluster_embeddings(
            &[observation(&[1., 0.], 0), observation(&[1., 0.], 0)],
            0.4,
            64,
            || false,
        )
        .unwrap();
        assert_ne!(result.roots[0], result.roots[1]);
        assert_eq!(result.embeddings[0], vec![1., 0.]);
    }

    #[test]
    fn local_channel_permutation_does_not_permute_global_identity() {
        let result = cluster_embeddings(
            &[
                observation(&[1., 0.], 0),
                observation(&[0., 1.], 0),
                observation(&[0., 1.], 160_000),
                observation(&[1., 0.], 160_000),
            ],
            0.4,
            64,
            || false,
        )
        .unwrap();
        assert_eq!(result.roots[0], result.roots[3]);
        assert_eq!(result.roots[1], result.roots[2]);
        assert_ne!(result.roots[0], result.roots[1]);
    }

    #[test]
    fn transitive_merge_retains_all_cannot_link_windows() {
        let result = cluster_embeddings(
            &[
                observation(&[1., 0.], 0),
                observation(&[1., 0.], 1),
                observation(&[1., 0.], 1),
            ],
            0.3,
            64,
            || false,
        )
        .unwrap();
        assert_ne!(result.roots[1], result.roots[2]);
    }

    #[test]
    fn global_best_pair_prevents_an_early_ambiguous_merge() {
        let input = [
            observation(&[0.8, 0.6], 0),
            observation(&[1., 0.], 1),
            observation(&[0.8, 0.6], 1),
        ];
        let result = cluster_embeddings(&input, 0.4, 64, || false).unwrap();
        assert_eq!(result.roots[0], result.roots[2]);
        assert_ne!(result.roots[0], result.roots[1]);
    }

    #[test]
    fn clustering_is_cancelled_bounded_and_rejects_nonfinite_evidence() {
        let input = [observation(&[1., 0.], 0), observation(&[0., 1.], 0)];
        assert!(cluster_embeddings(&input, 0.4, 1, || false).is_err());
        assert!(cluster_embeddings(&input, 0.4, 64, || true).is_err());
        assert!(cluster_embeddings(&[observation(&[f32::NAN], 0)], 0.4, 64, || false).is_err());
        assert!(cluster_embeddings(&vec![observation(&[1.], 0); 2161], 0.4, 64, || false).is_err());
        assert!(cluster_embeddings(&[], 0.4, 64, || false)
            .unwrap()
            .roots
            .is_empty());
    }

    #[test]
    fn adjacent_speaker_change_survives_without_silence() {
        let tracks = decode_window(0, 160_000, &[1, 1, 2, 2], 100).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].regions, vec![721..1261]);
        assert_eq!(tracks[1].regions, vec![1261..1801]);
    }

    #[test]
    fn all_overlap_pairs_keep_both_tracks_and_no_identity_audio() {
        for class in 4..7 {
            let tracks = decode_window(0, 160_000, &[class], 100).unwrap();
            assert_eq!(tracks.len(), 2);
            assert!(tracks.iter().all(|track| track.regions.len() == 1
                && track.regions[0] == (721..991)
                && track.clean_regions.is_empty()));
        }
    }

    #[test]
    fn overlap_is_removed_only_from_embedding_regions() {
        let tracks = decode_window(0, 160_000, &[1, 4, 1, 2], 100).unwrap();
        assert_eq!(tracks[0].regions, vec![721..1531]);
        assert_eq!(tracks[0].clean_regions, vec![721..991, 1261..1531]);
        assert_eq!(tracks[1].regions, vec![991..1261, 1531..1801]);
        assert_eq!(tracks[1].clean_regions, vec![1531..1801]);
    }

    #[test]
    fn ten_minute_clock_and_padded_tail_use_audio_coordinates() {
        for window in (0..10_240_000).step_by(160_000) {
            let tracks = decode_window(window, 10_240_000, &[1; 589], 10).unwrap();
            assert_eq!(tracks[0].window, window);
            assert_eq!(tracks[0].regions[0].start, window + FRAME_START);
            assert!(tracks[0].regions[0].end <= window + 160_000);
        }
        let tracks = decode_window(160_000, 161_000, &[1; 589], 10).unwrap();
        assert_eq!(tracks[0].regions, vec![160_721..161_000]);
        assert!(decode_window(160_000, 160_100, &[1; 589], 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn rejects_bad_classes_and_fragment_budget() {
        assert!(decode_window(0, 160_000, &[7], 100).is_err());
        assert!(decode_window(0, 160_000, &[1, 0, 1], 3).is_err());
    }
}
