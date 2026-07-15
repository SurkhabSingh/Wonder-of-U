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
        default_translation_provider, whisper_model_spec, AnkiFieldMapping, AnkiSettings,
        AppPathsState, AppSettings, FeatureSettings, PersistedData, TranslationSettings,
        WhisperSettings,
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
        },
        theme: theme.into(),
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

pub(crate) fn sanitize_recording_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    trimmed
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('.')
        .trim()
        .to_string()
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
