use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, Runtime};

use super::{
    client::{anki_connect_request, anki_offline_message},
    fields::{
        anki_media_file_name, html_escape, prepend_anki_field_value, user_friendly_anki_error,
    },
    furigana::{
        insert_furigana_field, recording_transcript_supports_furigana, request_furigana_html,
    },
    screenshot::capture_screenshot,
};
use crate::{
    app_runtime::{build_app_bootstrap, update_shell_snapshot},
    app_state::transcript_looks_japanese,
    app_types::{
        AppSettings, RecentRecording, RecordingActionItem, RecordingBatchResult,
        SharedPersistedState,
    },
    recording_library::{find_recent_recording, playback_path, unique_path_with_suffix},
    runtime_assets::detect_local_ffmpeg,
};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub(crate) fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Extra audio kept on each side of a mined line's clip.
///
/// Asymmetric on purpose: a line's start is usually the tighter edge (speech begins
/// almost immediately) while its end often clips a trailing syllable, so one symmetric
/// value cannot serve both. `clip_padding_ms` in settings is the default for both sides;
/// the subtitle list can override per mine.
#[derive(Debug, Clone, Copy)]
pub(super) struct ClipPadding {
    pub(super) before_ms: u64,
    pub(super) after_ms: u64,
}

impl ClipPadding {
    fn symmetric(padding_ms: u64) -> Self {
        Self {
            before_ms: padding_ms,
            after_ms: padding_ms,
        }
    }
}

/// Formats a millisecond offset as the `S.mmm` seconds string ffmpeg expects for
/// `-ss`/`-to` (e.g. `1500` -> `"1.500"`, `250` -> `"0.250"`).
fn format_ffmpeg_timestamp(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// Builds the ffmpeg argument list for slicing `[start_ms, end_ms]` (with padding)
/// out of `input` into `output`. Kept pure so the ordering and timestamp
/// formatting can be unit-tested without spawning ffmpeg. `-ss`/`-to` come before
/// `-i` so ffmpeg seeks by keyframe before decoding.
fn slice_ffmpeg_args(
    start_ms: u64,
    end_ms: u64,
    padding: ClipPadding,
    input: &str,
    output: &str,
) -> Vec<String> {
    let start = start_ms.saturating_sub(padding.before_ms);
    let end = end_ms.saturating_add(padding.after_ms);
    vec![
        "-y".into(),
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        format_ffmpeg_timestamp(start),
        "-to".into(),
        format_ffmpeg_timestamp(end),
        "-i".into(),
        input.to_string(),
        "-map".into(),
        "0:a:0".into(),
        "-vn".into(),
        "-codec:a".into(),
        "libmp3lame".into(),
        "-b:a".into(),
        "128k".into(),
        output.to_string(),
    ]
}

/// Slices the requested sentence out of `audio_path` into a fresh MP3 beside it.
/// FFmpeg is mandatory here: unlike the optional WAV->MP3 compression, a mine has
/// nothing to attach without the clip, so a missing binary is a hard error.
fn slice_segment_clip(
    settings: &AppSettings,
    audio_path: &Path,
    start_ms: u64,
    end_ms: u64,
    padding: ClipPadding,
) -> Result<PathBuf, String> {
    let detection = detect_local_ffmpeg(settings);
    let executable_path = detection
        .executable_path
        .clone()
        .ok_or_else(|| "FFmpeg is required to mine audio; install it in Setup.".to_string())?;

    let parent = audio_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = audio_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    let clip_path = unique_path_with_suffix(parent, &format!("{stem}_seg{start_ms}"), ".mp3");

    let mut command = Command::new(&executable_path);
    hide_command_window(&mut command);
    if let Some(ffmpeg_directory) = Path::new(&executable_path).parent() {
        command.current_dir(ffmpeg_directory);
    }
    command.args(slice_ffmpeg_args(
        start_ms,
        end_ms,
        padding,
        &audio_path.display().to_string(),
        &clip_path.display().to_string(),
    ));

    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "FFmpeg is required to mine audio; install it in Setup.".to_string()
        } else {
            format!("FFmpeg could not slice the audio clip: {error}")
        }
    })?;

    let clip_ready = output.status.success()
        && fs::metadata(&clip_path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);

    if !clip_ready {
        let _ = fs::remove_file(&clip_path);
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "FFmpeg did not produce an audio clip for this sentence.".to_string()
        } else {
            format!("FFmpeg could not slice the audio clip: {stderr}")
        });
    }

    Ok(clip_path)
}

