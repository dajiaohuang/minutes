# Meeting summary language and empty sections

Summary generation resolves its language once before adding user notes or
splitting the transcript into provider chunks. An explicit template language
takes precedence, followed by the configured summary language and an explicit
transcription language. A template set to `auto` inherits the meeting's resolved
language instead of resetting individual provider prompts to automatic mode.

When all settings are automatic, a local Whatlang classifier samples the whole
meeting. Short transcripts use one bounded sample; longer transcripts use five
distributed samples of up to 3,000 characters each. Detection requires at least
200 alphabetic characters in a sample, a reliable result with confidence at
least 0.85, and agreement in four of five samples for long meetings. Timestamp,
speaker and source labels are excluded. User notes do not choose the language.
These confidence checks are conservative heuristics, not calibrated guarantees
for every language or acoustic condition. Short, uncertain and balanced
multilingual transcripts retain automatic behavior. Detection runs locally and
adds no provider call. The configuration file is not modified.

The resulting language is passed to all existing provider paths and their
synthesis prompts. This pins the requested output language consistently; it
does not translate or rewrite a model response after generation. A provider
can still fail to follow its instructions.

The structured-output contract represents an empty section as its header with
no bullet items. The parser also recognizes a bounded list of exact English and
French absence placeholders from older/provider-generated responses. These
become empty collections instead of decisions, actions, questions, commitments
or people. Known placeholders are removed from freeform fallback rendering too.

Extraction independently guards these collections, and decision-derived project
extraction checks the original decision text before using an inferred topic.
This prevents a French placeholder from becoming the truncated project name
`Aucune Décision Spécifique N'a`. Existing meeting files are not rewritten.

Negative decisions are substantive information. Statements such as "No changes
to pricing", "No layoffs were approved" and more specific negative decisions
are retained exactly. Unrecognized absence wording is retained rather than
being deleted through a broad keyword or translation heuristic.
