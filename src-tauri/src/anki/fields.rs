use std::path::Path;

use crate::{
    app_state::sanitize_recording_name,
    app_types::{AnkiSettings, RecentRecording},
};

pub(super) fn anki_media_file_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(sanitize_recording_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "recording".into())
        .replace(' ', "_");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("wav");
    format!("wonder_of_u_{stem}.{extension}")
}

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
) -> bool {
    recording.anki_note_id.is_some()
        && recording.anki_deck_name.as_deref() == Some(settings.deck_name.as_str())
        && recording.anki_note_type.as_deref() == Some(settings.note_type.as_str())
}