/// Everything a mine needs about where the sentence came from, with no notion of
/// whether that was a library recording or a video being watched.
///
/// This split is deliberate. Mining is identical either way — cut the audio, grab a
/// still, build the fields, add the note — and the only difference is where those facts
/// come from. Keeping one implementation behind this struct is what stops the watch
/// session and the transcript viewer drifting into two subtly different miners.
pub(super) struct MineSource {
    /// The file the audio is cut from. A library recording's audio, or the video itself
    /// when watching — ffmpeg reads the first audio stream either way, so a container
    /// with video in it needs no special handling.
    pub(super) media_path: PathBuf,
    /// The file a still frame is grabbed from, when there is one to grab.
    pub(super) video_path: Option<PathBuf>,
    /// Optional card metadata. Each is written only when its field is mapped AND the
    /// value exists, so a source with less provenance simply produces a smaller card.
    pub(super) source_path: Option<String>,
    pub(super) created_at_ms: Option<u64>,
    pub(super) source_url: Option<String>,
    pub(super) display_title: String,
    /// Whether furigana should be attempted. Resolved by the caller because the answer
    /// comes from the recording's declared language, which a watch session does not have.
    pub(super) supports_furigana: bool,
}

/// Grabs a still from the source's video at the middle of the line, or explains why not.
///
/// `Ok(None)` — not an error — covers every ordinary reason there is no picture: no image
/// field mapped, or a source with no video at all (a mic recording, a YouTube import, an
/// audio file). `Err` is reserved for "there should have been a frame and there wasn't",
/// which the caller reports alongside a card it still creates.
///
/// The midpoint rather than the start: a line's first frame is often the tail of the
/// previous shot or a hard cut, while the middle reliably shows whoever is speaking.
fn capture_line_screenshot(
    settings: &AppSettings,
    source: &MineSource,
    start_ms: u64,
    end_ms: u64,
) -> Result<Option<PathBuf>, String> {
    if settings.anki.fields.image.is_empty() {
        return Ok(None);
    }
    let Some(video_path) = source.video_path.as_ref() else {
        return Ok(None);
    };
    if !video_path.exists() {
        return Err(format!("the video is no longer at {}", video_path.display()));
    }

    let detection = detect_local_ffmpeg(settings);
    let executable_path = detection
        .executable_path
        .clone()
        .ok_or_else(|| "FFmpeg is required to take a screenshot".to_string())?;

    let parent = source
        .media_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let stem = source
        .media_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    let shot_path = unique_path_with_suffix(parent, &format!("{stem}_shot{start_ms}"), ".jpg");

    let midpoint_ms = start_ms + (end_ms.saturating_sub(start_ms)) / 2;
    capture_screenshot(
        Path::new(&executable_path),
        video_path,
        midpoint_ms,
        &shot_path,
    )
    .map_err(|error| {
        let _ = fs::remove_file(&shot_path);
        error
    })?;
    Ok(Some(shot_path))
}

/// Runs the whole mine for one sentence and returns the single action item plus
/// the batch status string that item maps to.
fn mine_single_segment<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
    text: &str,
    start_ms: u64,
    end_ms: u64,
    translation: Option<&str>,
) -> (RecordingActionItem, &'static str) {
    let failed = |message: String| {
        (
            RecordingActionItem {
                file_path: file_path.to_string(),
                status: "failed".into(),
                message,
                note_id: None,
            },
            "partial",
        )
    };

    // Resolve the recording and its audio on disk, then hand the rest to the shared
    // miner. This function is now ONLY the library-recording half.
    let recording = match find_recent_recording(app, file_path) {
        Ok(recording) => recording,
        Err(error) => return failed(error),
    };
    let audio_path = match playback_path(&recording) {
        Ok(path) => path,
        Err(error) => return failed(error),
    };

    let source = MineSource {
        media_path: audio_path,
        // A library recording's video, if any, is not tracked — the watch session is
        // where a picture comes from.
        video_path: None,
        source_path: Some(recording.file_path.clone()),
        created_at_ms: Some(recording.created_at_ms),
        source_url: recording.source_url.clone(),
        display_title: recording_display_title(&recording),
        supports_furigana: recording_transcript_supports_furigana(&recording, text.trim()),
    };

    mine_media_to_anki(app, file_path, &source, text, start_ms, end_ms, translation, None)
}

