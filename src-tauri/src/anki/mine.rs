use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::{
    clip::capture_clip,
    client::{anki_connect_request, anki_offline_message},
    fields::{
        anki_media_file_name, html_escape, prepend_anki_field_value, user_friendly_anki_error,
    },
    furigana::{
        insert_furigana_field, recording_transcript_supports_furigana, request_furigana_html,
    },
    media_temp::{mining_temp_dir, TempMedia},
    screenshot::capture_screenshot,
};
use crate::{
    app_runtime::{build_app_bootstrap, log_event, update_shell_snapshot},
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

/// Emitted after a watch line is mined, whichever of the three ways started it.
pub(crate) const WATCH_LINE_MINED_EVENT: &str = "watch-line-mined";

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

/// Names one of a mine's scratch files inside the temp directory.
///
/// `unique_path_with_suffix` still does the work, so two mines of the same line at the same
/// moment cannot collide; only the directory has changed.
fn temp_media_path(stem_source: &Path, label: &str, start_ms: u64, extension: &str) -> Result<PathBuf, String> {
    let directory = mining_temp_dir()?;
    let stem = stem_source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    Ok(unique_path_with_suffix(
        &directory,
        &format!("{stem}_{label}{start_ms}"),
        extension,
    ))
}

/// Slices the requested sentence out of `audio_path` into a fresh MP3.
/// FFmpeg is mandatory here: unlike the optional WAV->MP3 compression, a mine has
/// nothing to attach without the clip, so a missing binary is a hard error.
fn slice_segment_clip(
    ffmpeg_path: &Path,
    audio_path: &Path,
    start_ms: u64,
    end_ms: u64,
    padding: ClipPadding,
) -> Result<TempMedia, String> {
    // Checked before ffmpeg runs so a moved or renamed source reads as what it is. mpv
    // keeps playing a file that has been renamed underneath it — it holds the handle — so
    // this is reachable while the video is still visibly on screen, and without this the
    // user gets an ffmpeg stderr dump for a file they can see playing.
    if !audio_path.exists() {
        return Err(format!(
            "The media is no longer at {}. It was moved or renamed after this session started.",
            audio_path.display()
        ));
    }

    let clip = TempMedia::new(temp_media_path(audio_path, "seg", start_ms, ".mp3")?);

    let mut command = Command::new(ffmpeg_path);
    hide_command_window(&mut command);
    if let Some(ffmpeg_directory) = ffmpeg_path.parent() {
        command.current_dir(ffmpeg_directory);
    }
    command.args(slice_ffmpeg_args(
        start_ms,
        end_ms,
        padding,
        &audio_path.display().to_string(),
        &clip.path().display().to_string(),
    ));

    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "FFmpeg is required to mine audio; install it in Setup.".to_string()
        } else {
            format!("FFmpeg could not slice the audio clip: {error}")
        }
    })?;

    let clip_ready = output.status.success()
        && fs::metadata(clip.path())
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);

    if !clip_ready {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "FFmpeg did not produce an audio clip for this sentence.".to_string()
        } else {
            format!("FFmpeg could not slice the audio clip: {stderr}")
        });
    }

    Ok(clip)
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
    ffmpeg_path: &Path,
    settings: &AppSettings,
    source: &MineSource,
    start_ms: u64,
    end_ms: u64,
) -> Result<Option<TempMedia>, String> {
    if settings.anki.fields.image.is_empty() {
        return Ok(None);
    }
    let Some(video_path) = source.video_path.as_ref() else {
        return Ok(None);
    };
    if !video_path.exists() {
        return Err(format!("the video is no longer at {}", video_path.display()));
    }

    let shot = TempMedia::new(temp_media_path(&source.media_path, "shot", start_ms, ".jpg")?);
    let midpoint_ms = start_ms + (end_ms.saturating_sub(start_ms)) / 2;
    // The guard drops on the `?`, so a failed grab cleans up after itself.
    capture_screenshot(ffmpeg_path, video_path, midpoint_ms, shot.path())?;
    Ok(Some(shot))
}

