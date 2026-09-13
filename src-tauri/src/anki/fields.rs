use std::path::Path;

use crate::{
    app_state::sanitize_recording_name,
    app_types::{AnkiSettings, RecentRecording},
};

/// Which piece of a recording a media file holds.
#[derive(Debug, Clone, Copy)]
pub(super) enum MediaPart {
    WholeRecording,
    Line { label: &'static str, start_ms: u64 },
}

impl MediaPart {
    fn suffix(&self) -> String {
        match self {
            MediaPart::WholeRecording => String::new(),
            MediaPart::Line { label, start_ms } => format!("_{label}{start_ms}"),
        }
    }
}

/// Names the file Anki stores, from parts, so the piece that makes it unique cannot be lost.
pub(super) fn anki_media_file_name(source_path: &Path, part: MediaPart, extension: &str) -> String {
    let suffix = part.suffix();
    let title = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(sanitize_recording_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "recording".into())
        .replace(' ', "_")
        .replace(|character: char| character == '[' || character == ']', "_");

    let room = MAX_ANKI_TITLE_CHARS.saturating_sub(suffix.chars().count());
    let capped: String = title.chars().take(room).collect();
    let capped = capped.trim_end_matches('.').trim_end_matches('_');
    let title = if capped.is_empty() { "recording" } else { capped };

    format!("wonder_of_u_{title}{suffix}.{extension}")
}

/// Budget for the title part of an Anki media name.
const MAX_ANKI_TITLE_CHARS: usize = 80;

pub(super) fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\n', "<br>")
}

pub(super) fn user_friendly_anki_error(error: &str, settings: &AnkiSettings) -> String {
    let normalized = error.to_lowercase();
    if normalized.contains("duplicate") {
        return format!(
            "This transcript already exists in the '{}' deck. Wonder of U did not create a duplicate card.",
            settings.deck_name
        );
    }

    if normalized.contains("model") && normalized.contains("not") && normalized.contains("found") {
        return format!(
            "Anki could not find the '{}' note type. Refresh Anki mapping and choose an available note type.",
            settings.note_type
        );
    }

    if normalized.contains("deck") && normalized.contains("not") && normalized.contains("found") {
        return format!(
            "Anki could not find the '{}' deck. Refresh Anki mapping and choose an available deck.",
            settings.deck_name
        );
    }

    if normalized.contains("empty") {
        return format!(
            "Anki rejected the card because the first field on '{}' was blank, and Anki refuses any note that starts empty. Check in Settings that something is mapped to that field and that the name still exists on the note type; if both are right, this card had nothing to put there.",
            settings.note_type
        );
    }

    if normalized.contains("field") {
        return "Anki rejected one of the mapped fields. Refresh Anki mapping and check that every selected field still exists on the note type.".into();
    }

    format!("Anki could not create the card. {error}")
}

pub(super) fn prepend_anki_field_value(
    fields: &mut serde_json::Map<String, serde_json::Value>,
    field_name: &str,
    value: String,
) {
    if field_name.is_empty() {
        return;
    }

    let next_value = fields
        .get(field_name)
        .and_then(|existing| existing.as_str())
        .map(|existing| join_anki_field_parts(&value, existing))
        .unwrap_or(value);
    fields.insert(
        field_name.to_string(),
        serde_json::Value::String(next_value),
    );
}

pub(crate) fn join_anki_field_parts(first: &str, second: &str) -> String {
    let first = first.trim();
    let second = second.trim();
    match (first.is_empty(), second.is_empty()) {
        (true, true) => String::new(),
        (true, false) => second.to_string(),
        (false, true) => first.to_string(),
        (false, false) => format!("{first}<br>{second}"),
    }
}

