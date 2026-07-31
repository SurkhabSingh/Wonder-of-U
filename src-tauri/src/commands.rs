use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::{
    recording_library::import::probe_duration_ms,
    watch::library::{
        capture_thumbnail, normalize_origin, now_ms, remove_watched_video, upsert_watched_video,
        ORIGIN_GENERATED, ORIGIN_SYNCED,
    },
    watch::transcribe::{generate_watch_subtitles_inner, GeneratedSubtitles},
    app_types::SharedPersistedState,
    runtime_assets::{detect_local_ffmpeg, detect_local_mpv},
    anki::{
        lookup_term_inner, mine_watched_line_inner, preview_segment_clip_inner, LookupResult,
    },
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
        load_mined_sentences_inner, mine_segment_to_anki_inner, push_recordings_to_anki_deck_inner,
        push_recordings_to_anki_inner,
    },
    app_runtime::build_app_bootstrap,
    app_types::{
        AnkiCatalog, AppBootstrap, AppSettings, MinedSentences, RecordingBatchResult,
        RecordingTexts, WhisperAssetUpdateResult,
    },
    asset_downloads::{
        cancel_whisper_model_download_inner, download_recommended_ffmpeg_inner,
        download_recommended_whisper_model_inner, download_recommended_whisper_runtime_inner,
        download_recommended_alass_inner, download_recommended_ytdlp_inner,
        download_whisper_runtime_version_inner,
        toggle_whisper_model_download_pause_inner,
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
        let settings = {
            let persisted_state = app.state::<SharedPersistedState>();
            let persisted = persisted_state
                .0
                .lock()
                .map_err(|_| "Could not read the app settings.".to_string())?;
            persisted.settings.clone()
        };
        let detection = detect_local_mpv(&settings);
        let executable_path = detection.executable_path.clone().ok_or_else(|| {
            "mpv is required to watch a video; install it in Setup.".to_string()
        })?;
        start_watch_session_inner(
            Path::new(&executable_path),
            Path::new(&video_path),
            subtitle_path.as_deref().map(Path::new),
        )?;

        // Re-apply the overlay setting to the player that just started.
        //
        // "mpv's subtitles are off exactly when ours are on" is an invariant of the SESSION,
        // not of the toggle that last changed it: mpv is a fresh process with its own
        // defaults every time, and the setting outlives it. Leaving this to the frontend's
        // toggle handler meant a user who had the overlay switched on from a previous
        // session saw mpv's own subtitles until they toggled it off and on again.
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
///
/// Reads the player fresh rather than trusting anything the UI passed: the user presses
/// the hotkey because of what they are hearing right now, and a stale line would make a
/// card for the wrong sentence.
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
        // Both bounds come from mpv. Without them there is nothing to cut, and guessing a
        // window around the current position would produce a clip that does not match the
        // line on the card.
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
///
/// A sidecar the user picked wins; otherwise the requested embedded track. The frontend
/// parses the returned text, because the parser is shared with the rest of the UI.
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
///
/// Deliberately separate from `mine_watched_line`, which takes no arguments and re-reads
/// mpv so the hotkey always captures what you are hearing. This one mines the row you
/// picked — including one you scrolled back to, or one you merged — so it must be told
/// the bounds rather than discovering them.
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
///
/// Takes the sentence and a character offset rather than a word, because the backend
/// deinflects prefix candidates and picks the longest match — segmenting first would be
/// a second, worse segmenter.
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
///
/// The cheap fix for the common fault — a file off by a constant. Nothing is written to
/// disk and it is instantly reversible, which is why it is offered before alass.
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
///
/// For the harder fault, where the drift varies across the episode and no single offset
/// works. Writes beside the original rather than over it.
/// Cut the clip the viewer plays for one sentence — the same slice a mine makes.
#[tauri::command]
pub(crate) async fn preview_segment_clip(
    app: AppHandle,
    file_path: String,
    start_ms: u64,
    end_ms: u64,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        preview_segment_clip_inner(&app, file_path, start_ms, end_ms)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Remember a video, with a thumbnail and its duration, before it has ever been played.
///
/// Adding is deliberately separate from playing: a video you have queued up but not started is
/// still one you want the app to keep, along with whatever subtitle you pair with it.
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

        let settings = {
            let persisted_state = app.state::<SharedPersistedState>();
            let persisted = persisted_state
                .0
                .lock()
                .map_err(|_| "Could not read the app settings.".to_string())?;
            persisted.settings.clone()
        };

        // Both are best-effort. ffmpeg missing is a perfectly ordinary state for a new install,
        // and it must cost a thumbnail and a duration, not the ability to add a video.
        let ffmpeg = detect_local_ffmpeg(&settings).executable_path;
        let duration_ms = probe_duration_ms(ffmpeg.as_deref(), &path);
        let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        let added_at_ms = now_ms();
        let thumbnail = ffmpeg.as_ref().and_then(|ffmpeg| {
            capture_thumbnail(
                Path::new(ffmpeg),
                &path,
                Path::new(&settings.asset_directory),
                duration_ms,
                added_at_ms,
            )
        });

        upsert_watched_video(&app, &video_path, |video| {
            video.duration_ms = duration_ms;
            video.bytes = bytes;
            // Re-adding a video that is already listed refreshes its facts but keeps the
            // subtitle it is paired with — losing that to a second Add would be the one
            // outcome this feature exists to prevent.
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
///
/// Opening is how a video most often enters the list — you find a file, watch it, and expect it
/// to be there next time without having thought about "adding" anything.
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

/// Which remembered videos are no longer on disk.
///
/// On demand rather than a field on the entry: this stats every video, and the snapshot that
/// carries the library is emitted on every download progress tick. A per-emit filesystem walk
/// is exactly the kind of hot-path work that had to be reverted once already.
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
///
/// Blocking work on the blocking pool, like every other whisper pass. The result is handed
/// back rather than loaded here: the caller sets it as the session's sidecar, which is what
/// makes it eligible for the alass Sync button exactly like a downloaded subtitle.
#[tauri::command]
pub(crate) async fn generate_watch_subtitles(
    app: AppHandle,
    video_path: String,
) -> Result<GeneratedSubtitles, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let generated = generate_watch_subtitles_inner(&app, Path::new(&video_path))?;
        // Recorded here rather than left to the caller: the backend knows for certain which
        // file it just wrote and for which video, and a mapping that depends on the frontend
        // remembering to report it is a mapping that will eventually be wrong.
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
        // Same reasoning as the generated case: alass has just rewritten which file this video
        // should be watched with, and that is the mapping.
        upsert_watched_video(&app, &video_path, |video| {
            video.subtitle_path = Some(synced.clone());
            video.subtitle_origin = Some(ORIGIN_SYNCED.to_string());
        })?;
        // Hand the corrected file to the player when one is running: reporting success while
        // mpv keeps showing the old subtitles would have the user trusting a fix they are not
        // watching. When nothing is playing there is nothing to mislead — realigning from the
        // library is exactly that case — and the file is on disk with the mapping pointing at
        // it, so the next open uses it. This previously failed the whole call, which also
        // meant closing mpv mid-align reported a failure for a sync that had worked.
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
    download_recommended_alass_inner(&app)?;
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
///
/// Saved beside the video rather than into a temp directory because it is the user's file
/// now: they will re-open it, and alass will write its corrected copy alongside.
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
///
/// Also flips mpv's own subtitle rendering the other way — two layers at once would draw
/// every line twice.
#[tauri::command]
pub(crate) async fn set_scanner_overlay(app: AppHandle, enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || set_scanner_overlay_enabled(&app, enabled))
        .await
        .map_err(|error| error.to_string())?
}

/// The overlay reporting whether a dictionary popup is on screen.
///
/// Needed because click-through is a property of the whole window: without this, releasing
/// the scan modifier would make the popup unclickable the instant it appeared.
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