/// Mines one sentence from any media source. Shared by the transcript viewer and the
/// watch session; see `MineSource`.
pub(super) fn mine_media_to_anki<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
    source: &MineSource,
    text: &str,
    start_ms: u64,
    end_ms: u64,
    translation: Option<&str>,
    padding_override: Option<ClipPadding>,
) -> (RecordingActionItem, &'static str) {
    let failed = |message: String| {
        (
            RecordingActionItem {
                file_path: file_path.to_string(),
                status: "failed".into(),
                message,
                note_id: None,
            },
            "partial",
        )
    };

    // 1. Settings + field-mapping validation.
    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = match persisted_state.0.lock() {
            Ok(persisted) => persisted,
            Err(_) => return failed("Could not read the Anki settings.".into()),
        };
        persisted.settings.clone()
    };
    let anki = settings.anki.clone();
    if anki.deck_name.is_empty() {
        return failed("Choose an Anki deck before mining sentences.".into());
    }
    if anki.note_type.is_empty() {
        return failed("Choose an Anki note type before mining sentences.".into());
    }
    if anki.fields.transcription.is_empty() {
        return failed("Map an Anki field for the transcript before mining sentences.".into());
    }

    let trimmed_text = text.trim();
    if trimmed_text.is_empty() {
        return failed("There is no sentence text to mine.".into());
    }

    // 3. Slice the sentence clip (ffmpeg is mandatory). The per-mine override wins; the
    // global setting is the default for both sides.
    let padding = padding_override
        .unwrap_or_else(|| ClipPadding::symmetric(settings.anki.clip_padding_ms));
    let clip_path = match slice_segment_clip(&settings, &source.media_path, start_ms, end_ms, padding)
    {
        Ok(path) => path,
        Err(error) => return failed(error),
    };

    // 3 (cont.). A still from the video, when there is one. Deliberately not fatal: a
    // card with audio and no picture is worth far more than no card, so a failure here
    // rides along as a note on an otherwise successful mine.
    let (screenshot_path, mut screenshot_problem) =
        match capture_line_screenshot(&settings, source, start_ms, end_ms) {
            Ok(path) => (path, None),
            Err(problem) => (None, Some(problem)),
        };

    // 4. Anki must be reachable before we store media or add the note.
    if let Err(error) = anki_connect_request("version", serde_json::json!({})) {
        let _ = fs::remove_file(&clip_path);
        return (
            RecordingActionItem {
                file_path: file_path.to_string(),
                status: "failed".into(),
                message: anki_offline_message(&error),
                note_id: None,
            },
            "unavailable",
        );
    }

    let clip_media_file_name = anki_media_file_name(&clip_path);
    let store_result = anki_connect_request(
        "storeMediaFile",
        serde_json::json!({
            "filename": clip_media_file_name,
            "path": clip_path.display().to_string()
        }),
    );
    // 8. Anki copies the clip into its own media folder, so the temp file is done
    // regardless of whether storing (or the later addNote) succeeds.
    let _ = fs::remove_file(&clip_path);
    if let Err(error) = store_result {
        if let Some(path) = &screenshot_path {
            let _ = fs::remove_file(path);
        }
        return failed(format!("Anki could not store the audio clip. {error}"));
    }

    // The still goes into Anki's media folder the same way. A store failure here demotes
    // the card to audio-only rather than losing it.
    let screenshot_media_file_name = match &screenshot_path {
        Some(path) => {
            let media_file_name = anki_media_file_name(path);
            let stored = anki_connect_request(
                "storeMediaFile",
                serde_json::json!({
                    "filename": media_file_name,
                    "path": path.display().to_string()
                }),
            );
            let _ = fs::remove_file(path);
            match stored {
                Ok(_) => Some(media_file_name),
                Err(error) => {
                    screenshot_problem =
                        Some(format!("Anki could not store the screenshot. {error}"));
                    None
                }
            }
        }
        None => None,
    };

    // 4 (cont.). Build the note fields from the mapping.
    let mut fields = serde_json::Map::new();
    fields.insert(
        anki.fields.transcription.clone(),
        serde_json::Value::String(html_escape(trimmed_text)),
    );
    prepend_anki_field_value(
        &mut fields,
        &anki.fields.audio,
        format!("[sound:{clip_media_file_name}]"),
    );
    if let Some(media_file_name) = &screenshot_media_file_name {
        // The name comes from a path this app built, but it is interpolated into an HTML
        // attribute, so it is escaped like every other field value.
        fields.insert(
            anki.fields.image.clone(),
            serde_json::Value::String(format!(
                "<img src=\"{}\">",
                html_escape(media_file_name)
            )),
        );
    }
    if !anki.fields.source_path.is_empty() {
        if let Some(source_path) = source.source_path.as_deref() {
            fields.insert(
                anki.fields.source_path.clone(),
                serde_json::Value::String(html_escape(source_path)),
            );
        }
    }
    if !anki.fields.created_at.is_empty() {
        if let Some(created_at_ms) = source.created_at_ms {
            fields.insert(
                anki.fields.created_at.clone(),
                serde_json::Value::String(created_at_ms.to_string()),
            );
        }
    }

    // 4 (cont.). Source link / title / timestamp — each written only when the field
    // is mapped AND the data exists (a local recording with no URL omits the link).
    let display_title = source.display_title.clone();
    if !anki.fields.source_url.is_empty() {
        if let Some(url) = source
            .source_url
            .as_deref()
            .map(str::trim)
            // Only ever build a link for http(s) URLs — never let a stray
            // javascript:/data: URL become a clickable link in the rendered card.
            .filter(|value| {
                let lower = value.to_ascii_lowercase();
                lower.starts_with("https://") || lower.starts_with("http://")
            })
        {
            let href = youtube_timestamped_link(url, start_ms).unwrap_or_else(|| url.to_string());
            let link_text = if display_title.is_empty() {
                "Source".to_string()
            } else {
                display_title.clone()
            };
            fields.insert(
                anki.fields.source_url.clone(),
                serde_json::Value::String(format!(
                    "<a href=\"{}\">{}</a>",
                    html_escape(&href),
                    html_escape(&link_text),
                )),
            );
        }
    }
    if !anki.fields.title.is_empty() && !display_title.is_empty() {
        fields.insert(
            anki.fields.title.clone(),
            serde_json::Value::String(html_escape(&display_title)),
        );
    }
    if !anki.fields.position.is_empty() {
        fields.insert(
            anki.fields.position.clone(),
            serde_json::Value::String(format_position(start_ms)),
        );
    }

    // 5. Translation: reuse the recording's existing translation for this
    // sentence when one is present (the paired line the viewer already shows).
    // Mining never generates a fresh translation — if none exists, the card
    // carries the text alone, mirroring the whole-recording push.
    if !anki.fields.translation.is_empty() {
        if let Some(translation) = translation.map(str::trim).filter(|value| !value.is_empty()) {
            fields.insert(
                anki.fields.translation.clone(),
                serde_json::Value::String(html_escape(translation)),
            );
        }
    }

    // 6. Furigana (non-fatal).
    if source.supports_furigana {
        if let Ok(furigana_html) = request_furigana_html(trimmed_text) {
            insert_furigana_field(&anki, &furigana_html, &clip_media_file_name, &mut fields);
        }
    }

    // 7. Create the note with the same dedup guard the push flow uses.
    let note_result = anki_connect_request(
        "addNote",
        serde_json::json!({
            "note": {
                "deckName": anki.deck_name.clone(),
                "modelName": anki.note_type.clone(),
                "fields": fields,
                "options": {
                    "allowDuplicate": false,
                    "duplicateScope": "deck",
                    "duplicateScopeOptions": {
                        "deckName": anki.deck_name.clone(),
                        "checkChildren": false,
                        "checkAllModels": false
                    }
                },
                "tags": ["wonder-of-u"]
            }
        }),
    );

    let note_id = match note_result {
        Ok(value) => match value.as_i64() {
            Some(note_id) => note_id,
            None => return failed("AnkiConnect did not return a note id.".into()),
        },
        Err(error) => {
            if error.to_lowercase().contains("duplicate") {
                return (
                    RecordingActionItem {
                        file_path: file_path.to_string(),
                        status: "skipped".into(),
                        message: "This sentence is already mined.".into(),
                        note_id: None,
                    },
                    "completed",
                );
            }
            return failed(user_friendly_anki_error(&error, &anki));
        }
    };

    (
        RecordingActionItem {
            file_path: file_path.to_string(),
            status: "success".into(),
            // The card exists either way, but a screenshot the user expected and did not
            // get has to be said out loud — silently dropping it would report a partial
            // result as a whole one.
            message: match screenshot_problem {
                Some(problem) => format!(
                    "Mined sentence into Anki note {note_id}, without a screenshot: {problem}."
                ),
                None => format!("Mined sentence into Anki note {note_id}."),
            },
            note_id: Some(note_id),
        },
        "completed",
    )
}

