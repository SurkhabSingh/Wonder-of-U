use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::{
    app_config::RECOMMENDED_WHISPER_RUNTIME_VERSION,
    recording_library::import::probe_duration_ms,
    watch::library::{
        capture_thumbnail, normalize_origin, now_ms, remove_watched_video, resume_point_ms,
        upsert_watched_video, ORIGIN_GENERATED, ORIGIN_SYNCED,
    },
    watch::transcribe::{generate_watch_subtitles_inner, GeneratedSubtitles},
    app_types::SharedPersistedState,
    progress::measured::Measured,
    progress::report::{load_progress_inner, ProgressReport},
    runtime_assets::{detect_local_ffmpeg, detect_local_mpv},
    anki::{lookup_term_inner, mine_watched_line_inner, LookupResult},
    jimaku::{
        download_file, entry_files, sanitize_subtitle_file_name, search_entries, JimakuEntry,
        JimakuFile,
    },
    scanner_overlay::{set_scanner_overlay_enabled, set_scanner_popup_open},
    watch::{
        seek_watch_session as seek_watch_session_inner,
        add_watch_subtitle_file_if_playing,
        set_watch_subtitle_delay as set_watch_subtitle_delay_inner,
        start_watch_session as start_watch_session_inner,
        stop_watch_session as stop_watch_session_inner,
        subtitles::{load_subtitle_source, SubtitleSource},
        sync::sync_subtitles_with_alass,
        watch_snapshot as watch_snapshot_inner, WatchSnapshot,
    },
};
use crate::{
    anki::{
        add_furigana_to_anki_inner, create_recommended_note_type_inner, load_anki_catalog_inner,
        lookup_dictionaries_inner, LookupDictionaries,
        load_mined_sentences_inner, mine_segment_to_anki_inner, mine_segments_to_anki_inner,
        push_recordings_to_anki_deck_inner,
        push_recordings_to_anki_inner, rank_transcript_lines_inner, refresh_known_words_inner,
        scan_vocabulary_sources_inner,
    },
    app_runtime::{build_app_bootstrap, emit_app_snapshot, log_event},
    app_types::{
        AnkiCatalog, AppBootstrap, AppSettings, KnownWordsSnapshot, MinedSentences,
        MineLineRequest, MinedLinesResult, RecordingBatchResult, RecordingTexts,
        TranscriptRanking, VocabularySuggestions, WhisperAssetUpdateResult,
    },
    asset_downloads::{
        cancel_whisper_model_download_inner, enqueue_download,
        toggle_whisper_model_download_pause_inner, QueuedDownload,
    },
    desktop_shell::{
        hide_main_window as hide_main_window_inner, show_main_window as show_main_window_inner,
    },
    recording_library::{
        convert_recordings_to_mp3_inner, delete_recording_inner, delete_recordings_inner,
        download_missing_essentials_inner, import_media_inner, import_youtube_inner,
        play_recording_inner, read_recording_texts_inner, transcribe_recordings_inner,
        translate_recordings_inner,
    },
    recording_session::{start_recording_inner, stop_recording_inner},
    runtime_assets::{
        check_whisper_model_update_inner, check_whisper_runtime_update_inner,
        check_ytdlp_update_inner,
    },
    settings::save_settings_inner,
};

/// Writes a line the interface produced.
#[tauri::command]
pub(crate) fn log_from_ui(
    app: AppHandle,
    level: String,
    event: String,
    message: String,
    details: serde_json::Value,
) {
    let level = match level.as_str() {
        "ERROR" => "ERROR",
        "WARN" => "WARN",
        _ => "INFO",
    };
    let mut details = details;
    if let Some(fields) = details.as_object_mut() {
        fields.insert("message".into(), serde_json::json!(message));
    }
    log_event(&app, level, &format!("ui.{}", event.trim_start_matches("ui.")), details);
}

