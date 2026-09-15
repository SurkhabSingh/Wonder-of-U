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
    definitions::{definitions_for, definitions_for_word, Definitions},
    client::{anki_connect_health_check, anki_connect_request, anki_offline_message},
    fields::{
        anki_media_file_name, html_escape, prepend_anki_field_value, user_friendly_anki_error,
        MediaPart,
    },
    furigana::{
        insert_furigana_field, request_furigana_html,
    },
    media_temp::{mining_temp_dir, TempMedia},
    screenshot::capture_screenshot,
    tags,
};
use crate::{
    app_runtime::{build_app_bootstrap, log_event, update_shell_snapshot},
    app_state::transcript_looks_japanese,
    app_types::{
        AppSettings, MineLineRequest, MinedLineOutcome, MinedLinesResult, RecentRecording,
        RecordingActionItem, RecordingBatchResult, SharedPersistedState,
    },
    media_errors::stderr_indicates_no_audio,
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
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClipPadding {
    pub(super) before_ms: u64,
    pub(super) after_ms: u64,
}

impl ClipPadding {
    pub(crate) fn symmetric(padding_ms: u64) -> Self {
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

pub(crate) fn slice_ffmpeg_args(
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

/// Shown when the media a mine was asked for carries no audio track at all.
const NO_AUDIO_MESSAGE: &str = "This video has no sound, so there is nothing to mine.";

fn slice_failure_message(stderr: &str) -> String {
    if stderr_indicates_no_audio(stderr) {
        NO_AUDIO_MESSAGE.to_string()
    } else if stderr.is_empty() {
        "FFmpeg did not produce an audio clip for this sentence.".to_string()
    } else {
        format!("FFmpeg could not slice the audio clip: {stderr}")
    }
}

fn slice_segment_clip(
    ffmpeg_path: &Path,
    audio_path: &Path,
    start_ms: u64,
    end_ms: u64,
    padding: ClipPadding,
) -> Result<TempMedia, String> {
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
        return Err(slice_failure_message(&stderr));
    }

    Ok(clip)
}

/// Hands one scratch file to Anki and returns the name the card should reference it by.
fn store_media_with_anki(
    file: &TempMedia,
    source_path: &Path,
    label: &'static str,
    start_ms: u64,
) -> Result<String, String> {
    let extension = file
        .path()
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("wav");
    let media_file_name =
        anki_media_file_name(source_path, MediaPart::Line { label, start_ms }, extension);
    anki_connect_request(
        "storeMediaFile",
        serde_json::json!({
            "filename": media_file_name,
            "path": file.path().display().to_string()
        }),
    )?;
    Ok(media_file_name)
}

/// Everything a mine needs about where the sentence came from, with no notion of
/// whether that was a library recording or a video being watched.
pub(super) struct MineSource {
    pub(super) media_path: PathBuf,
    pub(super) video_path: Option<PathBuf>,
    pub(super) source_path: Option<String>,
    pub(super) created_at_ms: Option<u64>,
    pub(super) source_url: Option<String>,
    pub(super) display_title: String,
    pub(super) supports_furigana: bool,
}

/// Grabs a still from the source's video at the middle of the line, or explains why not.
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
    capture_clip(ffmpeg_path, video_path, start_ms, end_ms, padding, clip.path())?;
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
    target_word: Option<&str>,
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
        // Judged on the mined LINE, exactly as the watch path judges it, because that is
        // what the card will hold.
        //
        // The recording-level gate takes `transcript_language`, which is the language of
        // whichever pass ran LAST — not of the transcript this line came from. On a
        // recording transcribed in Japanese and later in English, mining a Japanese line
        // asked "is the recording English?", got yes, and skipped furigana on Japanese
        // text without a word. The push and the furigana updater avoid it by overriding
        // that field with the variant's own language first; this call did not, and a single
        // line does not need the indirection — the text is right here.
        supports_furigana: transcript_looks_japanese(text.trim()),
    };

    mine_media_to_anki(
        app,
        file_path,
        &source,
        &MinedLine {
            text,
            start_ms,
            end_ms,
            translation,
            target_word,
        },
        None,
    )
}

/// Mines one sentence from any media source. Shared by the transcript viewer and the
/// watch session; see `MineSource`.
pub(super) struct MinedLine<'a> {
    pub(super) text: &'a str,
    pub(super) start_ms: u64,
    pub(super) end_ms: u64,
    pub(super) translation: Option<&'a str>,
    pub(super) target_word: Option<&'a str>,
}