/// Mines the line currently on screen in a watch session.
///
/// Everything comes from mpv: the video path, the line, and its exact bounds. There is no
/// import, no transcription, and no library entry — which is the point. The card carries
/// the video's file name as its title and the line's timestamp; there is no source URL
/// because a local file has none.
pub(crate) fn mine_watched_line_inner<R: Runtime>(
    app: &AppHandle<R>,
    video_path: String,
    text: String,
    start_ms: u64,
    end_ms: u64,
    pad_before_ms: Option<u64>,
    pad_after_ms: Option<u64>,
) -> Result<RecordingBatchResult, String> {
    // Either side may be overridden on its own; an unset side falls back to the global
    // setting, resolved inside the miner.
    let padding = match (pad_before_ms, pad_after_ms) {
        (None, None) => None,
        (before, after) => Some(ClipPadding {
            before_ms: before.unwrap_or(0),
            after_ms: after.unwrap_or(0),
        }),
    };
    let video = PathBuf::from(&video_path);
    let display_title = video
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_string();
    let source = MineSource {
        // ffmpeg cuts the audio straight out of the video container.
        media_path: video.clone(),
        video_path: Some(video),
        source_path: Some(video_path.clone()),
        created_at_ms: None,
        source_url: None,
        display_title,
        // A watch session has no declared language, so this falls back to reading the
        // line itself — the same test the library path uses for an untagged transcript.
        supports_furigana: transcript_looks_japanese(&text),
    };

    let (item, batch_status) =
        mine_media_to_anki(app, &video_path, &source, &text, start_ms, end_ms, None, padding);
    let message = item.message.clone();

    update_shell_snapshot(app, |shell| {
        shell.status_text = message.clone();
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: batch_status.into(),
        message,
        items: vec![item],
        bootstrap: build_app_bootstrap(app)?,
    })
}

