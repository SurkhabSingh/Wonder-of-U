use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_types::{
        default_theme_preference, whisper_model_spec, AnkiFieldMapping, AnkiSettings,
        AppPathsState, AppSettings, FeatureSettings, PersistedData, RecentRecording,
        WhisperSettings,
    },
    runtime_assets::{all_managed_model_paths, collect_managed_whisper_cli_candidates},
    RECENT_RECORDINGS_LIMIT,
};

pub(crate) fn build_app_paths<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<AppPathsState, tauri::Error> {
    let data_dir = app.path().app_local_data_dir()?;
    let log_dir = app.path().app_log_dir()?;
    let assets_dir = data_dir.join("assets");

    fs::create_dir_all(&data_dir)?;
    fs::create_dir_all(&log_dir)?;
    fs::create_dir_all(&assets_dir)?;

    Ok(AppPathsState {
        state_file: data_dir.join("state.json"),
        log_file: log_dir.join("wonder-of-u.log"),
        data_dir,
        assets_dir,
    })
}

fn default_output_directory<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, tauri::Error> {
    let base = app
        .path()
        .document_dir()
        .or_else(|_| app.path().download_dir())
        .or_else(|_| app.path().home_dir())?;

    Ok(base.join("Wonder of U Recordings"))
}

fn default_asset_directory(paths: &AppPathsState) -> PathBuf {
    paths.assets_dir.clone()
}

pub(crate) fn normalize_theme_preference(theme: &str) -> &str {
    match theme.trim() {
        "light" => "light",
        "dark" => "dark",
        _ => "system",
    }
}

fn default_settings<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPathsState,
) -> Result<AppSettings, tauri::Error> {
    Ok(AppSettings {
        output_directory: default_output_directory(app)?.display().to_string(),
        asset_directory: default_asset_directory(paths).display().to_string(),
        whisper: WhisperSettings::default(),
        anki: AnkiSettings::default(),
        features: FeatureSettings::default(),
        theme: default_theme_preference(),
        launch_at_login: false,
        start_minimized: false,
    })
}

pub(crate) fn load_persisted_data<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPathsState,
) -> Result<PersistedData, tauri::Error> {
    let defaults = default_settings(app, paths)?;

    let mut state = match fs::read_to_string(&paths.state_file) {
        Ok(raw) => serde_json::from_str::<PersistedData>(&raw).unwrap_or(PersistedData {
            settings: defaults.clone(),
            recent_recordings: Vec::new(),
            untitled_counter: 1,
        }),
        Err(_) => PersistedData {
            settings: defaults.clone(),
            recent_recordings: Vec::new(),
            untitled_counter: 1,
        },
    };

    state.settings = normalize_settings(app, paths, state.settings)?;
    state.recent_recordings.truncate(RECENT_RECORDINGS_LIMIT);
    normalize_recent_recording_languages(&mut state.recent_recordings);
    if state.untitled_counter == 0 {
        state.untitled_counter = 1;
    }

    Ok(state)
}

fn normalize_recent_recording_languages(recordings: &mut [RecentRecording]) {
    for recording in recordings {
        if recording.transcript_language.is_some() {
            continue;
        }

        let Some(transcript_path) = recording.transcript_path.as_deref() else {
            continue;
        };

        recording.transcript_language =
            derive_transcript_language_from_path(Path::new(transcript_path), "auto");
    }
}

pub(crate) fn derive_transcript_language_from_path(
    transcript_path: &Path,
    requested_language: &str,
) -> Option<String> {
    if let Some(language) = normalize_transcript_language_code(requested_language) {
        return Some(language);
    }

    let transcript = fs::read_to_string(transcript_path).ok()?;
    if transcript_looks_japanese(&transcript) {
        Some("ja".into())
    } else {
        None
    }
}

fn normalize_transcript_language_code(language: &str) -> Option<String> {
    let normalized = language.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "auto" => None,
        "ja" | "japanese" => Some("ja".into()),
        _ => Some(normalized),
    }
}

pub(crate) fn transcript_looks_japanese(transcript: &str) -> bool {
    transcript.chars().any(is_japanese_kana)
}

fn is_japanese_kana(character: char) -> bool {
    matches!(
        character as u32,
        0x3040..=0x30ff | 0x31f0..=0x31ff | 0xff66..=0xff9f
    )
}

pub(crate) fn is_japanese_transcript_language(language: Option<&str>) -> bool {
    language
        .map(normalize_transcript_language_code)
        .is_some_and(|normalized| normalized.as_deref() == Some("ja"))
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
        },
        theme: theme.into(),
        launch_at_login: settings.launch_at_login,
        start_minimized: settings.start_minimized,
    })
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

pub(crate) fn write_persisted_data<R: Runtime>(
    app: &AppHandle<R>,
    state: &PersistedData,
) -> Result<(), String> {
    let paths = app.state::<AppPathsState>().inner().clone();
    let serialized = serde_json::to_string_pretty(state).map_err(|error| error.to_string())?;
    fs::write(&paths.state_file, serialized).map_err(|error| error.to_string())
}
