use std::path::{Path, PathBuf};

use tauri::{AppHandle, Runtime};

mod history;
mod persistence;

pub(crate) use history::{
    derive_transcript_language_from_path, is_japanese_transcript_language,
    reconcile_recording_history, transcript_looks_japanese,
};
pub(crate) use persistence::{build_app_paths, load_persisted_data, write_persisted_data};
use persistence::{default_asset_directory, default_output_directory};

use crate::{
    app_types::{
        default_translation_provider, default_translation_target_language, whisper_model_spec,
        AnkiFieldMapping, AnkiSettings, AppPathsState, AppSettings, FeatureSettings, PersistedData,
        TranslationSettings, WhisperSettings,
    },
    runtime_assets::{all_managed_model_paths, collect_managed_whisper_cli_candidates},
};

pub(crate) fn normalize_theme_preference(theme: &str) -> &str {
    match theme.trim() {
        "light" => "light",
        "dark" => "dark",
        _ => "system",
    }
}

/// Clamp the recording-indicator anchor to the six placements the overlay knows
/// how to position, so a hand-edited or stale `state.json` can never leave the
/// toast off-screen. Anything unrecognized falls back to the centered top edge.
pub(crate) fn normalize_indicator_position(position: &str) -> &str {
    match position.trim() {
        "top-left" => "top-left",
        "top-right" => "top-right",
        "bottom-left" => "bottom-left",
        "bottom-center" => "bottom-center",
        "bottom-right" => "bottom-right",
        _ => "top-center",
    }
}

pub(crate) fn normalize_settings<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPathsState,
    settings: AppSettings,
) -> Result<AppSettings, tauri::Error> {
    let output_directory =
        normalize_directory_input(&settings.output_directory, &default_output_directory(app)?);
    let asset_directory =
        normalize_directory_input(&settings.asset_directory, &default_asset_directory(paths));
    let language = settings.whisper.language.trim();
    let runtime_version = sanitize_runtime_version(&settings.whisper.runtime_version);
    let model_choice = whisper_model_spec(settings.whisper.model_choice.trim()).id;
    let managed_cli_candidates =
        collect_managed_whisper_cli_candidates(&asset_directory, &runtime_version);
    let managed_model_candidates = all_managed_model_paths(&asset_directory);
    let cli_path = settings.whisper.cli_path.trim();
    let model_path = settings.whisper.model_path.trim();
    let theme = normalize_theme_preference(&settings.theme);
    let indicator_position = normalize_indicator_position(&settings.indicator_position);

    let normalized_cli_path = if cli_path.is_empty() {
        String::new()
    } else {
        let candidate = PathBuf::from(cli_path);
        if managed_cli_candidates
            .iter()
            .any(|managed| managed == &candidate)
        {
            String::new()
        } else {
            cli_path.to_string()
        }
    };
    let normalized_model_path = if model_path.is_empty() {
        String::new()
    } else {
        let candidate = PathBuf::from(model_path);
        if managed_model_candidates
            .iter()
            .any(|managed| managed == &candidate)
        {
            String::new()
        } else {
            model_path.to_string()
        }
    };

    Ok(AppSettings {
        output_directory: output_directory.display().to_string(),
        asset_directory: asset_directory.display().to_string(),
        whisper: WhisperSettings {
            cli_path: normalized_cli_path,
            model_path: normalized_model_path,
            runtime_version,
            model_choice: model_choice.to_string(),
            language: if language.is_empty() {
                "auto".into()
            } else {
                language.to_string()
            },
        },
        anki: AnkiSettings {
            deck_name: settings.anki.deck_name.trim().to_string(),
            note_type: settings.anki.note_type.trim().to_string(),
            fields: AnkiFieldMapping {
                transcription: settings.anki.fields.transcription.trim().to_string(),
                furigana: settings.anki.fields.furigana.trim().to_string(),
                audio: settings.anki.fields.audio.trim().to_string(),
                translation: settings.anki.fields.translation.trim().to_string(),
                source_path: settings.anki.fields.source_path.trim().to_string(),
                created_at: settings.anki.fields.created_at.trim().to_string(),
                source_url: settings.anki.fields.source_url.trim().to_string(),
                title: settings.anki.fields.title.trim().to_string(),
                position: settings.anki.fields.position.trim().to_string(),
            },
        },
        features: FeatureSettings {
            transcription: settings.features.transcription,
            delete_local_audio_after_anki_push: settings
                .features
                .delete_local_audio_after_anki_push,
            allow_mp3_conversion: settings.features.allow_mp3_conversion,
            auto_add_furigana_after_anki_push: settings.features.auto_add_furigana_after_anki_push,
            translate_after_transcription: settings.features.translate_after_transcription,
        },
        translation: TranslationSettings {
            provider: normalize_translation_provider(&settings.translation.provider),
            target_language: normalize_translation_target_language(
                &settings.translation.target_language,
            ),
        },
        theme: theme.into(),
        indicator_position: indicator_position.into(),
        launch_at_login: settings.launch_at_login,
        start_minimized: settings.start_minimized,
    })
}

/// Keep the persisted provider to the ids the extension actually routes on,
/// falling back to the default for anything empty or unrecognized.
fn normalize_translation_provider(provider: &str) -> String {
    match provider.trim() {
        "google-translate" => "google-translate".to_string(),
        "deepl" => "deepl".to_string(),
        _ => default_translation_provider(),
    }
}