pub(crate) fn preserve_anki_sound_tags(
    existing_value: Option<&str>,
    new_value: &str,
    fallback_sound_tag: Option<&str>,
) -> String {
    let mut sound_tags = existing_value
        .map(extract_anki_sound_tags)
        .unwrap_or_default();

    if let Some(fallback_sound_tag) = fallback_sound_tag {
        if !new_value.contains(fallback_sound_tag)
            && !sound_tags.iter().any(|tag| tag == fallback_sound_tag)
        {
            sound_tags.push(fallback_sound_tag.to_string());
        }
    }

    let sound_prefix = sound_tags.join(" ");
    join_anki_field_parts(&sound_prefix, new_value)
}

fn extract_anki_sound_tags(value: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut remaining = value;
    while let Some(start) = remaining.find("[sound:") {
        let candidate = &remaining[start..];
        let Some(end) = candidate.find(']') else {
            break;
        };
        let tag = candidate[..=end].to_string();
        if !tags.contains(&tag) {
            tags.push(tag);
        }
        remaining = &candidate[end + 1..];
    }
    tags
}

pub(crate) fn recording_pushed_to_anki_target(
    recording: &RecentRecording,
    settings: &AnkiSettings,
    transcription_language: &str,
) -> bool {
    if !recording.anki_pushes.is_empty() {
        return recording
            .anki_push_for_target(
                transcription_language,
                &settings.deck_name,
                &settings.note_type,
            )
            .is_some();
    }

    recording.anki_note_id.is_some()
        && recording.anki_deck_name.as_deref() == Some(settings.deck_name.as_str())
        && recording.anki_note_type.as_deref() == Some(settings.note_type.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn settings_for(note_type: &str, transcription: &str) -> AnkiSettings {
        AnkiSettings {
            note_type: note_type.to_string(),
            deck_name: "wonder of u".to_string(),
            fields: crate::app_types::AnkiFieldMapping {
                transcription: transcription.to_string(),
                ..Default::default()
            },
            ..AnkiSettings::default()
        }
    }

    #[test]
    fn the_empty_card_error_blames_the_first_field_not_the_transcript_field() {
        let message = user_friendly_anki_error(
            "cannot create note because it is empty",
            &settings_for("Lapis", "Sentence"),
        );
        assert!(message.contains("first field"), "{message}");
        assert!(message.contains("Lapis"), "{message}");
        assert!(
            !message.contains("'Sentence'"),
            "the transcript field is not the cause and must not be accused: {message}"
        );
    }

    #[test]
    fn the_empty_card_error_does_not_prescribe_a_fix_that_is_already_done() {
        // Three routes end in this error and one of them is a card that simply had
        // nothing to put in a field that IS mapped — a recording transcribed from
        // silence. An instruction to map something to the first field is advice to do
        // what has already been done, so the sentence has to offer the settings as
        // something to CHECK and then account for both of them being right.
        let message = user_friendly_anki_error(
            "cannot create note because it is empty",
            &settings_for("Wonder of U Listening", "Sentence"),
        );
        assert!(message.contains("Check in Settings"), "{message}");
        assert!(
            message.contains("nothing to put there"),
            "the card-had-no-content route must be accounted for: {message}"
        );
    }

    #[test]
    fn the_other_anki_errors_still_name_what_they_are_about() {
        let settings = settings_for("Lapis", "Sentence");
        assert!(user_friendly_anki_error("duplicate", &settings).contains("wonder of u"));
        assert!(
            user_friendly_anki_error("model was not found", &settings).contains("Lapis")
        );
        assert!(
            user_friendly_anki_error("deck was not found", &settings).contains("wonder of u")
        );
        assert!(user_friendly_anki_error("kaboom", &settings).contains("kaboom"));
    }

    const SEG: MediaPart = MediaPart::Line {
        label: "seg",
        start_ms: 1000,
    };

    #[test]
    fn media_file_name_strips_brackets_so_sound_tags_parse() {
        // A YouTube import is named `Title [id]`. The brackets must not survive into the
        // media name, or the `[sound:...]` tag truncates at the first one.
        let name = anki_media_file_name(
            &PathBuf::from("Rust in 100 Seconds [abC-1].mp3"),
            SEG,
            "mp3",
        );
        assert!(!name.contains('['), "media name must not contain '[': {name}");
        assert!(!name.contains(']'), "media name must not contain ']': {name}");

        // The resulting tag parses back to exactly one tag whose inner filename equals
        // the stored media name — Anki's first-`]` truncation no longer cuts it short.
        let tag = format!("[sound:{name}]");
        assert_eq!(extract_anki_sound_tags(&tag), vec![tag.clone()]);
        let inner = &tag["[sound:".len()..tag.len() - 1];
        assert_eq!(inner, name);
    }

    #[test]
    fn media_file_name_leaves_plain_names_unchanged() {
        // A mic recording has no brackets and must be untouched apart from the prefix.
        let name = anki_media_file_name(&PathBuf::from("recording_1.mp3"), SEG, "mp3");
        assert_eq!(name, "wonder_of_u_recording_1_seg1000.mp3");
    }

    /// The defect this was rewritten for. An 88-character source stem previously left no room
    /// for the suffix, so every line mined from that video — and the whole recording — resolved
    /// to one identical name and `storeMediaFile` overwrote each with the next. Confirmed on a
    /// real library: one media file serving every card from an entire video.
    #[test]
    fn a_long_title_cannot_cost_a_line_its_identity() {
        let long = PathBuf::from(
            "Connecting with Japanese, one conversation at a time #japanese #learnjapanese.mp3",
        );

        let first = anki_media_file_name(
            &long,
            MediaPart::Line {
                label: "seg",
                start_ms: 1_000,
            },
            "mp3",
        );
        let second = anki_media_file_name(
            &long,
            MediaPart::Line {
                label: "seg",
                start_ms: 2_000,
            },
            "mp3",
        );

        assert_ne!(first, second, "two lines must never share a media name");
        assert!(first.ends_with("_seg1000.mp3"), "suffix intact: {first}");
        assert!(second.ends_with("_seg2000.mp3"), "suffix intact: {second}");
    }

    /// The same title, at the length that used to shave exactly one digit off the timestamp:
    /// `…_seg514420.mp3` reached Anki as `…_seg51442.mp3`.
    #[test]
    fn a_six_digit_timestamp_survives_a_borderline_title() {
        let name = anki_media_file_name(
            &PathBuf::from("Can you survive in Japan Cafe Japanese conversation [#54] [XdmYsZnYXRI].mp3"),
            MediaPart::Line {
                label: "seg",
                start_ms: 514_420,
            },
            "mp3",
        );

        assert!(name.ends_with("_seg514420.mp3"), "not one digit short: {name}");
    }

    /// A line's media and the whole recording's must never collide either — they did once the
    /// title alone filled the budget.
    #[test]
    fn a_line_and_its_whole_recording_get_different_names() {
        let long = PathBuf::from(
            "England fans and MESSI haters FUME after dramatic late comeback in the final.mp3",
        );

        let whole = anki_media_file_name(&long, MediaPart::WholeRecording, "mp3");
        let line = anki_media_file_name(&long, SEG, "mp3");

        assert_ne!(whole, line);
    }

    /// Every name still fits the budget the cap exists to enforce.
    #[test]
    fn the_name_stays_within_the_cap() {
        let name = anki_media_file_name(
            &PathBuf::from("a".repeat(400) + ".mp3"),
            MediaPart::Line {
                label: "seg",
                start_ms: 999_999,
            },
            "mp3",
        );

        let stem = name
            .trim_start_matches("wonder_of_u_")
            .trim_end_matches(".mp3");
        assert!(stem.chars().count() <= MAX_ANKI_TITLE_CHARS, "{name}");
        assert!(name.ends_with("_seg999999.mp3"), "suffix still intact: {name}");
    }
}
