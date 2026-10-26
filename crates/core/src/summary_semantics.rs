//! Conservative meeting-level language selection and structured empty sections.

/// Recognize exact section-absence sentinels, not negative decisions. A sentence
/// such as "No layoffs were approved" is substantive and must survive unchanged.
pub(crate) fn is_absence_item(section: &str, item: &str) -> bool {
    let normalized = item.replace('’', "'").to_lowercase();
    let normalized = normalized
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, '.' | '*' | '_' | '`'))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if matches!(
        normalized.as_str(),
        "none" | "n/a" | "not applicable" | "aucun" | "aucune"
    ) {
        return true;
    }
    let phrases: &[&str] = match section {
        "decisions" => &[
            "no decisions",
            "no decisions made",
            "no decisions were made",
            "no specific decisions were made",
            "no decisions were identified",
            "aucune décision",
            "aucune décision n'a été prise",
            "aucune décision spécifique n'a été prise",
        ],
        "action_items" => &[
            "no action items",
            "no action items were identified",
            "no action items identified",
            "aucune action à entreprendre",
            "aucune action n'a été identifiée",
        ],
        "open_questions" => &[
            "no open questions",
            "no open questions were identified",
            "aucune question ouverte",
            "aucune question en suspens",
        ],
        "commitments" => &[
            "no commitments",
            "no commitments were made",
            "aucun engagement",
        ],
        "participants" => &["no participants identified", "aucun participant identifié"],
        _ => &[],
    };
    phrases.contains(&normalized.as_str())
}

fn language_of_sample(text: &str) -> Option<whatlang::Lang> {
    let clean = text
        .lines()
        .map(|line| {
            let mut line = line.trim();
            // Ignore timestamp/speaker/source prefixes, including a second source
            // marker after a diarized timestamp. These labels are not meeting speech.
            for _ in 0..2 {
                if let Some((_, rest)) = line.strip_prefix('[').and_then(|s| s.split_once(']')) {
                    line = rest.trim_start();
                }
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");
    if clean.chars().filter(|c| c.is_alphabetic()).count() < 200 {
        return None;
    }
    let info = whatlang::detect(&clean)?;
    (info.is_reliable() && info.confidence() >= 0.85).then(|| info.lang())
}

/// Select once from the transcript alone, before notes are added or any
/// provider chunks are made. Bounded samples cover the whole meeting. Short,
/// unreliable and balanced multilingual input abstains rather than imposing
/// an arbitrary language. Explicit settings always take precedence at caller.
pub(crate) fn meeting_language(transcript: &str) -> Option<String> {
    if transcript.len() <= 4096 {
        return language_of_sample(transcript).map(|language| language.eng_name().to_string());
    }
    let mut votes = Vec::new();
    for index in 0..5 {
        let mut start = transcript.len() * index / 5;
        let mut end = transcript.len() * (index + 1) / 5;
        while !transcript.is_char_boundary(start) {
            start += 1;
        }
        while !transcript.is_char_boundary(end) {
            end -= 1;
        }
        // Cap each sample without allocating a copy of the full transcript.
        let section = &transcript[start..end];
        let bounded_end = section
            .char_indices()
            .nth(3000)
            .map(|(i, _)| i)
            .unwrap_or(section.len());
        if let Some(language) = language_of_sample(&section[..bounded_end]) {
            votes.push(language);
        }
    }
    votes
        .iter()
        .find(|language| votes.iter().filter(|other| *other == *language).count() >= 4)
        .map(|language| language.eng_name().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    const EN: &str = "The team reviewed the annual billing proposal and discussed how the product should work for customers. We decided to keep the existing pricing and prepare the follow up document next Tuesday. The engineering team will investigate recording quality before the next release. ";
    const FR: &str = "L'équipe a examiné la proposition de facturation annuelle et discuté du fonctionnement du produit pour les clients. Nous avons décidé de conserver les tarifs et de préparer le document de suivi mardi prochain. Les ingénieurs vont examiner la qualité des enregistrements avant la prochaine version. ";

    #[test]
    fn detects_meeting_language_from_speech_without_timestamp_or_source_labels() {
        assert_eq!(
            meeting_language(&format!("[SPEAKER_0 0:01] [MICROPHONE] {EN}{EN}")),
            Some("English".into())
        );
        assert_eq!(
            meeting_language(&format!("[SYSTEM 0:01] {FR}{FR}")),
            Some("French".into())
        );
    }

    #[test]
    fn whole_meeting_samples_agree_for_long_transcripts() {
        assert_eq!(meeting_language(&EN.repeat(100)), Some("English".into()));
        assert_eq!(meeting_language(&FR.repeat(100)), Some("French".into()));
        assert_eq!(
            meeting_language(&format!("{}{}", EN.repeat(50), FR.repeat(50))),
            None
        );
        assert_eq!(meeting_language("Mat: yes. Sam: agreed."), None);
    }

    #[test]
    fn strips_only_exact_absence_sentinels_in_their_own_section() {
        assert!(is_absence_item("decisions", "No decisions were made."));
        assert!(is_absence_item(
            "decisions",
            "Aucune décision spécifique n’a été prise."
        ));
        assert!(is_absence_item("action_items", "**None.**"));
        for real_decision in [
            "No layoffs were approved.",
            "No changes to pricing.",
            "No decisions were made about Acme; we approved Beta.",
            "Aucune réduction des tarifs n'a été approuvée.",
        ] {
            assert!(
                !is_absence_item("decisions", real_decision),
                "{real_decision}"
            );
        }
        assert!(!is_absence_item("key_points", "No decisions were made."));
    }
}