/// Force the target language into the one shape the extension's page providers can
/// consume: this code is interpolated straight into a provider URL — Google's
/// `?sl=..&tl=<code>` query and DeepL's `#<src>/<tgt>/<text>` fragment — so a
/// stored `"JA"` or `" en "` loads a page that translates into nothing. Trimmed
/// lowercase, falling back to English when empty.
///
/// Deliberately not validated against a language list: the UI owns which codes it
/// offers, Rust owns the format. Same split as `whisper.language`, which is only
/// normalized here as empty -> `"auto"` while the TS `LANGUAGE_OPTIONS` drives the
/// picker.
fn normalize_translation_target_language(language: &str) -> String {
    let normalized = language.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        default_translation_target_language()
    } else {
        normalized
    }
}

fn normalize_directory_input(input: &str, fallback: &Path) -> PathBuf {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return fallback.to_path_buf();
    }

    let candidate = PathBuf::from(trimmed);
    if candidate.is_absolute() {
        candidate
    } else {
        fallback.join(candidate)
    }
}

/// Longest name we let through, in characters.
///
/// This is only ever the START of a path: callers append `_{recording_id}`,
/// `.transcript.txt`, `.translation.en.txt`, and a `_N` uniqueness suffix on top
/// of a recordings folder the user chose and that may itself be deep. Windows
/// caps a single component at 255 UTF-16 units and a whole path at 260 unless the
/// long-path opt-in is active, so an uncapped `requested_name` from the start
/// command builds a path that simply cannot be created. 80 leaves room for every
/// suffix above and still fits a real transcript title.
const MAX_RECORDING_NAME_CHARS: usize = 80;

/// Names Windows resolves to a DOS device instead of a file, with or without an
/// extension — `NUL.wav` is the device just as `NUL` is. A recording named after
/// one either fails to create or writes into the device and is gone, so the name
/// is pushed out of the reserved namespace rather than rejected.
const WINDOWS_RESERVED_DEVICE_NAMES: [&str; 24] = [
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Matches on the segment before the first dot, and trims it: Windows ignores
/// trailing spaces when resolving a device, so `NUL .wav` is `NUL` too.
fn is_windows_reserved_device_name(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or(name).trim();
    WINDOWS_RESERVED_DEVICE_NAMES
        .iter()
        .any(|reserved| base.eq_ignore_ascii_case(reserved))
}

/// The single chokepoint every filename in the app passes through: recording
/// stems, transcript-derived titles, imported and YouTube titles, and Anki media
/// names.
///
/// `&`, `^`, `%`, `(`, `)` and `!` are stripped alongside the characters Windows
/// forbids outright. They are legal in a filename, but the name is attacker-chosen
/// — a recording is named after the transcript of whatever system audio was
/// playing — and shell metacharacters in a filename have exactly one use. This is
/// defense in depth only: nothing downstream may rely on it, and nothing does
/// (`play_recording_inner` hands the path to Win32 as data, and every other
/// spawn passes argv directly).
pub(crate) fn sanitize_recording_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let collapsed = trimmed
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            '&' | '^' | '%' | '(' | ')' | '!' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // Cap before the final trim: truncation can expose a trailing space or dot
    // that was interior a moment ago, and Windows drops both silently.
    let capped = collapsed
        .chars()
        .take(MAX_RECORDING_NAME_CHARS)
        .collect::<String>();
    let cleaned = capped.trim_end_matches('.').trim();

    if is_windows_reserved_device_name(cleaned) {
        return format!("_{cleaned}");
    }

    cleaned.to_string()
}

pub(crate) fn sanitize_runtime_version(version: &str) -> String {
    let trimmed = version.trim();
    if trimmed.is_empty()
        || !trimmed.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        return crate::app_types::default_whisper_runtime_version();
    }

    trimmed.to_string()
}

pub(crate) fn next_recording_stem(
    state: &mut PersistedData,
    requested_name: Option<&str>,
) -> String {
    let requested = requested_name
        .map(sanitize_recording_name)
        .unwrap_or_default();

    if !requested.is_empty() {
        return requested;
    }

    let stem = format!("recording_{}", state.untitled_counter.max(1));
    state.untitled_counter = state.untitled_counter.max(1) + 1;
    stem
}

pub(crate) fn unique_wav_path(directory: &Path, file_stem: &str) -> PathBuf {
    let sanitized_stem = if file_stem.is_empty() {
        "recording".to_string()
    } else {
        file_stem.to_string()
    };

    let mut attempt = 0usize;
    loop {
        let candidate = if attempt == 0 {
            directory.join(format!("{sanitized_stem}.wav"))
        } else {
            directory.join(format!("{sanitized_stem}_{attempt}.wav"))
        };

        if !candidate.exists() {
            return candidate;
        }

        attempt += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_translation_target_language;

    #[test]
    fn translation_target_language_is_normalized_to_a_url_safe_code() {
        // The extension interpolates this straight into a provider URL, so the
        // shape matters more than the value.
        assert_eq!(normalize_translation_target_language("EN-US"), "en-us");
        assert_eq!(normalize_translation_target_language("  Ja  "), "ja");
        assert_eq!(normalize_translation_target_language("es"), "es");
    }

    #[test]
    fn an_empty_translation_target_language_falls_back_to_english() {
        assert_eq!(normalize_translation_target_language(""), "en");
        assert_eq!(normalize_translation_target_language("   "), "en");
    }

    #[test]
    fn an_unknown_translation_target_language_is_kept() {
        // The UI owns which codes it offers; Rust only owns the format. Rejecting
        // anything not on a Rust-side list would mean a new UI language silently
        // translating to English.
        assert_eq!(normalize_translation_target_language("zh-Hans"), "zh-hans");
    }
}