pub(super) fn mine_media_to_anki<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
    source: &MineSource,
    line: &MinedLine<'_>,
    padding_override: Option<ClipPadding>,
) -> (RecordingActionItem, &'static str) {
    let MinedLine {
        text,
        start_ms,
        end_ms,
        translation,
        target_word,
    } = *line;
    let mined_word = target_word.map(str::trim).filter(|word| !word.is_empty());
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

    let ffmpeg_path = match detect_local_ffmpeg(&settings).executable_path {
        Some(path) => PathBuf::from(path),
        None => return failed("FFmpeg is required to mine audio; install it in Setup.".into()),
    };

    let padding = padding_override
        .unwrap_or_else(|| ClipPadding::symmetric(settings.anki.clip_padding_ms));
    let clip = match slice_segment_clip(&ffmpeg_path, &source.media_path, start_ms, end_ms, padding)
    {
        Ok(clip) => clip,
        Err(error) => return failed(error),
    };

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

    let clip_media_file_name = match store_media_with_anki(&clip, &source.media_path, "seg", start_ms) {
        Ok(name) => name,
        Err(error) => return failed(format!("Anki could not store the audio clip. {error}")),
    };

    let screenshot_media_file_name = screenshot.as_ref().and_then(|shot| {
        store_media_with_anki(shot, &source.media_path, "shot", start_ms)
            .map_err(|error| {
                screenshot_problem = Some(format!("Anki could not store the screenshot. {error}"));
            })
            .ok()
    });
    let video_media_file_name = video_clip.as_ref().and_then(|video| {
        store_media_with_anki(video, &source.media_path, "clip", start_ms)
            .map_err(|error| {
                video_problem = Some(format!("Anki could not store the video clip. {error}"));
            })
            .ok()
    });

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
        fields.insert(
            anki.fields.image.clone(),
            serde_json::Value::String(format!(
                "<img src=\"{}\">",
                html_escape(media_file_name)
            )),
        );
    }
    if let Some(media_file_name) = &video_media_file_name {
        fields.insert(
            anki.fields.video.clone(),
            serde_json::Value::String(format!(
                concat!(
                    "<video src=\"{}\" autoplay muted loop controls playsinline ",
                    "preload=\"auto\" ",
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

    let display_title = source.display_title.clone();
    if !anki.fields.source_url.is_empty() {
        if let Some(url) = source
            .source_url
            .as_deref()
            .map(str::trim)
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

    if !anki.fields.translation.is_empty() {
        if let Some(translation) = translation.map(str::trim).filter(|value| !value.is_empty()) {
            fields.insert(
                anki.fields.translation.clone(),
                serde_json::Value::String(html_escape(translation)),
            );
        }
    }

    if let Some(word) = mined_word {
        if !anki.fields.word.is_empty() {
            fields.insert(
                anki.fields.word.clone(),
                serde_json::Value::String(html_escape(word)),
            );
        }
    }

    let mut definition_problem = None;
    if settings.features.add_definitions_to_mined_cards {
        if anki.fields.definition.is_empty() {
            definition_problem = Some("definitions (no Anki field is mapped)".to_string());
        } else if anki.definition_dictionary_ids.is_empty() {
            definition_problem =
                Some("definitions (no dictionaries are chosen for them)".to_string());
        } else {
            let found = match mined_word {
                Some(word) => definitions_for_word(app, word, &anki.definition_dictionary_ids),
                None => definitions_for(app, trimmed_text, &anki.definition_dictionary_ids),
            };
            match found {
                Definitions::Ready { html, missing } => {
                    fields.insert(
                        anki.fields.definition.clone(),
                        serde_json::Value::String(html),
                    );
                    if !missing.is_empty() {
                        definition_problem = Some(format!(
                            "a meaning for {} (not in your chosen dictionaries)",
                            missing.join("、")
                        ));
                    }
                }
                Definitions::Unavailable(reason) => {
                    definition_problem = Some(format!("definitions ({reason})"))
                }
                Definitions::NothingToAdd => {}
            }
        }
    }

    let mut furigana_problem = None;
    if settings.features.auto_add_furigana_after_anki_push && source.supports_furigana {
        match request_furigana_html(trimmed_text) {
            Ok(furigana_html) => {
                insert_furigana_field(&anki, &furigana_html, &clip_media_file_name, &mut fields);
            }
            Err(error) => furigana_problem = Some(error),
        }
    }

    let kind_tag = if mined_word.is_some() {
        tags::MINED_WORD
    } else {
        tags::MINED_LINE
    };
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
                "tags": [tags::MINED, kind_tag]
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
    crate::progress::record_mined_act(app);

    (
        RecordingActionItem {
            file_path: file_path.to_string(),
            status: "success".into(),
            message: {
                let missing: Vec<String> = [
                    screenshot_problem.map(|problem| format!("a screenshot ({problem})")),
                    video_problem.map(|problem| format!("a video clip ({problem})")),
                    furigana_problem.map(|problem| format!("furigana ({problem})")),
                    definition_problem,
                ]
                .into_iter()
                .flatten()
                .collect();

                if missing.is_empty() {
                    format!("Mined sentence into Anki note {note_id}.")
                } else {
                    format!(
                        "Mined sentence into Anki note {note_id}, without {}.",
                        missing.join(", "),
                    )
                }
            },
            note_id: Some(note_id),
        },
        "completed",
    )
}

/// Mines the line currently on screen in a watch session.
pub(crate) fn mine_watched_line_inner<R: Runtime>(
    app: &AppHandle<R>,
    video_path: String,
    text: String,
    start_ms: u64,
    end_ms: u64,
    pad_before_ms: Option<u64>,
    pad_after_ms: Option<u64>,
) -> Result<RecordingBatchResult, String> {
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
        media_path: video.clone(),
        video_path: Some(video),
        source_path: Some(video_path.clone()),
        created_at_ms: None,
        source_url: None,
        display_title,
        supports_furigana: transcript_looks_japanese(&text),
    };

    let (item, batch_status) =
        mine_media_to_anki(
            app,
            &video_path,
            &source,
            &MinedLine {
                text: &text,
                start_ms,
                end_ms,
                translation: None,
                target_word: None,
            },
            padding,
        );
    let message = item.message.clone();

    // Tell the watch page which line was mined, so its row shows the mark.
    if item.status == "success" || item.status == "skipped" {
        let emitted = app.emit(
            WATCH_LINE_MINED_EVENT,
            serde_json::json!({
                "startMs": start_ms,
                "endMs": end_ms,
                "text": text,
            }),
        );
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
    target_word: Option<String>,
) -> Result<RecordingBatchResult, String> {
    let (item, batch_status) = mine_single_segment(
        app,
        &file_path,
        &text,
        start_ms,
        end_ms,
        translation.as_deref(),
        target_word.as_deref(),
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

/// How many failures in a row end the run.
const CONSECUTIVE_FAILURE_LIMIT: usize = 3;

/// Roughly how many progress updates a run should emit, whatever its length.
const PROGRESS_UPDATE_COUNT: usize = 20;

fn mined_outcome(line: &MineLineRequest, status: &str, message: String) -> MinedLineOutcome {
    MinedLineOutcome {
        text: line.text.clone(),
        start_ms: line.start_ms,
        end_ms: line.end_ms,
        status: status.into(),
        message,
    }
}

/// Mines several lines from one recording in a single pass.
pub(crate) fn mine_segments_to_anki_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_path: String,
    lines: Vec<MineLineRequest>,
) -> Result<MinedLinesResult, String> {
    let total = lines.len();
    if let Err(error) = anki_connect_health_check() {
        return Ok(MinedLinesResult {
            status: "failed".into(),
            message: anki_offline_message(&error),
            added: 0,
            failed: 0,
            lines: lines
                .iter()
                .map(|line| mined_outcome(line, "notAttempted", String::new()))
                .collect(),
            bootstrap: build_app_bootstrap(app)?,
        });
    }

    let progress_every = (total / PROGRESS_UPDATE_COUNT).max(1);
    let mut outcomes: Vec<MinedLineOutcome> = Vec::with_capacity(total);
    let mut added = 0usize;
    let mut failed = 0usize;
    let mut consecutive_failures = 0usize;
    let mut stopped_early = false;

    for (index, line) in lines.iter().enumerate() {
        if stopped_early {
            outcomes.push(mined_outcome(line, "notAttempted", String::new()));
            continue;
        }

        let (item, _) = mine_single_segment(
            app,
            &file_path,
            &line.text,
            line.start_ms,
            line.end_ms,
            line.translation.as_deref(),
            None,
        );
        if item.status == "failed" {
            failed += 1;
            consecutive_failures += 1;
            log_event(
                app,
                "WARN",
                "mine.batch_line_failed",
                serde_json::json!({
                    "startMs": line.start_ms,
                    "text": line.text,
                    "message": item.message,
                }),
            );
            outcomes.push(mined_outcome(line, "failed", item.message));
            stopped_early = consecutive_failures >= CONSECUTIVE_FAILURE_LIMIT;
        } else {
            added += 1;
            consecutive_failures = 0;
            outcomes.push(mined_outcome(line, "added", item.message));
        }

        if index % progress_every == 0 {
            let _ = update_shell_snapshot(app, |shell| {
                shell.status_text = format!("Mining line {} of {total}…", index + 1);
                shell.transition_count += 1;
            });
        }
    }

    let status = if stopped_early {
        "stopped"
    } else if failed > 0 {
        "partial"
    } else {
        "ready"
    };
    let message = if stopped_early {
        format!(
            "Stopped after {CONSECUTIVE_FAILURE_LIMIT} lines failed in a row — Anki stopped answering. {added} added before that."
        )
    } else if failed > 0 {
        format!("{added} added, {failed} could not be mined.")
    } else {
        format!("{added} {} added to Anki.", if added == 1 { "card" } else { "cards" })
    };

    log_event(
        app,
        if failed > 0 { "WARN" } else { "INFO" },
        "mine.batch_finished",
        serde_json::json!({
            "requested": total,
            "added": added,
            "failed": failed,
            "stoppedEarly": stopped_early,
        }),
    );

    update_shell_snapshot(app, |shell| {
        shell.status_text = message.clone();
        shell.transition_count += 1;
    })?;

    Ok(MinedLinesResult {
        status: status.into(),
        message,
        added,
        failed,
        lines: outcomes,
        bootstrap: build_app_bootstrap(app)?,
    })
}

fn youtube_timestamped_link(url: &str, start_ms: u64) -> Option<String> {
    let is_youtube = url.contains("youtube.com/watch")
        || url.contains("youtu.be/")
        || url.contains("youtube.com/shorts/");
    if !is_youtube {
        return None;
    }
    let seconds = start_ms / 1000;
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
        format_ffmpeg_timestamp, format_position, slice_failure_message, slice_ffmpeg_args,
        ClipPadding, youtube_timestamped_link,
    };

    #[test]
    fn a_silent_video_is_named_rather_than_reported_as_an_ffmpeg_fault() {
        assert_eq!(
            slice_failure_message(
                "Stream map '' matches no streams.\nTo ignore this, add a trailing '?' to the map."
            ),
            "This video has no sound, so there is nothing to mine."
        );

        assert_eq!(
            slice_failure_message("Unknown encoder 'libmp3lame'"),
            "FFmpeg could not slice the audio clip: Unknown encoder 'libmp3lame'"
        );
        assert_eq!(
            slice_failure_message(""),
            "FFmpeg did not produce an audio clip for this sentence."
        );
    }

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
        assert_eq!(
            youtube_timestamped_link("https://www.youtube.com/watch?v=abc&t=298s", 153_000)
                .as_deref(),
            Some("https://www.youtube.com/watch?v=abc&t=153s"),
        );
        assert_eq!(
            youtube_timestamped_link("https://www.youtube.com/watch?v=abc&start=60", 12_000)
                .as_deref(),
            Some("https://www.youtube.com/watch?v=abc&t=12s"),
        );
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