/// Shows the log file in the file manager, with it selected.
#[tauri::command]
pub(crate) fn open_log_folder(app: AppHandle) -> Result<(), String> {
    let path = app.state::<crate::app_types::AppPathsState>().inner().log_file.clone();
    let folder = path
        .parent()
        .ok_or_else(|| "The log file has no folder.".to_string())?
        .to_path_buf();
    // Create it first: on a fresh install nothing has been logged yet, and opening a folder
    // that does not exist reports a confusing failure.
    let _ = std::fs::create_dir_all(&folder);

    #[cfg(windows)]
    {
        let argument = if path.exists() {
            format!("/select,{}", path.display())
        } else {
            folder.display().to_string()
        };
        std::process::Command::new("explorer.exe")
            .arg(argument)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(&folder)
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

/// The text block for a bug report.
#[tauri::command]
pub(crate) fn copy_diagnostics(app: AppHandle) -> String {
    crate::app_runtime::diagnostics_text(&app)
}

#[tauri::command]
pub(crate) fn get_app_bootstrap(app: AppHandle) -> Result<AppBootstrap, String> {
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn download_recommended_whisper_model(app: AppHandle) -> Result<AppBootstrap, String> {
    enqueue_download(&app, QueuedDownload::WhisperModel)?;
    build_app_bootstrap(&app)
}

/// Fetches only the speech detector, for the repair offered when it is missing but the model
/// is not. Kept apart from the model download so that button cannot start a multi-gigabyte
/// transfer — see `download_whisper_vad_model_inner` for why that was possible.
#[tauri::command]
pub(crate) fn download_whisper_vad_model(app: AppHandle) -> Result<AppBootstrap, String> {
    enqueue_download(&app, QueuedDownload::WhisperVadModel)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn download_recommended_whisper_runtime(app: AppHandle) -> Result<AppBootstrap, String> {
    download_whisper_runtime_version(app, RECOMMENDED_WHISPER_RUNTIME_VERSION.to_string())
}

#[tauri::command]
pub(crate) fn download_whisper_runtime_version(
    app: AppHandle,
    runtime_version: String,
) -> Result<AppBootstrap, String> {
    enqueue_download(
        &app,
        QueuedDownload::WhisperRuntime {
            version: runtime_version,
        },
    )?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn download_recommended_ffmpeg(app: AppHandle) -> Result<AppBootstrap, String> {
    enqueue_download(&app, QueuedDownload::Ffmpeg { reinstall: false })?;
    build_app_bootstrap(&app)
}

/// Fetches a fresh FFmpeg over a working one.
#[tauri::command]
pub(crate) fn reinstall_ffmpeg(app: AppHandle) -> Result<AppBootstrap, String> {
    enqueue_download(&app, QueuedDownload::Ffmpeg { reinstall: true })?;
    build_app_bootstrap(&app)
}

/// Fetches the player Watch & Mine drives.
#[tauri::command]
pub(crate) fn download_recommended_mpv(app: AppHandle) -> Result<AppBootstrap, String> {
    enqueue_download(&app, QueuedDownload::Mpv { reinstall: false })?;
    build_app_bootstrap(&app)
}

/// Fetches a fresh mpv over a working one, for the button offered once a managed copy exists.
#[tauri::command]
pub(crate) fn reinstall_mpv(app: AppHandle) -> Result<AppBootstrap, String> {
    enqueue_download(&app, QueuedDownload::Mpv { reinstall: true })?;
    build_app_bootstrap(&app)
}

#[tauri::command]
pub(crate) fn download_recommended_ytdlp(app: AppHandle) -> Result<AppBootstrap, String> {
    enqueue_download(&app, QueuedDownload::Ytdlp)?;
    build_app_bootstrap(&app)
}

/// Fetches everything a fresh install is still missing, from one press.
#[tauri::command]
pub(crate) fn download_missing_essentials(app: AppHandle) -> Result<AppBootstrap, String> {
    download_missing_essentials_inner(&app)?;
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

/// Returns nothing, deliberately.
///
/// Pausing already emits an app snapshot, so handing the caller a second copy of the same
/// state gives that state two writers with no ordering between them. The download thread emits
/// on every chunk, so a snapshot built here — a few microseconds later — could be built from a
/// "downloading" the worker wrote in the gap, and the frontend applied it *after* the
/// worker's "paused" arrived. The button then read "Pause Download" over an already-paused
/// download, pressing it resumed instead of pausing, and the transfer looked stuck.
///
/// Pause is where this shows because pause is where the events stop: the worker emits "paused"
/// once and then blocks on the condvar, so nothing follows to correct a bad overwrite. Every
/// other status keeps emitting and heals itself, which is why only this one stuck.
///
/// `refresh_known_words` reached the same conclusion for the same reason — see the comment on
/// its caller. Returning `()` is what makes it structural rather than a convention: there is
/// no stale snapshot for a call site to apply, now or later.
#[tauri::command]
pub(crate) fn toggle_whisper_model_download_pause(app: AppHandle) -> Result<(), String> {
    toggle_whisper_model_download_pause_inner(&app)
}

/// Returns nothing, for the same reason as `toggle_whisper_model_download_pause`.
#[tauri::command]
pub(crate) fn cancel_whisper_model_download(app: AppHandle) -> Result<(), String> {
    cancel_whisper_model_download_inner(&app)
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

/// Lists the dictionaries the Anki add-on can look words up in.
#[tauri::command]
pub(crate) async fn lookup_dictionaries() -> Result<LookupDictionaries, String> {
    tauri::async_runtime::spawn_blocking(lookup_dictionaries_inner)
        .await
        .map_err(|error| error.to_string())?
}

/// Mines several lines from one recording in one pass.
#[tauri::command]
pub(crate) async fn mine_segments_to_anki(
    app: AppHandle,
    file_path: String,
    lines: Vec<MineLineRequest>,
) -> Result<MinedLinesResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        mine_segments_to_anki_inner(&app, file_path, lines)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Counts the words in each line that are not yet known.
#[tauri::command]
pub(crate) async fn rank_transcript_lines(
    app: AppHandle,
    lines: Vec<String>,
) -> Result<TranscriptRanking, String> {
    tauri::async_runtime::spawn_blocking(move || rank_transcript_lines_inner(&app, &lines))
        .await
        .map_err(|error| error.to_string())?
}

/// Looks through the collection for note types that hold vocabulary.
#[tauri::command]
pub(crate) async fn scan_vocabulary_sources(
    app: AppHandle,
) -> Result<VocabularySuggestions, String> {
    tauri::async_runtime::spawn_blocking(move || scan_vocabulary_sources_inner(&app))
        .await
        .map_err(|error| error.to_string())?
}

/// Rebuilds the known-word list from Anki. Manual by design — see
/// `refresh_known_words_inner`.
#[tauri::command]
pub(crate) async fn refresh_known_words(app: AppHandle) -> Result<KnownWordsSnapshot, String> {
    let app_for_blocking = app.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        let snapshot = refresh_known_words_inner(&app_for_blocking);
        if snapshot.as_ref().is_ok_and(|ready| ready.status == "ready") {
            crate::anki::record_comprehension_sample(&app_for_blocking);
        }
        snapshot
    })
    .await
    .map_err(|error| error.to_string())??;
    emit_app_snapshot(&app);
    Ok(snapshot)
}

#[tauri::command]
pub(crate) async fn load_progress(app: AppHandle) -> Result<ProgressReport, String> {
    tauri::async_runtime::spawn_blocking(move || load_progress_inner(&app))
        .await
        .map_err(|error| error.to_string())?
}

/// One reading of the audio element. Video is not reportable from the frontend: mpv is
/// sampled in Rust, and a second writer for that surface would double-count the same
/// minutes. `Instant::now()` is taken before the hop to the pool, where samples can swap.
#[tauri::command]
pub(crate) async fn record_listening_sample(
    app: AppHandle,
    playing: bool,
    position_ms: u64,
    rate: f64,
) {
    let arrived = std::time::Instant::now();
    let app_for_blocking = app.clone();
    if let Err(error) = tauri::async_runtime::spawn_blocking(move || {
        crate::progress::listen_sampler::record(&app_for_blocking, arrived, playing, position_ms, rate);
    })
    .await
    {
        // Silence here would read as zero immersion forever, beside a healthy status.
        crate::app_runtime::log_event(
            &app,
            "WARN",
            "progress.sample_rejected",
            serde_json::json!({ "message": error.to_string() }),
        );
    }
}

/// How many cards in the open collection came from this app.
#[tauri::command]
pub(crate) async fn count_mined_cards() -> Result<Measured<usize>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::anki::count_mined_cards_inner(crate::app_runtime::now_ms())
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn load_mined_sentences(app: AppHandle) -> Result<MinedSentences, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || load_mined_sentences_inner(&app_for_blocking))
        .await
        .map_err(|error| error.to_string())?
}

/// Starts mpv on a video, optionally with a subtitle file, replacing any session already
/// running. mpv is resolved fresh each time so a user who installs it mid-session does
/// not have to restart the app.
#[tauri::command]
pub(crate) async fn start_watch_session(
    app: AppHandle,
    video_path: String,
    subtitle_path: Option<String>,
) -> Result<WatchSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (settings, resume_position_ms) = {
            let persisted_state = app.state::<SharedPersistedState>();
            let persisted = persisted_state
                .0
                .lock()
                .map_err(|_| "Could not read the app settings.".to_string())?;
            let resume_position_ms = persisted
                .watched_videos
                .iter()
                .find(|video| video.video_path == video_path)
                .and_then(|video| video.resume_position_ms);
            (persisted.settings.clone(), resume_position_ms)
        };
        let detection = detect_local_mpv(&settings);
        let executable_path = detection.executable_path.clone().ok_or_else(|| {
            "mpv is required to watch a video. Download it in Settings, under Storage."
                .to_string()
        })?;
        start_watch_session_inner(
            Path::new(&executable_path),
            Path::new(&video_path),
            subtitle_path.as_deref().map(Path::new),
            resume_position_ms,
        )?;
        crate::progress::watch_sampler::spawn_watch_sampler(&app);

        // Re-apply the overlay setting to the player that just started.
        set_scanner_overlay_enabled(&app, settings.scanner.overlay_enabled)?;

        watch_snapshot_inner()
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Everything the watch page and the mine action need, in one read. A closed player
/// comes back as a disconnected snapshot rather than an error.
#[tauri::command]
pub(crate) async fn watch_snapshot() -> Result<WatchSnapshot, String> {
    tauri::async_runtime::spawn_blocking(watch_snapshot_inner)
        .await
        .map_err(|error| error.to_string())?
}

/// Mines the line mpv currently has on screen.
#[tauri::command]
pub(crate) async fn mine_watched_line(app: AppHandle) -> Result<RecordingBatchResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = watch_snapshot_inner()?;
        if !snapshot.connected {
            return Err("No video is playing.".into());
        }
        let Some(video_path) = snapshot.path else {
            return Err("mpv did not report which file it is playing.".into());
        };
        let Some(text) = snapshot
            .subtitle_text
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
        else {
            return Err("There is no subtitle on screen to mine.".into());
        };
        let (Some(start_ms), Some(end_ms)) = (snapshot.subtitle_start_ms, snapshot.subtitle_end_ms)
        else {
            return Err("mpv did not report this line's timing.".into());
        };
        mine_watched_line_inner(&app, video_path, text, start_ms, end_ms, None, None)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// The whole cue list for the video being watched, plus which subtitle tracks it has.
#[tauri::command]
pub(crate) async fn load_watch_subtitles(
    app: AppHandle,
    video_path: String,
    subtitle_path: Option<String>,
    track_index: Option<u32>,
) -> Result<SubtitleSource, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = {
            let persisted_state = app.state::<SharedPersistedState>();
            let persisted = persisted_state
                .0
                .lock()
                .map_err(|_| "Could not read the app settings.".to_string())?;
            persisted.settings.clone()
        };
        load_subtitle_source(
            &settings,
            Path::new(&video_path),
            subtitle_path.as_deref().map(Path::new),
            track_index,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Mines a specific line from the subtitle list, with optional per-mine padding.
#[tauri::command]
pub(crate) async fn mine_watch_line_at(
    app: AppHandle,
    video_path: String,
    text: String,
    start_ms: u64,
    end_ms: u64,
    pad_before_ms: Option<u64>,
    pad_after_ms: Option<u64>,
) -> Result<RecordingBatchResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        mine_watched_line_inner(
            &app,
            video_path,
            text,
            start_ms,
            end_ms,
            pad_before_ms,
            pad_after_ms,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Looks a word up in the Anki add-on's dictionary.
#[tauri::command]
pub(crate) async fn lookup_term(
    text: String,
    offset: usize,
    limit: Option<u32>,
) -> Result<LookupResult, String> {
    tauri::async_runtime::spawn_blocking(move || lookup_term_inner(text, offset, limit))
        .await
        .map_err(|error| error.to_string())?
}

/// Shifts the subtitles against the audio, in milliseconds.
#[tauri::command]
pub(crate) async fn set_watch_subtitle_delay(delay_ms: i64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || set_watch_subtitle_delay_inner(delay_ms))
        .await
        .map_err(|error| error.to_string())?
}

/// Where the synced file landed, and what alass reported doing to get there.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubtitleSyncResult {
    path: String,
    summary: String,
}

/// Realigns a subtitle file against the video's audio with alass, returning the new path.
#[tauri::command]
pub(crate) async fn add_watched_video(
    app: AppHandle,
    video_path: String,
) -> Result<AppBootstrap, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(&video_path);
        if !path.exists() {
            return Err(format!("The video is no longer at {}", path.display()));
        }

        let (settings, already_has_thumbnail) = {
            let persisted_state = app.state::<SharedPersistedState>();
            let persisted = persisted_state
                .0
                .lock()
                .map_err(|_| "Could not read the app settings.".to_string())?;
            let has_thumbnail = persisted
                .watched_videos
                .iter()
                .find(|video| video.video_path == video_path)
                .is_some_and(|video| video.thumbnail_path.is_some());
            (persisted.settings.clone(), has_thumbnail)
        };

        let ffmpeg = detect_local_ffmpeg(&settings).executable_path;
        let duration_ms = probe_duration_ms(ffmpeg.as_deref(), &path);
        let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        let added_at_ms = now_ms();
        let thumbnail = if already_has_thumbnail {
            None
        } else {
            ffmpeg.as_ref().and_then(|ffmpeg| {
                capture_thumbnail(
                    &app,
                    Path::new(ffmpeg),
                    &path,
                    Path::new(&settings.asset_directory),
                    duration_ms,
                    added_at_ms,
                )
            })
        };

        upsert_watched_video(&app, &video_path, |video| {
            video.duration_ms = duration_ms;
            video.bytes = bytes;
            if video.thumbnail_path.is_none() {
                video.thumbnail_path = thumbnail.map(|path| path.display().to_string());
            }
        })?;
        build_app_bootstrap(&app)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Note that a video was just opened, adding it to the library if it was not already there.
#[tauri::command]
pub(crate) async fn mark_watched_video_opened(
    app: AppHandle,
    video_path: String,
) -> Result<AppBootstrap, String> {
    tauri::async_runtime::spawn_blocking(move || {
        upsert_watched_video(&app, &video_path, |video| {
            video.last_opened_at_ms = Some(now_ms());
        })?;
        build_app_bootstrap(&app)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Remember where the user is in a video, so the next open picks it up there.
#[tauri::command]
pub(crate) async fn set_watched_video_position(
    app: AppHandle,
    video_path: String,
    position_ms: u64,
    duration_ms: u64,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let resume = resume_point_ms(position_ms, duration_ms);
        upsert_watched_video(&app, &video_path, |video| {
            video.resume_position_ms = resume;
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Which remembered videos are no longer on disk.
#[tauri::command]
pub(crate) async fn missing_watched_videos(app: AppHandle) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let videos = {
            let persisted_state = app.state::<SharedPersistedState>();
            let persisted = persisted_state
                .0
                .lock()
                .map_err(|_| "Could not read the video library.".to_string())?;
            persisted.watched_videos.clone()
        };

        Ok(videos
            .into_iter()
            .filter(|video| !Path::new(&video.video_path).exists())
            .map(|video| video.video_path)
            .collect())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Pair a subtitle with a video, or clear the pairing when `subtitle_path` is `None`.
#[tauri::command]
pub(crate) async fn set_watched_video_subtitle(
    app: AppHandle,
    video_path: String,
    subtitle_path: Option<String>,
    origin: Option<String>,
) -> Result<AppBootstrap, String> {
    tauri::async_runtime::spawn_blocking(move || {
        upsert_watched_video(&app, &video_path, |video| {
            video.subtitle_path = subtitle_path;
            video.subtitle_origin = normalize_origin(origin);
        })?;
        build_app_bootstrap(&app)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Forget a video. Deletes the entry and the thumbnail this app made — never the video itself.
#[tauri::command]
pub(crate) async fn forget_watched_video(
    app: AppHandle,
    video_path: String,
) -> Result<AppBootstrap, String> {
    tauri::async_runtime::spawn_blocking(move || {
        remove_watched_video(&app, &video_path)?;
        build_app_bootstrap(&app)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Transcribe the playing/selected video's own audio into a subtitle file beside it.
#[tauri::command]
pub(crate) async fn generate_watch_subtitles(
    app: AppHandle,
    video_path: String,
) -> Result<GeneratedSubtitles, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let generated = generate_watch_subtitles_inner(&app, Path::new(&video_path))?;
        upsert_watched_video(&app, &video_path, |video| {
            video.subtitle_path = Some(generated.path.clone());
            video.subtitle_origin = Some(ORIGIN_GENERATED.to_string());
        })?;
        Ok(generated)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn sync_watch_subtitles(
    app: AppHandle,
    video_path: String,
    subtitle_path: String,
) -> Result<SubtitleSyncResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = {
            let persisted_state = app.state::<SharedPersistedState>();
            let persisted = persisted_state
                .0
                .lock()
                .map_err(|_| "Could not read the app settings.".to_string())?;
            persisted.settings.clone()
        };
        let outcome =
            sync_subtitles_with_alass(&settings, Path::new(&video_path), Path::new(&subtitle_path))?;
        let synced = outcome.output_path.display().to_string();

        upsert_watched_video(&app, &video_path, |video| {
            video.subtitle_path = Some(synced.clone());
            video.subtitle_origin = Some(ORIGIN_SYNCED.to_string());
        })?;
        add_watch_subtitle_file_if_playing(&synced);
        Ok(SubtitleSyncResult {
            path: synced,
            summary: outcome.summary,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Downloads alass into the managed asset directory.
#[tauri::command]
pub(crate) fn download_recommended_alass(app: AppHandle) -> Result<AppBootstrap, String> {
    enqueue_download(&app, QueuedDownload::Alass)?;
    build_app_bootstrap(&app)
}

/// Downloads the Japanese dictionary into the managed asset directory.
#[tauri::command]
pub(crate) fn download_recommended_dictionary(app: AppHandle) -> Result<AppBootstrap, String> {
    enqueue_download(&app, QueuedDownload::Dictionary)?;
    build_app_bootstrap(&app)
}

fn jimaku_api_key(app: &AppHandle) -> Result<String, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the app settings.".to_string())?;
    Ok(persisted.settings.jimaku_api_key.clone())
}

/// Searches Jimaku for a title.
#[tauri::command]
pub(crate) async fn jimaku_search(
    app: AppHandle,
    query: String,
) -> Result<Vec<JimakuEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let key = jimaku_api_key(&app)?;
        search_entries(&key, &query)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Lists every subtitle file for an entry.
#[tauri::command]
pub(crate) async fn jimaku_files(
    app: AppHandle,
    entry_id: i64,
) -> Result<Vec<JimakuFile>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let key = jimaku_api_key(&app)?;
        entry_files(&key, entry_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Downloads a Jimaku subtitle file next to the video, and returns where it landed.
#[tauri::command]
pub(crate) async fn jimaku_download(
    app: AppHandle,
    url: String,
    file_name: String,
    video_path: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let key = jimaku_api_key(&app)?;
        let content = download_file(&key, &url)?;

        let directory = video_path
            .as_deref()
            .map(Path::new)
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .map_or_else(
                || {
                    let persisted_state = app.state::<SharedPersistedState>();
                    let persisted = persisted_state
                        .0
                        .lock()
                        .map_err(|_| "Could not read the app settings.".to_string())?;
                    Ok::<_, String>(std::path::PathBuf::from(&persisted.settings.output_directory))
                },
                Ok,
            )?;

        let target = directory.join(sanitize_subtitle_file_name(&file_name));
        std::fs::write(&target, content)
            .map_err(|error| format!("The subtitle file could not be saved: {error}"))?;
        Ok(target.display().to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Turns the scannable subtitle overlay over mpv on or off.
#[tauri::command]
pub(crate) async fn set_scanner_overlay(app: AppHandle, enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || set_scanner_overlay_enabled(&app, enabled))
        .await
        .map_err(|error| error.to_string())?
}

/// The overlay reporting whether a dictionary popup is on screen.
#[tauri::command]
pub(crate) fn set_scanner_popup(app: AppHandle, open: bool) {
    set_scanner_popup_open(&app, open);
}

/// Jumps the player to a position — what clicking a line in the subtitle list does.
#[tauri::command]
pub(crate) async fn seek_watch_session(position_ms: u64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || seek_watch_session_inner(position_ms))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn stop_watch_session() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(stop_watch_session_inner)
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

/// Cuts the sentence the viewer is about to play, and answers with the clip's path.
#[tauri::command]
pub(crate) async fn preview_segment_clip(
    app: AppHandle,
    file_path: String,
    start_ms: u64,
    end_ms: u64,
) -> Result<String, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::segment_preview::preview_segment_clip_inner(
            &app_for_blocking,
            file_path,
            start_ms,
            end_ms,
        )
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
    target_word: Option<String>,
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
            target_word,
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
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    let force = force.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        transcribe_recordings_inner(&app_for_blocking, file_paths, force)
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