pub(crate) fn mine_segment_to_anki_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_path: String,
    text: String,
    start_ms: u64,
    end_ms: u64,
    translation: Option<String>,
) -> Result<RecordingBatchResult, String> {
    let (item, batch_status) = mine_single_segment(
        app,
        &file_path,
        &text,
        start_ms,
        end_ms,
        translation.as_deref(),
    );
    let message = item.message.clone();

    update_shell_snapshot(app, |shell| {
        shell.status_text = message.clone();
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: batch_status.into(),
        message,
        items: vec![item],
        bootstrap: build_app_bootstrap(app)?,
    })
}

/// For a YouTube URL, returns the same URL with a `t=<seconds>s` parameter so the
/// link opens at the sentence's moment. Returns None for non-YouTube URLs (the
/// caller then links the URL plainly). Chooses `?`/`&` based on any existing query.
fn youtube_timestamped_link(url: &str, start_ms: u64) -> Option<String> {
    let is_youtube = url.contains("youtube.com/watch")
        || url.contains("youtu.be/")
        || url.contains("youtube.com/shorts/");
    if !is_youtube {
        return None;
    }
    let seconds = start_ms / 1000;
    let separator = if url.contains('?') { '&' } else { '?' };
    Some(format!("{url}{separator}t={seconds}s"))
}

