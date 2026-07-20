use tauri::AppHandle;

use crate::{
    anki::{
        add_furigana_to_anki_inner, create_recommended_note_type_inner, load_anki_catalog_inner,
        mine_segment_to_anki_inner, push_recordings_to_anki_deck_inner,
        push_recordings_to_anki_inner,
    },
    app_runtime::build_app_bootstrap,
    app_types::{
        AnkiCatalog, AppBootstrap, AppSettings, RecordingBatchResult, RecordingTexts,
        WhisperAssetUpdateResult,
    },
    asset_downloads::{
        cancel_whisper_model_download_inner, download_recommended_ffmpeg_inner,
        download_recommended_whisper_model_inner, download_recommended_whisper_runtime_inner,
        download_recommended_ytdlp_inner, download_vad_model_inner,
        download_whisper_runtime_version_inner, toggle_whisper_model_download_pause_inner,
    },
    desktop_shell::{
        hide_main_window as hide_main_window_inner, show_main_window as show_main_window_inner,
    },
    recording_library::{
        convert_recordings_to_mp3_inner, delete_recording_inner, delete_recordings_inner,
        import_media_inner, import_youtube_inner, play_recording_inner, read_recording_texts_inner,
        transcribe_recordings_inner, translate_recordings_inner,
    },
    recording_session::{start_recording_inner, stop_recording_inner},
    runtime_assets::{
        check_whisper_model_update_inner, check_whisper_runtime_update_inner,
        check_ytdlp_update_inner,
    },
    settings::save_settings_inner,
};

#[tauri::command]
pub(crate) fn get_app_bootstrap(app: AppHandle) -> Result<AppBootstrap, String> {
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn download_recommended_whisper_model(app: AppHandle) -> Result<AppBootstrap, String> {
    download_recommended_whisper_model_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn download_recommended_whisper_runtime(app: AppHandle) -> Result<AppBootstrap, String> {
    download_recommended_whisper_runtime_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn download_whisper_runtime_version(
    app: AppHandle,
    runtime_version: String,
) -> Result<AppBootstrap, String> {
    download_whisper_runtime_version_inner(&app, &runtime_version)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn download_vad_model(app: AppHandle) -> Result<AppBootstrap, String> {
    download_vad_model_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn download_recommended_ffmpeg(app: AppHandle) -> Result<AppBootstrap, String> {
    download_recommended_ffmpeg_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn download_recommended_ytdlp(app: AppHandle) -> Result<AppBootstrap, String> {
    download_recommended_ytdlp_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) async fn check_whisper_runtime_update(
    app: AppHandle,
) -> Result<WhisperAssetUpdateResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        check_whisper_runtime_update_inner(&app_for_blocking)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn check_whisper_model_update(
    app: AppHandle,
) -> Result<WhisperAssetUpdateResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        check_whisper_model_update_inner(&app_for_blocking)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn check_ytdlp_update(app: AppHandle) -> Result<WhisperAssetUpdateResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || check_ytdlp_update_inner(&app_for_blocking))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn toggle_whisper_model_download_pause(app: AppHandle) -> Result<AppBootstrap, String> {
    toggle_whisper_model_download_pause_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn cancel_whisper_model_download(app: AppHandle) -> Result<AppBootstrap, String> {
    cancel_whisper_model_download_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) async fn save_settings(
    app: AppHandle,
    settings: AppSettings,
) -> Result<AppBootstrap, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        save_settings_inner(&app_for_blocking, settings)?;
        build_app_bootstrap(&app_for_blocking)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn start_recording(
    app: AppHandle,
    requested_name: Option<String>,
) -> Result<AppBootstrap, String> {
    start_recording_inner(&app, requested_name)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn stop_recording(app: AppHandle) -> Result<AppBootstrap, String> {
    stop_recording_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn show_main_window(app: AppHandle) -> Result<(), String> {
    show_main_window_inner(&app).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn hide_main_window(app: AppHandle) -> Result<(), String> {
    hide_main_window_inner(&app).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn load_anki_catalog(
    app: AppHandle,
    note_type: Option<String>,
) -> Result<AnkiCatalog, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        load_anki_catalog_inner(&app_for_blocking, note_type)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn create_anki_note_type() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(create_recommended_note_type_inner)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn play_recording(app: AppHandle, file_path: String) -> Result<(), String> {
    play_recording_inner(&app, &file_path)
}

#[tauri::command]
pub(crate) async fn read_recording_texts(
    app: AppHandle,
    file_path: String,
) -> Result<RecordingTexts, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        read_recording_texts_inner(&app_for_blocking, &file_path)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn delete_recording(app: AppHandle, file_path: String) -> Result<AppBootstrap, String> {
    delete_recording_inner(&app, &file_path)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn delete_recordings(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    delete_recordings_inner(&app, file_paths)
}

#[tauri::command]
pub(crate) async fn push_recordings_to_anki(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        push_recordings_to_anki_inner(&app_for_blocking, file_paths)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn push_recordings_to_anki_deck(
    app: AppHandle,
    file_paths: Vec<String>,
    deck_name: String,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        push_recordings_to_anki_deck_inner(&app_for_blocking, file_paths, deck_name)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn mine_segment_to_anki(
    app: AppHandle,
    file_path: String,
    text: String,
    start_ms: u64,
    end_ms: u64,
    translation: Option<String>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        mine_segment_to_anki_inner(
            &app_for_blocking,
            file_path,
            text,
            start_ms,
            end_ms,
            translation,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn translate_recordings(
    app: AppHandle,
    file_paths: Vec<String>,
    force: Option<bool>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    let force = force.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        translate_recordings_inner(&app_for_blocking, file_paths, force)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn add_furigana_to_anki(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        add_furigana_to_anki_inner(&app_for_blocking, file_paths)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn transcribe_recordings(
    app: AppHandle,
    file_paths: Vec<String>,
    force: Option<bool>,
    high_accuracy: Option<bool>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    let force = force.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        transcribe_recordings_inner(&app_for_blocking, file_paths, force, high_accuracy)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn import_media(
    app: AppHandle,
    paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || import_media_inner(&app_for_blocking, paths))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn import_youtube(
    app: AppHandle,
    url: String,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || import_youtube_inner(&app_for_blocking, url))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn convert_recordings_to_mp3(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        convert_recordings_to_mp3_inner(&app_for_blocking, file_paths)
    })
    .await
    .map_err(|error| error.to_string())?
}