/// Cuts a short video of the line, or explains why not.
///
/// Same contract as the screenshot: `Ok(None)` for every ordinary reason there is no clip —
/// no video field mapped, or a source with no video at all — and `Err` only when there
/// should have been one. The card is still created either way; audio is the only part a
/// mine cannot do without.
///
/// Unlike the screenshot, this uses the SAME padded window as the audio. A still is a single
/// moment and the midpoint is the best guess at it, but a clip the user watches has to start
/// and end where the audio they hear does, or the two disagree on the same card.
fn capture_line_video(
    ffmpeg_path: &Path,
    settings: &AppSettings,
    source: &MineSource,
    start_ms: u64,
    end_ms: u64,
    padding: ClipPadding,
) -> Result<Option<TempMedia>, String> {
    if settings.anki.fields.video.is_empty() {
        return Ok(None);
    }
    let Some(video_path) = source.video_path.as_ref() else {
        return Ok(None);
    };
    if !video_path.exists() {
        return Err(format!("the video is no longer at {}", video_path.display()));
    }

    let clip = TempMedia::new(temp_media_path(&source.media_path, "clip", start_ms, ".webm")?);
    capture_clip(
        ffmpeg_path,
        video_path,
        start_ms.saturating_sub(padding.before_ms),
        end_ms.saturating_add(padding.after_ms),
        clip.path(),
    )?;
    Ok(Some(clip))
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

    // 3. Resolve ffmpeg once. Each call re-walks the managed install directory, and a mine
    // needs it up to three times now — audio, still, clip.
    let ffmpeg_path = match detect_local_ffmpeg(&settings).executable_path {
        Some(path) => PathBuf::from(path),
        None => return failed("FFmpeg is required to mine audio; install it in Setup.".into()),
    };

    // 3 (cont.). Slice the sentence clip (ffmpeg is mandatory). The per-mine override wins;
    // the global setting is the default for both sides.
    let padding = padding_override
        .unwrap_or_else(|| ClipPadding::symmetric(settings.anki.clip_padding_ms));
    let clip = match slice_segment_clip(&ffmpeg_path, &source.media_path, start_ms, end_ms, padding)
    {
        Ok(clip) => clip,
        Err(error) => return failed(error),
    };

    // 3 (cont.). A still and a video of the line, when there is a video to take them from.
    // Deliberately not fatal: a card with audio and no picture is worth far more than no
    // card, so a failure here rides along as a note on an otherwise successful mine.
    let (screenshot, mut screenshot_problem) =
        match capture_line_screenshot(&ffmpeg_path, &settings, source, start_ms, end_ms) {
            Ok(path) => (path, None),
            Err(problem) => (None, Some(problem)),
        };
    let (video_clip, mut video_problem) =
        match capture_line_video(&ffmpeg_path, &settings, source, start_ms, end_ms, padding) {
            Ok(path) => (path, None),
            Err(problem) => (None, Some(problem)),
        };

    // 4. Anki must be reachable before we store media or add the note. Every scratch file
    // is owned by a `TempMedia`, so returning from here cleans them up on its own.
    if let Err(error) = anki_connect_request("version", serde_json::json!({})) {
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

    // Anki copies each file into its own media folder, so the originals stay scratch.
    let clip_media_file_name = anki_media_file_name(clip.path());
    if let Err(error) = anki_connect_request(
        "storeMediaFile",
        serde_json::json!({
            "filename": clip_media_file_name,
            "path": clip.path().display().to_string()
        }),
    ) {
        return failed(format!("Anki could not store the audio clip. {error}"));
    }

    // The still and the clip go in the same way. A store failure here demotes the card to
    // audio-only rather than losing it.
    let screenshot_media_file_name = match &screenshot {
        Some(shot) => {
            let media_file_name = anki_media_file_name(shot.path());
            match anki_connect_request(
                "storeMediaFile",
                serde_json::json!({
                    "filename": media_file_name,
                    "path": shot.path().display().to_string()
                }),
            ) {
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

    let video_media_file_name = match &video_clip {
        Some(video) => {
            let media_file_name = anki_media_file_name(video.path());
            match anki_connect_request(
                "storeMediaFile",
                serde_json::json!({
                    "filename": media_file_name,
                    "path": video.path().display().to_string()
                }),
            ) {
                Ok(_) => Some(media_file_name),
                Err(error) => {
                    video_problem = Some(format!("Anki could not store the video clip. {error}"));
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
    if let Some(media_file_name) = &video_media_file_name {
        // A `<video>` element, NOT `[sound:...]`. Anki hands a `[sound:]` video to its
        // external player, which on the desktop means the clip opens in its own mpv window
        // on top of the card. An element plays inside the card itself, on every client:
        // Anki's own media check still finds the file through the `src`, so it is tracked
        // and never swept as unused.
        //
        // `playsinline` is what stops iOS taking the clip fullscreen, and no `autoplay`
        // because the back of the card already replays the audio — both at once is noise.
        fields.insert(
            anki.fields.video.clone(),
            // Sized on the element itself rather than left to the note type's stylesheet.
            // A card can be any note type, including one this app has never styled, and an
            // unconstrained 720p clip renders at its native size — wider than the card,
            // pushed off to one side, with the whole card scrolling sideways.
            serde_json::Value::String(format!(
                concat!(
                    "<video src=\"{}\" controls preload=\"metadata\" playsinline ",
                    "style=\"max-width:100%;height:auto;display:block;margin:0 auto\">",
                    "</video>"
                ),
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
            // The card exists either way, but media the user expected and did not get has
            // to be said out loud — silently dropping it would report a partial result as a
            // whole one.
            message: match (screenshot_problem, video_problem) {
                (None, None) => format!("Mined sentence into Anki note {note_id}."),
                (Some(problem), None) => format!(
                    "Mined sentence into Anki note {note_id}, without a screenshot: {problem}."
                ),
                (None, Some(problem)) => format!(
                    "Mined sentence into Anki note {note_id}, without a video clip: {problem}."
                ),
                (Some(shot), Some(video)) => format!(
                    "Mined sentence into Anki note {note_id}, without a screenshot ({shot})                      or a video clip ({video})."
                ),
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

    // Tell the watch page which line was mined, so its row shows the mark.
    //
    // Emitted HERE rather than at each caller because there are three ways to mine a watch
    // line — the subtitle row, the Mine button, and the global hotkey — and only the row
    // used to record it. The hotkey cannot record it from the frontend at all: it fires in
    // Rust while mpv has focus, and the app never hears about it. One event at the single
    // point they all pass through is what stops the three drifting again.
    if item.status == "success" || item.status == "skipped" {
        let emitted = app.emit(
            WATCH_LINE_MINED_EVENT,
            serde_json::json!({
                "startMs": start_ms,
                "endMs": end_ms,
                "text": text,
            }),
        );
        // Logged because this is the only signal the watch page gets, and the hotkey path
        // has no other way to be observed: it fires while mpv has focus, so a failure to
        // deliver looks exactly like nothing having happened.
        log_event(
            app,
            if emitted.is_ok() { "INFO" } else { "WARN" },
            "watch.line_mined",
            serde_json::json!({
                "startMs": start_ms,
                "endMs": end_ms,
                "status": item.status,
                "emitted": emitted.is_ok(),
            }),
        );
    }

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
    // Drop any `t=` the source URL already carries before adding ours. Imports store the
    // URL the user pasted, which is very often already timestamped
    // (`...?v=abc&t=298s`) — and YouTube honours the FIRST `t` it sees, so appending a
    // second one silently sent every card to wherever the import started instead of to
    // its own sentence.
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, query),
        None => (url, ""),
    };
    let kept = query
        .split('&')
        .filter(|parameter| {
            !parameter.is_empty()
                && parameter != &"t"
                && !parameter.starts_with("t=")
                // `start=` is YouTube's other seek parameter and would win the same way.
                && parameter != &"start"
                && !parameter.starts_with("start=")
        })
        .collect::<Vec<_>>();

    let mut rebuilt = String::from(base);
    if !kept.is_empty() {
        rebuilt.push('?');
        rebuilt.push_str(&kept.join("&"));
        rebuilt.push('&');
    } else {
        rebuilt.push('?');
    }
    rebuilt.push_str(&format!("t={seconds}s"));
    Some(rebuilt)
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
    fn youtube_links_replace_a_timestamp_the_url_already_had() {
        // Imports store the URL the user pasted, which is very often already
        // timestamped. YouTube honours the FIRST `t`, so appending a second one sent
        // every card to the import's start instead of to its own sentence.
        assert_eq!(
            youtube_timestamped_link("https://www.youtube.com/watch?v=abc&t=298s", 153_000)
                .as_deref(),
            Some("https://www.youtube.com/watch?v=abc&t=153s"),
        );
        // `start=` is YouTube's other seek parameter and would win the same way.
        assert_eq!(
            youtube_timestamped_link("https://www.youtube.com/watch?v=abc&start=60", 12_000)
                .as_deref(),
            Some("https://www.youtube.com/watch?v=abc&t=12s"),
        );
        // Every other parameter is preserved, and order is otherwise untouched.
        assert_eq!(
            youtube_timestamped_link(
                "https://www.youtube.com/watch?v=abc&list=PL1&t=30s&index=2",
                7_000
            )
            .as_deref(),
            Some("https://www.youtube.com/watch?v=abc&list=PL1&index=2&t=7s"),
        );
        // A short link whose only parameter was the timestamp must not keep a stray `&`.
        assert_eq!(
            youtube_timestamped_link("https://youtu.be/abc?t=99s", 4_000).as_deref(),
            Some("https://youtu.be/abc?t=4s"),
        );
    }

    #[test]
    fn positions_format_as_a_clock() {
        assert_eq!(format_position(5_000), "0:05");
        assert_eq!(format_position(153_000), "2:33");
        assert_eq!(format_position(3_723_000), "1:02:03");
    }
}