/// Formats a segment start time as `H:MM:SS`, or `M:SS` under an hour.
fn format_position(start_ms: u64) -> String {
    let total_secs = start_ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// The recording's display title: its stored title (an imported file's original
/// name) when set, else the file stem of its path.
fn recording_display_title(recording: &RecentRecording) -> String {
    if let Some(title) = recording
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return title.to_string();
    }
    std::path::Path::new(&recording.file_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        format_ffmpeg_timestamp, format_position, slice_ffmpeg_args, ClipPadding,
        youtube_timestamped_link,
    };

    #[test]
    fn formats_millisecond_offsets_as_padded_seconds() {
        assert_eq!(format_ffmpeg_timestamp(0), "0.000");
        assert_eq!(format_ffmpeg_timestamp(250), "0.250");
        assert_eq!(format_ffmpeg_timestamp(1500), "1.500");
        assert_eq!(format_ffmpeg_timestamp(60123), "60.123");
    }

    #[test]
    fn slice_args_pad_the_window_and_order_seek_before_input() {
        let args = slice_ffmpeg_args(1000, 2000, ClipPadding::symmetric(250), "in.wav", "out.mp3");

        let ss = args.iter().position(|arg| arg == "-ss").expect("-ss present");
        let to = args.iter().position(|arg| arg == "-to").expect("-to present");
        let input = args.iter().position(|arg| arg == "-i").expect("-i present");

        // Seek flags must precede the input for keyframe-accurate seeking.
        assert!(ss < input);
        assert!(to < input);

        // 250ms of padding on each side, clamped by saturating math.
        assert_eq!(args[ss + 1], "0.750");
        assert_eq!(args[to + 1], "2.250");

        assert_eq!(args.last().map(String::as_str), Some("out.mp3"));
        assert!(args.iter().any(|arg| arg == "libmp3lame"));
        assert!(args.iter().any(|arg| arg == "128k"));
    }

    #[test]
    fn slice_args_clamp_padding_at_the_start_of_the_file() {
        let args = slice_ffmpeg_args(100, 500, ClipPadding::symmetric(250), "in.wav", "out.mp3");
        let ss = args.iter().position(|arg| arg == "-ss").expect("-ss present");
        // 100ms - 250ms padding saturates to the start of the file.
        assert_eq!(args[ss + 1], "0.000");
    }

    #[test]
    fn slice_args_pad_each_side_independently() {
        // A line's start is usually the tighter edge while its end clips a trailing
        // syllable, so the two sides must be settable apart from each other.
        let args = slice_ffmpeg_args(
            5_000,
            6_000,
            ClipPadding { before_ms: 100, after_ms: 900 },
            "in.mkv",
            "out.mp3",
        );
        let ss = args.iter().position(|arg| arg == "-ss").expect("-ss present");
        let to = args.iter().position(|arg| arg == "-to").expect("-to present");
        assert_eq!(args[ss + 1], "4.900");
        assert_eq!(args[to + 1], "6.900");
    }

    #[test]
    fn slice_args_accept_no_padding_at_all() {
        let args = slice_ffmpeg_args(
            5_000,
            6_000,
            ClipPadding { before_ms: 0, after_ms: 0 },
            "in.mkv",
            "out.mp3",
        );
        let ss = args.iter().position(|arg| arg == "-ss").expect("-ss present");
        let to = args.iter().position(|arg| arg == "-to").expect("-to present");
        assert_eq!(args[ss + 1], "5.000");
        assert_eq!(args[to + 1], "6.000");
    }

    #[test]
    fn youtube_links_deep_link_to_the_moment() {
        assert_eq!(
            youtube_timestamped_link("https://www.youtube.com/watch?v=abc", 153_000).as_deref(),
            Some("https://www.youtube.com/watch?v=abc&t=153s"),
        );
        assert_eq!(
            youtube_timestamped_link("https://youtu.be/abc", 5_000).as_deref(),
            Some("https://youtu.be/abc?t=5s"),
        );
        assert_eq!(
            youtube_timestamped_link("https://youtube.com/shorts/xyz", 0).as_deref(),
            Some("https://youtube.com/shorts/xyz?t=0s"),
        );
        assert_eq!(youtube_timestamped_link("https://example.com/v", 5_000), None);
    }

    #[test]
    fn positions_format_as_a_clock() {
        assert_eq!(format_position(5_000), "0:05");
        assert_eq!(format_position(153_000), "2:33");
        assert_eq!(format_position(3_723_000), "1:02:03");
    }
}
